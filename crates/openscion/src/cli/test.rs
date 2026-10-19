//! Unit tests for command parsing and dispatch.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{self, Write};

use clap::Parser;
use openscion_core::{
    Change, CreateChangeset, EntryKind, FrameWriter, ObjectId, ObjectKind, PROTOCOL_VERSION,
    RepoPath, UpdateRef, object_id,
};

use super::{Cli, Command, DEFAULT_AUTHOR, DEFAULT_REF, DEFAULT_SERVER, run};
use crate::test_support::{real_server, scratch, unreachable_server};
use crate::{Error, ServerClient};

fn output(cli: &Cli) -> String {
    let mut out = Vec::new();
    run(cli, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

/// Runs a subcommand against `base` and returns its captured output.
fn run_at(base: &str, args: &[&str]) -> String {
    let mut full = vec!["opnscn", "--server", base];
    full.extend_from_slice(args);
    output(&Cli::try_parse_from(full).unwrap())
}

fn put(client: &ServerClient, bytes: &[u8]) -> ObjectId {
    let id = object_id(ObjectKind::Blob, bytes);
    let mut frames = FrameWriter::new();
    frames.push(id, bytes);
    client.upload_blobs(&frames.finish()).unwrap();
    id
}

fn upsert(path: &str, blob: ObjectId) -> Change {
    Change::Upsert {
        path: RepoPath::new(path).unwrap(),
        kind: EntryKind::File,
        blob,
    }
}

/// A server with `main` at a child changeset that rewrote `a.txt` and added
/// `b.txt` over a root. Returns the base URL and the root and child ids.
fn history() -> (String, ObjectId, ObjectId) {
    let base = real_server();
    let client = ServerClient::new(&base);
    let one = put(&client, b"one");
    let two = put(&client, b"two");
    let b = put(&client, b"b");
    let c1 = client
        .create_changeset(&CreateChangeset::new(
            vec![],
            vec![upsert("a.txt", one)],
            "t",
            "root",
        ))
        .unwrap();
    let c2 = client
        .create_changeset(&CreateChangeset::new(
            vec![c1.id],
            vec![upsert("a.txt", two), upsert("b.txt", b)],
            "t",
            "edit",
        ))
        .unwrap();
    client
        .update_ref("main", UpdateRef::new(None, c2.id))
        .unwrap();
    (base, c1.id, c2.id)
}

#[test]
fn the_server_defaults_to_a_local_openscion_server() {
    let cli = Cli::try_parse_from(["opnscn", "version"]).unwrap();
    assert_eq!(cli.server, DEFAULT_SERVER);
    assert_eq!(cli.command, Command::Version);
}

#[test]
fn server_is_accepted_after_the_subcommand() {
    let cli = Cli::try_parse_from(["opnscn", "server-info", "--server", "http://h:1"]).unwrap();
    assert_eq!(cli.server, "http://h:1");
    assert_eq!(cli.command, Command::ServerInfo);
}

#[test]
fn an_unknown_command_is_rejected() {
    assert!(Cli::try_parse_from(["opnscn", "frobnicate"]).is_err());
}

#[test]
fn version_prints_the_client_and_protocol_versions() {
    let cli = Cli::try_parse_from(["opnscn", "version"]).unwrap();
    let (major, minor) = PROTOCOL_VERSION;
    assert_eq!(
        output(&cli),
        format!(
            "opnscn {} (protocol {major}.{minor})\n",
            env!("CARGO_PKG_VERSION")
        )
    );
}

#[test]
fn server_info_prints_what_the_server_reports() {
    let base = real_server();
    let cli = Cli::try_parse_from(["opnscn", "--server", &base, "server-info"]).unwrap();
    let (major, minor) = PROTOCOL_VERSION;
    assert_eq!(
        output(&cli),
        format!(
            "{base}: openscion-server {} (protocol {major}.{minor})\n",
            env!("CARGO_PKG_VERSION")
        )
    );
}

#[test]
fn server_info_fails_when_the_server_is_unreachable() {
    let cli =
        Cli::try_parse_from(["opnscn", "--server", &unreachable_server(), "server-info"]).unwrap();
    assert!(matches!(
        run(&cli, &mut Vec::new()),
        Err(Error::Request { .. })
    ));
}

struct BrokenPipe;

impl Write for BrokenPipe {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_failed_write_is_an_output_error() {
    let cli = Cli::try_parse_from(["opnscn", "version"]).unwrap();
    let error = run(&cli, &mut BrokenPipe).unwrap_err();
    assert!(matches!(error, Error::Output(_)));
    assert!(error.to_string().starts_with("cannot write output: "));

    let cli = Cli::try_parse_from(["opnscn", "--server", &real_server(), "server-info"]).unwrap();
    assert!(matches!(run(&cli, &mut BrokenPipe), Err(Error::Output(_))));
}

#[test]
fn import_commit_and_clone_run_from_the_command_line() {
    let base = real_server();
    let source = scratch("cli-source");
    std::fs::write(source.join("a.txt"), b"a").unwrap();
    let dir = source.to_str().unwrap();

    let cli = Cli::try_parse_from(["opnscn", "--server", &base, "import", dir]).unwrap();
    assert!(output(&cli).starts_with("imported 1 files as "));

    std::fs::write(source.join("b.txt"), b"b").unwrap();
    let cli = Cli::try_parse_from(["opnscn", "commit", "-m", "add b", "--dir", dir]).unwrap();
    assert!(output(&cli).starts_with("committed 1 changes as "));

    let dest = scratch("cli-clone").join("copy");
    let dest = dest.to_str().unwrap();
    let cli = Cli::try_parse_from([
        "opnscn", "--server", &base, "clone", dest, "--path", "b.txt",
    ])
    .unwrap();
    assert!(output(&cli).starts_with("cloned 1 files of "));
}

#[test]
fn status_reports_changes_and_a_clean_tree_from_the_command_line() {
    let base = real_server();
    let source = scratch("cli-status");
    std::fs::write(source.join("keep.txt"), b"keep").unwrap();
    std::fs::write(source.join("edit.txt"), b"before").unwrap();
    std::fs::write(source.join("gone.txt"), b"gone").unwrap();
    let dir = source.to_str().unwrap();
    run_at(&base, &["import", dir]);

    // A fresh import has nothing pending.
    assert_eq!(run_at(&base, &["status", "--dir", dir]), "clean\n");

    std::fs::write(source.join("edit.txt"), b"after!").unwrap();
    std::fs::write(source.join("add.txt"), b"new").unwrap();
    std::fs::remove_file(source.join("gone.txt")).unwrap();
    assert_eq!(
        run_at(&base, &["status", "--dir", dir]),
        "A add.txt\nM edit.txt\nD gone.txt\n"
    );
}

#[test]
fn clone_rejects_an_invalid_path() {
    let cli = Cli::try_parse_from(["opnscn", "clone", "x", "--path", "../up"]).unwrap();
    assert!(matches!(run(&cli, &mut Vec::new()), Err(Error::Engine(_))));
}

#[test]
fn stat_ls_and_cat_read_a_snapshot() {
    let (base, c1, _c2) = history();
    let two = object_id(ObjectKind::Blob, b"two");

    assert_eq!(run_at(&base, &["stat", "a.txt"]), format!("file {two}\n"));
    assert_eq!(run_at(&base, &["stat", "missing"]), "absent\n".to_owned());
    assert_eq!(run_at(&base, &["ls"]), "a.txt\nb.txt\n".to_owned());
    // Against the root, only a.txt is there.
    assert_eq!(
        run_at(&base, &["stat", "b.txt", "--rev", &c1.to_string()]),
        "absent\n".to_owned()
    );
    assert_eq!(run_at(&base, &["cat", "a.txt"]), "two".to_owned());
}

#[test]
fn diff_log_ancestor_and_merge_base_read_history() {
    let (base, c1, c2) = history();

    assert_eq!(
        run_at(&base, &["diff", &c1.to_string(), "main"]),
        "+ a.txt\n+ b.txt\n".to_owned()
    );
    assert_eq!(
        run_at(&base, &["diff", &c1.to_string(), "main", "b.txt"]),
        "+ b.txt\n".to_owned()
    );
    assert_eq!(
        run_at(&base, &["log", "--rev", "main"]),
        format!("{c2} edit\n{c1} root\n")
    );
    assert_eq!(
        run_at(&base, &["log", "--rev", "main", "--limit", "1"]),
        format!("{c2} edit\n")
    );
    assert_eq!(
        run_at(&base, &["ancestor", &c1.to_string(), "main"]),
        "yes\n".to_owned()
    );
    assert_eq!(
        run_at(&base, &["ancestor", "main", &c1.to_string()]),
        "no\n".to_owned()
    );
    assert_eq!(
        run_at(&base, &["merge-base", &c1.to_string(), "main"]),
        format!("{c1}\n")
    );
}

#[test]
fn rebase_revert_and_land_move_changes() {
    let (base, c1, c2) = history();
    let client = ServerClient::new(&base);
    let c = put(&client, b"c");
    let side = client
        .create_changeset(&CreateChangeset::new(
            vec![c1],
            vec![upsert("c.txt", c)],
            "t",
            "side",
        ))
        .unwrap();

    // Rebasing the side branch onto main prints a fresh changeset id.
    let rebased = run_at(&base, &["rebase", &side.id.to_string(), "--onto", "main"]);
    let rebased: ObjectId = rebased.trim().parse().unwrap();
    assert_ne!(rebased, side.id);

    // Reverting main's edit onto main prints a fresh changeset id.
    let reverted = run_at(
        &base,
        &["revert", &c2.to_string(), "--onto", "main", "-m", "undo"],
    );
    assert_eq!(reverted.trim().len(), 64);

    // Landing a child of main advances the ref to version 2.
    let tip = client
        .create_changeset(&CreateChangeset::new(
            vec![c2],
            vec![upsert("d.txt", c)],
            "t",
            "tip",
        ))
        .unwrap();
    assert_eq!(
        run_at(&base, &["land", &tip.id.to_string()]),
        format!("landed {} on main at version 2\n", tip.id)
    );
    assert_eq!(client.get_ref("main").unwrap().target, tip.id);
}

#[test]
fn a_query_against_a_missing_ref_is_an_error() {
    let base = real_server();
    let cli = Cli::try_parse_from(["opnscn", "--server", &base, "ls", "--rev", "nope"]).unwrap();
    assert!(matches!(
        run(&cli, &mut Vec::new()),
        Err(Error::Server { .. })
    ));
}

#[test]
fn commands_default_their_ref_and_author() {
    let cli = Cli::try_parse_from(["opnscn", "import"]).unwrap();
    assert_eq!(
        cli.command,
        Command::Import {
            dir: ".".into(),
            ref_name: DEFAULT_REF.to_owned(),
            message: "import".to_owned(),
            author: std::env::var("OPENSCION_AUTHOR").unwrap_or_else(|_| DEFAULT_AUTHOR.to_owned()),
        }
    );
}

//! End-to-end tests for import, commit, and clone against a real server.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use openscion_core::{FrameWriter, ObjectId, ObjectKind, RepoPath, object_id};

use super::status::StatusCode;
use super::{clone, commit, import, status};
use crate::test_support::{real_server, scratch};
use crate::{Error, ServerClient};

/// Every file under `root` except the state directory, with its contents.
fn contents(root: &Path) -> BTreeMap<String, Vec<u8>> {
    crate::workcopy::scan(root)
        .unwrap()
        .into_iter()
        .map(|file| {
            let bytes = crate::workcopy::read_content(root, &file).unwrap();
            (file.path, bytes)
        })
        .collect()
}

fn seed(root: &Path) {
    for (path, body) in [
        ("README.md", "# demo\n"),
        ("src/lib.rs", "pub fn f() {}\n"),
        ("src/bin/main.rs", "fn main() {}\n"),
        ("docs/guide.md", "guide\n"),
        ("docs/copy.md", "guide\n"),
    ] {
        let full = root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, body).unwrap();
    }
}

fn imported() -> (ServerClient, std::path::PathBuf) {
    let client = ServerClient::new(real_server());
    let root = scratch("source");
    seed(&root);
    let (_, files) = import(&client, &root, "main", "test", "import").unwrap();
    assert_eq!(files, 5);
    (client, root)
}

#[test]
fn a_clone_reproduces_the_imported_tree() {
    let (client, root) = imported();
    let dest = scratch("clone").join("copy");
    let (target, files) = clone(&client, "main", &dest, None).unwrap();
    assert_eq!(files, 5);
    assert_eq!(contents(&dest), contents(&root));
    assert_eq!(client.get_ref("main").unwrap().target, target);
}

#[cfg(unix)]
#[test]
fn executables_and_symlinks_survive_a_round_trip() {
    use std::os::unix::fs::PermissionsExt;
    let client = ServerClient::new(real_server());
    let root = scratch("kinds-source");
    fs::write(root.join("run.sh"), b"#!/bin/sh\n").unwrap();
    fs::set_permissions(root.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("run.sh", root.join("link")).unwrap();
    import(&client, &root, "main", "t", "m").unwrap();
    let dest = scratch("kinds-clone");
    clone(&client, "main", &dest, None).unwrap();
    let kinds: Vec<_> = crate::workcopy::scan(&dest)
        .unwrap()
        .into_iter()
        .map(|file| file.kind)
        .collect();
    let source: Vec<_> = crate::workcopy::scan(&root)
        .unwrap()
        .into_iter()
        .map(|file| file.kind)
        .collect();
    assert_eq!(kinds, source);
}

#[test]
fn commit_records_edits_additions_and_deletions() {
    let (client, root) = imported();
    fs::write(root.join("README.md"), b"# changed\n").unwrap();
    fs::write(root.join("src/new.rs"), b"// new\n").unwrap();
    fs::remove_file(root.join("docs/guide.md")).unwrap();
    let (info, changes) = commit(&root, "test", "edit").unwrap();
    assert_eq!(changes, 3);
    assert_eq!(client.get_ref("main").unwrap().target, info.id);

    let dest = scratch("after-commit");
    clone(&client, "main", &dest, None).unwrap();
    assert_eq!(contents(&dest), contents(&root));
}

#[test]
fn a_clean_or_merely_touched_tree_has_nothing_to_commit() {
    let (_, root) = imported();
    assert!(matches!(
        commit(&root, "t", "m"),
        Err(Error::NothingToCommit)
    ));
    // Rewrite a file with identical bytes: its stat data changes, its
    // content does not.
    fs::write(root.join("README.md"), b"# demo\n").unwrap();
    assert!(matches!(
        commit(&root, "t", "m"),
        Err(Error::NothingToCommit)
    ));
}

#[test]
fn a_commit_from_a_clone_is_visible_to_the_next_clone() {
    let (client, _) = imported();
    let work = scratch("work");
    clone(&client, "main", &work, None).unwrap();
    fs::write(work.join("src/lib.rs"), b"pub fn g() {}\n").unwrap();
    commit(&work, "agent", "rename f").unwrap();
    let check = scratch("check");
    clone(&client, "main", &check, None).unwrap();
    assert_eq!(
        fs::read(check.join("src/lib.rs")).unwrap(),
        b"pub fn g() {}\n"
    );
}

#[test]
fn the_second_of_two_racing_commits_loses_the_swap() {
    let (client, _) = imported();
    let first = scratch("race-a");
    let second = scratch("race-b");
    clone(&client, "main", &first, None).unwrap();
    clone(&client, "main", &second, None).unwrap();
    fs::write(first.join("a.txt"), b"a").unwrap();
    fs::write(second.join("b.txt"), b"b").unwrap();
    commit(&first, "a", "first").unwrap();
    let error = commit(&second, "b", "second").unwrap_err();
    assert!(matches!(error, Error::Server { status: 409, ref code, .. } if code == "RefConflict"));
}

#[test]
fn a_sparse_clone_fetches_only_the_named_directory() {
    let (client, _) = imported();
    let dest = scratch("sparse");
    let path = RepoPath::new("src").unwrap();
    let (_, files) = clone(&client, "main", &dest, Some(&path)).unwrap();
    assert_eq!(files, 2);
    let paths: Vec<_> = contents(&dest).into_keys().collect();
    assert_eq!(paths, ["src/bin/main.rs", "src/lib.rs"]);

    let nested = scratch("sparse-nested");
    let path = RepoPath::new("src/bin").unwrap();
    clone(&client, "main", &nested, Some(&path)).unwrap();
    assert_eq!(
        contents(&nested).into_keys().collect::<Vec<_>>(),
        ["src/bin/main.rs"]
    );

    let single = scratch("sparse-file");
    let path = RepoPath::new("README.md").unwrap();
    clone(&client, "main", &single, Some(&path)).unwrap();
    assert_eq!(
        contents(&single).into_keys().collect::<Vec<_>>(),
        ["README.md"]
    );

    let none = scratch("sparse-none");
    let path = RepoPath::new("README.md/inside").unwrap();
    assert_eq!(clone(&client, "main", &none, Some(&path)).unwrap().1, 0);
}

#[test]
fn a_commit_from_a_sparse_clone_keeps_the_rest_of_the_tree() {
    let (client, root) = imported();
    let dest = scratch("sparse-commit");
    let path = RepoPath::new("docs").unwrap();
    clone(&client, "main", &dest, Some(&path)).unwrap();
    fs::write(dest.join("docs/guide.md"), b"better guide\n").unwrap();
    commit(&dest, "agent", "improve the guide").unwrap();

    let full = scratch("sparse-commit-check");
    clone(&client, "main", &full, None).unwrap();
    let mut expected = contents(&root);
    expected.insert("docs/guide.md".to_owned(), b"better guide\n".to_vec());
    assert_eq!(contents(&full), expected);
}

#[test]
fn status_reports_additions_modifications_and_deletions() {
    let (_, root) = imported();
    fs::write(root.join("README.md"), b"# changed\n").unwrap();
    fs::write(root.join("src/new.rs"), b"// new\n").unwrap();
    fs::remove_file(root.join("docs/guide.md")).unwrap();
    assert_eq!(
        status(&root).unwrap(),
        vec![
            ("README.md".to_owned(), StatusCode::Modified),
            ("docs/guide.md".to_owned(), StatusCode::Deleted),
            ("src/new.rs".to_owned(), StatusCode::Added),
        ]
    );
}

#[test]
fn status_is_clean_on_a_fresh_clone_and_after_a_mere_touch() {
    let (client, _) = imported();
    let work = scratch("status-clean");
    clone(&client, "main", &work, None).unwrap();
    assert!(status(&work).unwrap().is_empty());
    // Rewrite a file with its original bytes: its stat data moves, not content.
    fs::write(work.join("README.md"), b"# demo\n").unwrap();
    assert!(status(&work).unwrap().is_empty(), "a touch is not a change");
}

#[test]
fn status_reports_exactly_the_set_commit_records() {
    let (client, _) = imported();
    let work = scratch("status-eq");
    clone(&client, "main", &work, None).unwrap();
    fs::write(work.join("README.md"), b"# changed\n").unwrap();
    fs::write(work.join("extra.txt"), b"x\n").unwrap();
    fs::remove_file(work.join("docs/copy.md")).unwrap();

    let reported: Vec<String> = status(&work)
        .unwrap()
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    assert_eq!(reported, ["README.md", "docs/copy.md", "extra.txt"]);
    // The read-only report left the same change set for commit to record.
    let (_, changes) = commit(&work, "t", "m").unwrap();
    assert_eq!(changes, reported.len());
}

#[test]
fn status_needs_no_server() {
    let (client, _) = imported();
    let work = scratch("status-offline");
    clone(&client, "main", &work, None).unwrap();
    // Point the state at a server nothing listens on; status must not care.
    let mut state = crate::workcopy::load(&work).unwrap();
    state.server = "http://127.0.0.1:1".to_owned();
    crate::workcopy::save(&work, &mut state).unwrap();
    fs::write(work.join("new.txt"), b"n\n").unwrap();
    assert_eq!(
        status(&work).unwrap(),
        vec![("new.txt".to_owned(), StatusCode::Added)]
    );
}

#[test]
fn status_does_not_write_the_state() {
    let (client, _) = imported();
    let work = scratch("status-readonly");
    clone(&client, "main", &work, None).unwrap();
    // A touched-but-identical file would tempt a stat-cache refresh.
    fs::write(work.join("README.md"), b"# demo\n").unwrap();
    let before = fs::read(work.join(".opnscn/state.json")).unwrap();
    status(&work).unwrap();
    status(&work).unwrap();
    assert_eq!(
        fs::read(work.join(".opnscn/state.json")).unwrap(),
        before,
        "status must not write state"
    );
}

#[test]
fn status_outside_a_working_copy_fails() {
    let root = scratch("status-not-a-copy");
    assert!(matches!(status(&root), Err(Error::NotAWorkingCopy(_))));
}

#[test]
fn import_and_clone_refuse_an_occupied_directory() {
    let (client, root) = imported();
    assert!(matches!(
        import(&client, &root, "main", "t", "m"),
        Err(Error::AlreadyExists(_))
    ));
    assert!(matches!(
        clone(&client, "main", &root, None),
        Err(Error::AlreadyExists(_))
    ));
}

#[test]
fn commit_outside_a_working_copy_fails() {
    let root = scratch("not-a-copy");
    assert!(matches!(
        commit(&root, "t", "m"),
        Err(Error::NotAWorkingCopy(_))
    ));
}

#[test]
fn cloning_a_missing_ref_reports_the_server_error() {
    let client = ServerClient::new(real_server());
    let dest = scratch("no-ref");
    let error = clone(&client, "main", &dest, None).unwrap_err();
    assert!(matches!(error, Error::Server { status: 404, .. }));
}

#[test]
fn fetched_objects_are_verified() {
    let id = object_id(ObjectKind::Blob, b"good");
    let mut frames = FrameWriter::new();
    frames.push(id, b"evil");
    let bytes = frames.finish();
    let error = super::clone::verified(&bytes, ObjectKind::Blob, &[id]).unwrap_err();
    assert!(matches!(
        error,
        Error::Engine(openscion_core::Error::CorruptObject { .. })
    ));

    let missing = ObjectId::from_bytes([9; 32]);
    let error = super::clone::verified(&[], ObjectKind::Blob, &[missing]).unwrap_err();
    assert!(matches!(
        error,
        Error::Engine(openscion_core::Error::NotFound(id)) if id == missing
    ));
}

#[test]
fn uploads_are_split_into_batches() {
    let client = ServerClient::new(real_server());
    let blobs: Vec<_> = [b"aaaa", b"bbbb", b"cccc"]
        .iter()
        .map(|bytes| (object_id(ObjectKind::Blob, *bytes), bytes.to_vec()))
        .collect();
    // Each frame is 40 bytes of header plus 4 of body: two fit in 100 bytes.
    assert_eq!(super::upload_in_batches(&client, &blobs, 100).unwrap(), 2);
    assert_eq!(super::upload_in_batches(&client, &[], 100).unwrap(), 0);
    let ids: Vec<_> = blobs.iter().map(|(id, _)| *id).collect();
    assert!(!client.fetch_objects(ids).unwrap().is_empty());
}

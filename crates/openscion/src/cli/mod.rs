//! Command-line parsing and dispatch.
//!
//! [`Cli`] is the parsed command line; [`run`] executes it and writes the
//! result to any [`Write`], so commands are tested without spawning a process.

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use openscion_core::{PROTOCOL_VERSION, RepoPath};

use crate::{Error, Result, ServerClient, commands};

/// The server `opnscn` talks to when neither `--server` nor `OPENSCION_SERVER`
/// is set: a local `openscion-server` on its default port.
pub const DEFAULT_SERVER: &str = "http://127.0.0.1:7878";

/// The parsed `opnscn` command line.
#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(name = "opnscn", version, about = "The OpenScion command-line client")]
#[non_exhaustive]
pub struct Cli {
    /// Base URL of the OpenScion server.
    #[arg(long, global = true, env = "OPENSCION_SERVER", default_value = DEFAULT_SERVER)]
    pub server: String,

    /// The command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// The ref commands read and move when `--ref` is not given.
pub const DEFAULT_REF: &str = "main";

/// The author recorded when neither `--author` nor `OPENSCION_AUTHOR` is set.
pub const DEFAULT_AUTHOR: &str = "opnscn";

/// How many times `land` rebases and retries before giving up to contention.
pub const DEFAULT_LAND_ATTEMPTS: u32 = 10;

/// The commands `opnscn` understands.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
#[non_exhaustive]
pub enum Command {
    /// Print the client and protocol versions.
    Version,
    /// Ask the server for its version and check that its protocol binds.
    ServerInfo,
    /// Record a directory as the repository's first changeset and create the
    /// ref; the directory becomes a working copy.
    Import {
        /// The directory to import.
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// The ref to create.
        #[arg(long = "ref", default_value = DEFAULT_REF)]
        ref_name: String,
        /// The changeset message.
        #[arg(short, long, default_value = "import")]
        message: String,
        /// Who the changeset is attributed to.
        #[arg(long, env = "OPENSCION_AUTHOR", default_value = DEFAULT_AUTHOR)]
        author: String,
    },
    /// Record every change in a working copy and move its ref by
    /// compare-and-swap. Talks to the server the working copy came from.
    Commit {
        /// The changeset message.
        #[arg(short, long)]
        message: String,
        /// The working copy.
        #[arg(long, default_value = ".")]
        dir: PathBuf,
        /// Who the changeset is attributed to.
        #[arg(long, env = "OPENSCION_AUTHOR", default_value = DEFAULT_AUTHOR)]
        author: String,
    },
    /// Report a working copy's pending changes against its base changeset:
    /// added, modified, and deleted files. Reads only local state; prints
    /// `clean` when nothing has changed.
    Status {
        /// The working copy.
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
    /// Fetch a ref's snapshot into a new working copy, or only one directory
    /// of it with `--path`.
    Clone {
        /// Where to write the working copy; must be absent or empty.
        dest: PathBuf,
        /// The ref to clone.
        #[arg(long = "ref", default_value = DEFAULT_REF)]
        ref_name: String,
        /// Fetch only this directory (or file) of the snapshot.
        #[arg(long)]
        path: Option<String>,
    },
    /// Print what kind of entry and which object is at a path in a snapshot.
    Stat {
        /// The path to resolve.
        path: String,
        /// The revision to read: an object id or a ref name.
        #[arg(long, default_value = DEFAULT_REF)]
        rev: String,
    },
    /// List the entries under a path, or the root, in a snapshot.
    Ls {
        /// The directory to list; the root when omitted.
        path: Option<String>,
        /// The revision to read: an object id or a ref name.
        #[arg(long, default_value = DEFAULT_REF)]
        rev: String,
        /// List every entry beneath, not just the direct children.
        #[arg(long)]
        recursive: bool,
    },
    /// Write the bytes of a file in a snapshot to standard output.
    Cat {
        /// The file to read.
        path: String,
        /// The revision to read: an object id or a ref name.
        #[arg(long, default_value = DEFAULT_REF)]
        rev: String,
    },
    /// Print the changes that turn one snapshot into another.
    Diff {
        /// The revision to diff from: an object id or a ref name.
        from: String,
        /// The revision to diff to: an object id or a ref name.
        to: String,
        /// Keep only changes under these paths; all changes when omitted.
        paths: Vec<String>,
    },
    /// Print the first-parent history of a revision, newest first.
    Log {
        /// Keep only changesets that changed this path.
        path: Option<String>,
        /// The revision to walk back from: an object id or a ref name.
        #[arg(long, default_value = DEFAULT_REF)]
        rev: String,
        /// Stop after this many changesets.
        #[arg(long, default_value_t = commands::DEFAULT_LOG_LIMIT)]
        limit: u64,
        /// Stop before reaching this revision.
        #[arg(long)]
        until: Option<String>,
    },
    /// Print whether one revision is an ancestor of another.
    Ancestor {
        /// The revision that might be an ancestor.
        ancestor: String,
        /// The revision whose ancestry is searched.
        descendant: String,
    },
    /// Print a lowest common ancestor of two revisions.
    MergeBase {
        /// One revision.
        a: String,
        /// The other revision.
        b: String,
    },
    /// Rebase a changeset onto another and print the result.
    Rebase {
        /// The revision to rebase.
        rev: String,
        /// The revision to rebase onto.
        #[arg(long)]
        onto: String,
    },
    /// Revert a changeset onto another and print the result.
    Revert {
        /// The revision to undo.
        rev: String,
        /// The revision to apply the undo onto.
        #[arg(long)]
        onto: String,
        /// The revert's message.
        #[arg(short, long, default_value = "revert")]
        message: String,
        /// Who the revert is attributed to.
        #[arg(long, env = "OPENSCION_AUTHOR", default_value = DEFAULT_AUTHOR)]
        author: String,
    },
    /// Move a ref to include a changeset by compare-and-swap, rebasing onto the
    /// tip when another writer got there first.
    Land {
        /// The revision to land.
        rev: String,
        /// The ref to move.
        #[arg(long = "ref", default_value = DEFAULT_REF)]
        ref_name: String,
        /// How many times to rebase and retry before giving up.
        #[arg(long, default_value_t = DEFAULT_LAND_ATTEMPTS)]
        attempts: u32,
    },
}

/// Runs `cli`, writing its output to `out`.
///
/// # Errors
///
/// Returns the [`ServerClient`] error for commands that reach a server,
/// [`Error::Io`] and the working-copy variants for commands that touch the
/// disk, and [`Error::Output`] when writing to `out` fails.
pub fn run(cli: &Cli, out: &mut impl Write) -> Result<()> {
    let client = ServerClient::new(&cli.server);
    match &cli.command {
        Command::Version => version(out),
        Command::ServerInfo => server_info(&client, &cli.server, out),
        Command::Import {
            dir,
            ref_name,
            message,
            author,
        } => import_cmd(&client, dir, ref_name, message, author, out),
        Command::Commit {
            message,
            dir,
            author,
        } => commit_cmd(dir, author, message, out),
        Command::Status { dir } => status_cmd(dir, out),
        Command::Clone {
            dest,
            ref_name,
            path,
        } => clone_cmd(&client, dest, ref_name, path.as_deref(), out),
        Command::Stat { path, rev } => commands::stat(&client, rev, path, out),
        Command::Ls {
            path,
            rev,
            recursive,
        } => commands::ls(&client, rev, path.as_deref(), *recursive, out),
        Command::Cat { path, rev } => commands::cat(&client, rev, path, out),
        Command::Diff { from, to, paths } => commands::diff(&client, from, to, paths, out),
        Command::Log {
            path,
            rev,
            limit,
            until,
        } => commands::log(&client, rev, path.as_deref(), until.as_deref(), *limit, out),
        Command::Ancestor {
            ancestor,
            descendant,
        } => commands::ancestor(&client, ancestor, descendant, out),
        Command::MergeBase { a, b } => commands::merge_base(&client, a, b, out),
        Command::Rebase { rev, onto } => commands::rebase(&client, rev, onto, out),
        Command::Revert {
            rev,
            onto,
            message,
            author,
        } => commands::revert(&client, rev, onto, author, message, out),
        Command::Land {
            rev,
            ref_name,
            attempts,
        } => commands::land(&client, rev, ref_name, *attempts, out),
    }
}

/// Prints the client's own version and the protocol it speaks.
fn version(out: &mut impl Write) -> Result<()> {
    let (major, minor) = PROTOCOL_VERSION;
    writeln!(
        out,
        "opnscn {} (protocol {major}.{minor})",
        env!("CARGO_PKG_VERSION")
    )
    .map_err(Error::Output)
}

/// Prints what the server at `base` reports, after checking the protocol binds.
fn server_info(client: &ServerClient, base: &str, out: &mut impl Write) -> Result<()> {
    let info = client.info()?;
    let (major, minor) = info.protocol_version;
    writeln!(
        out,
        "{base}: openscion-server {} (protocol {major}.{minor})",
        info.server_version
    )
    .map_err(Error::Output)
}

/// Imports `dir` as the first changeset on `ref_name` and reports the count.
fn import_cmd(
    client: &ServerClient,
    dir: &Path,
    ref_name: &str,
    message: &str,
    author: &str,
    out: &mut impl Write,
) -> Result<()> {
    let (info, files) = commands::import(client, dir, ref_name, author, message)?;
    writeln!(out, "imported {files} files as {} on {ref_name}", info.id).map_err(Error::Output)
}

/// Commits the changes in the working copy at `dir` and reports the count.
fn commit_cmd(dir: &Path, author: &str, message: &str, out: &mut impl Write) -> Result<()> {
    let (info, changes) = commands::commit(dir, author, message)?;
    writeln!(out, "committed {changes} changes as {}", info.id).map_err(Error::Output)
}

/// Prints the working copy's pending changes as `<letter> <path>` lines,
/// sorted by path, or `clean` when there are none.
fn status_cmd(dir: &Path, out: &mut impl Write) -> Result<()> {
    let changes = commands::status(dir)?;
    if changes.is_empty() {
        return writeln!(out, "clean").map_err(Error::Output);
    }
    for (path, code) in changes {
        writeln!(out, "{} {path}", code.letter()).map_err(Error::Output)?;
    }
    Ok(())
}

/// Clones `ref_name` into `dest`, optionally only the directory at `path`.
fn clone_cmd(
    client: &ServerClient,
    dest: &Path,
    ref_name: &str,
    path: Option<&str>,
    out: &mut impl Write,
) -> Result<()> {
    let sparse = path.map(RepoPath::new).transpose()?;
    let (target, files) = commands::clone(client, ref_name, dest, sparse.as_ref())?;
    writeln!(
        out,
        "cloned {files} files of {target} into {}",
        dest.display()
    )
    .map_err(Error::Output)
}

#[cfg(test)]
mod test;

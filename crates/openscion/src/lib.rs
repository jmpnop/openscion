//! `opnscn`, the OpenScion command-line client.
//!
//! In the full design (`docs/specs/openscion-architecture.md`) `opnscn` lays out
//! a sparse working copy, watches it for edits, uploads blobs, and wraps the
//! server's history and landing APIs. Today it implements:
//!
//! | Command | What it does |
//! | --- | --- |
//! | `opnscn version` | prints the client and protocol versions |
//! | `opnscn server-info` | asks the server for its version and checks the protocol binds |
//! | `opnscn import [DIR]` | records a directory as the first changeset and creates `main` |
//! | `opnscn commit -m MSG` | records what changed and moves `main` by compare-and-swap |
//! | `opnscn status` | reports a working copy's pending changes, read-only |
//! | `opnscn watch` | renders those changes live as files change, via a filesystem watcher |
//! | `opnscn clone DEST [--path DIR]` | fetches a snapshot, or one directory of it |
//! | `opnscn sparse [add\|remove PATHS…]` | shows or edits which paths the working copy materializes |
//! | `opnscn stat PATH` | prints the entry at a path in a snapshot |
//! | `opnscn ls [PATH]` | lists the entries under a path, with `--recursive` |
//! | `opnscn cat PATH` | writes a file's bytes to standard output |
//! | `opnscn diff FROM TO [PATHS…]` | prints the changes between two snapshots |
//! | `opnscn log [PATH]` | prints first-parent history, newest first |
//! | `opnscn ancestor A B` | prints whether `A` is an ancestor of `B` |
//! | `opnscn merge-base A B` | prints a lowest common ancestor |
//! | `opnscn rebase REV --onto REV` | rebases a changeset and prints the result |
//! | `opnscn revert REV --onto REV` | reverts a changeset and prints the result |
//! | `opnscn land REV [--ref NAME]` | moves a ref to include a changeset |
//!
//! Commands that read or move history name a revision by object id or ref
//! name; a ref name resolves to its tip. The query and landing commands
//! default that revision to `main`.
//!
//! A working copy keeps its state in `.opnscn/state.json`: the server, the ref,
//! the base changeset, and each file's blob id and stat data, so a commit
//! re-reads only files whose size or modification time changed.
//!
//! Every command that talks to a server takes `--server <url>` or the
//! `OPENSCION_SERVER` environment variable, defaulting to
//! [`DEFAULT_SERVER`].
//!
//! # Example
//!
//! ```
//! use clap::Parser;
//! use openscion::{Cli, run};
//!
//! let cli = Cli::parse_from(["opnscn", "version"]);
//! let mut out = Vec::new();
//! run(&cli, &mut out)?;
//! assert!(String::from_utf8(out).unwrap().starts_with("opnscn "));
//! # Ok::<(), openscion::Error>(())
//! ```

mod cli;
mod client;
mod commands;
mod error;
#[cfg(test)]
mod test_support;
mod watch;
mod workcopy;

pub use cli::{
    Cli, Command, DEFAULT_AUTHOR, DEFAULT_LAND_ATTEMPTS, DEFAULT_REF, DEFAULT_SERVER, SparseAction,
    run,
};
pub use client::ServerClient;
pub use error::{Error, Result};

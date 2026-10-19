//! The server query commands: `stat`, `ls`, `cat`, `diff`, `log`, `ancestor`,
//! and `merge-base`.
//!
//! Each resolves its revisions — an object id or a ref name — through
//! [`resolve_rev`](super::resolve_rev), makes one client call, and writes a
//! line-oriented summary. None of them touch a working copy; they read what the
//! server already knows.

use std::io::Write;

use openscion_core::{Change, EntryKind, RepoPath};

use super::resolve_rev;
use crate::{Error, Result, ServerClient};

/// How many changesets `log` shows when `--limit` is not given.
pub(crate) const DEFAULT_LOG_LIMIT: u64 = 20;

/// Prints the entry at `path` in `rev`'s snapshot, or `absent`.
pub(crate) fn stat(
    client: &ServerClient,
    rev: &str,
    path: &str,
    out: &mut impl Write,
) -> Result<()> {
    let changeset = resolve_rev(client, rev)?;
    match client.stat(changeset, path)?.entry {
        Some(entry) => writeln!(out, "{} {}", kind_label(entry.kind), entry.id),
        None => writeln!(out, "absent"),
    }
    .map_err(Error::Output)
}

/// Lists the entries under `path` (or the root) in `rev`'s snapshot, one per
/// line, with a trailing `/` on directories.
pub(crate) fn ls(
    client: &ServerClient,
    rev: &str,
    path: Option<&str>,
    recursive: bool,
    out: &mut impl Write,
) -> Result<()> {
    let changeset = resolve_rev(client, rev)?;
    for entry in client.list_tree(changeset, path, recursive)?.entries {
        let slash = if entry.kind == EntryKind::Tree {
            "/"
        } else {
            ""
        };
        writeln!(out, "{}{slash}", entry.path).map_err(Error::Output)?;
    }
    Ok(())
}

/// Writes the bytes of the file at `path` in `rev`'s snapshot.
pub(crate) fn cat(
    client: &ServerClient,
    rev: &str,
    path: &str,
    out: &mut impl Write,
) -> Result<()> {
    let changeset = resolve_rev(client, rev)?;
    out.write_all(&client.read_file(changeset, path)?)
        .map_err(Error::Output)
}

/// Prints the changes that turn `from` into `to`: `+ path` for an upsert,
/// `- path` for a delete. `prefixes` keeps only changes under those paths.
pub(crate) fn diff(
    client: &ServerClient,
    from: &str,
    to: &str,
    prefixes: &[String],
    out: &mut impl Write,
) -> Result<()> {
    let from = resolve_rev(client, from)?;
    let to = resolve_rev(client, to)?;
    let prefixes: Vec<&str> = prefixes.iter().map(String::as_str).collect();
    for change in client.diff(from, to, &prefixes)?.changes {
        let (marker, path) = match &change {
            Change::Upsert { path, .. } => ('+', path),
            Change::Delete { path } => ('-', path),
        };
        writeln!(out, "{marker} {}", path.as_str()).map_err(Error::Output)?;
    }
    Ok(())
}

/// Prints `id message` for each changeset on the first-parent walk from `rev`,
/// newest first.
pub(crate) fn log(
    client: &ServerClient,
    rev: &str,
    path: Option<&str>,
    until: Option<&str>,
    limit: u64,
    out: &mut impl Write,
) -> Result<()> {
    let from = resolve_rev(client, rev)?;
    let until = until.map(|rev| resolve_rev(client, rev)).transpose()?;
    let path = path.map(RepoPath::new).transpose()?;
    for summary in client.log(from, until, path.as_ref(), limit)?.changesets {
        writeln!(out, "{} {}", summary.id, summary.message).map_err(Error::Output)?;
    }
    Ok(())
}

/// Prints `yes` when `ancestor` reaches `descendant` along parent edges, else
/// `no`.
pub(crate) fn ancestor(
    client: &ServerClient,
    ancestor: &str,
    descendant: &str,
    out: &mut impl Write,
) -> Result<()> {
    let ancestor = resolve_rev(client, ancestor)?;
    let descendant = resolve_rev(client, descendant)?;
    let answer = client.is_ancestor(ancestor, descendant)?.is_ancestor;
    writeln!(out, "{}", if answer { "yes" } else { "no" }).map_err(Error::Output)
}

/// Prints a lowest common ancestor of `a` and `b`, or `none`.
pub(crate) fn merge_base(
    client: &ServerClient,
    a: &str,
    b: &str,
    out: &mut impl Write,
) -> Result<()> {
    let a = resolve_rev(client, a)?;
    let b = resolve_rev(client, b)?;
    match client.merge_base(a, b)?.base {
        Some(id) => writeln!(out, "{id}"),
        None => writeln!(out, "none"),
    }
    .map_err(Error::Output)
}

/// The lowercase name of an entry kind.
fn kind_label(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::File => "file",
        EntryKind::Executable => "executable",
        EntryKind::Symlink => "symlink",
        EntryKind::Tree => "tree",
    }
}

//! Translating git tree entries and diffing flattened git trees — the pure
//! part of ingestion, independent of walking a repository.

use std::collections::BTreeMap;

use gix::ObjectId;
use gix::objs::tree::EntryKind as GitEntryKind;
use openscion_core::{EntryKind, RepoPath};

use crate::Result;

/// How a git tree entry's mode maps into OpenScion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mapped {
    /// A file, executable, or symlink to record under its path.
    File(EntryKind),
    /// A directory to descend into; not itself a tracked entry.
    Tree,
    /// A submodule gitlink, which OpenScion does not represent: skip it.
    Submodule,
}

/// Classifies a git tree entry's mode into [`Mapped`].
pub(crate) fn classify(kind: GitEntryKind) -> Mapped {
    match kind {
        GitEntryKind::Tree => Mapped::Tree,
        GitEntryKind::Blob => Mapped::File(EntryKind::File),
        GitEntryKind::BlobExecutable => Mapped::File(EntryKind::Executable),
        GitEntryKind::Link => Mapped::File(EntryKind::Symlink),
        GitEntryKind::Commit => Mapped::Submodule,
    }
}

/// A tracked file in a flattened git tree: its kind and the git blob it points
/// at. Two entries are equal when both match, so a kind change or a content
/// change both read as changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) kind: EntryKind,
    pub(crate) oid: ObjectId,
}

/// A file to materialize: its OpenScion path and kind, and the git blob whose
/// bytes the driver fetches and puts in the object store.
pub(crate) struct Upsert {
    pub(crate) path: RepoPath,
    pub(crate) kind: EntryKind,
    pub(crate) oid: ObjectId,
}

/// The difference between two flattened trees.
pub(crate) struct TreeDiff {
    /// Paths that are new or whose kind or content changed.
    pub(crate) upserts: Vec<Upsert>,
    /// Paths that are gone.
    pub(crate) deletes: Vec<RepoPath>,
}

/// Diffs `previous` into `current`: a path that is new, or whose kind or blob
/// differs, is an upsert; a path gone from `current` is a delete. Both inputs
/// are keyed by `/`-separated path.
///
/// # Errors
///
/// Returns [`Error::Core`](crate::Error::Core) when a path is not a valid
/// OpenScion repository path.
pub(crate) fn diff(
    previous: &BTreeMap<String, Entry>,
    current: &BTreeMap<String, Entry>,
) -> Result<TreeDiff> {
    let mut upserts = Vec::new();
    for (path, entry) in current {
        if previous.get(path) != Some(entry) {
            upserts.push(Upsert {
                path: RepoPath::new(path.as_str())?,
                kind: entry.kind,
                oid: entry.oid,
            });
        }
    }
    let mut deletes = Vec::new();
    for path in previous.keys() {
        if !current.contains_key(path) {
            deletes.push(RepoPath::new(path.as_str())?);
        }
    }
    Ok(TreeDiff { upserts, deletes })
}

#[cfg(test)]
mod test;

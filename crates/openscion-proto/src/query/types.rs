//! The query payload types: snapshot reads and history questions.

use serde::{Deserialize, Serialize};

use crate::{Change, ChangesetSummary, EntryKind, ObjectId, RepoPath};

/// One entry resolved in a snapshot: a path, what is there, and its object id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Entry {
    /// The entry's path, relative to the repository root.
    pub path: String,
    /// File, executable, symlink, or directory.
    pub kind: EntryKind,
    /// The blob id for files and symlinks, the tree id for directories.
    pub id: ObjectId,
}

impl Entry {
    /// Builds an entry.
    #[must_use]
    pub fn new(path: impl Into<String>, kind: EntryKind, id: ObjectId) -> Self {
        Self {
            path: path.into(),
            kind,
            id,
        }
    }
}

/// What a `Stat` answers: the entry at the path, or nothing when absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct StatResult {
    /// The entry at the queried path, or `None` when nothing is there.
    pub entry: Option<Entry>,
}

impl StatResult {
    /// Builds a stat result.
    #[must_use]
    pub fn new(entry: Option<Entry>) -> Self {
        Self { entry }
    }
}

/// What a `ListTree` answers: the entries under a path, sorted by path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TreeListing {
    /// The entries, sorted by path; empty when the path is absent.
    pub entries: Vec<Entry>,
}

impl TreeListing {
    /// Builds a tree listing.
    #[must_use]
    pub fn new(entries: Vec<Entry>) -> Self {
        Self { entries }
    }
}

/// A request to diff one snapshot into another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DiffRequest {
    /// The snapshot to diff from.
    pub from: ObjectId,
    /// The snapshot to diff to.
    pub to: ObjectId,
    /// Keep only changes under one of these path prefixes; empty keeps all.
    #[serde(default)]
    pub prefixes: Vec<String>,
}

impl DiffRequest {
    /// Builds a diff request.
    #[must_use]
    pub fn new(from: ObjectId, to: ObjectId, prefixes: Vec<String>) -> Self {
        Self { from, to, prefixes }
    }
}

/// What a `Diff` answers: the changes that turn `from` into `to`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DiffResult {
    /// The changes, sorted by path.
    pub changes: Vec<Change>,
}

impl DiffResult {
    /// Builds a diff result.
    #[must_use]
    pub fn new(changes: Vec<Change>) -> Self {
        Self { changes }
    }
}

/// A request to walk first-parent history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LogRequest {
    /// The changeset to walk back from, newest first.
    pub from: ObjectId,
    /// Stop before reaching this changeset, if given.
    #[serde(default)]
    pub until: Option<ObjectId>,
    /// Keep only changesets that changed this path, if given.
    #[serde(default)]
    pub path: Option<RepoPath>,
    /// Stop after this many changesets.
    pub limit: u64,
}

impl LogRequest {
    /// Builds a log request with no `until` and no path filter.
    #[must_use]
    pub fn new(from: ObjectId, limit: u64) -> Self {
        Self {
            from,
            until: None,
            path: None,
            limit,
        }
    }
}

/// What a `Log` answers: a page of first-parent history, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LogPage {
    /// The changesets, newest first.
    pub changesets: Vec<ChangesetSummary>,
}

impl LogPage {
    /// Builds a log page.
    #[must_use]
    pub fn new(changesets: Vec<ChangesetSummary>) -> Self {
        Self { changesets }
    }
}

/// A request to test whether one changeset is an ancestor of another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AncestorQuery {
    /// The changeset that might be an ancestor.
    pub ancestor: ObjectId,
    /// The changeset whose ancestry is searched.
    pub descendant: ObjectId,
}

impl AncestorQuery {
    /// Builds an ancestor query.
    #[must_use]
    pub fn new(ancestor: ObjectId, descendant: ObjectId) -> Self {
        Self {
            ancestor,
            descendant,
        }
    }
}

/// What an `IsAncestor` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AncestorResult {
    /// Whether the ancestor reaches the descendant along parent edges.
    pub is_ancestor: bool,
}

impl AncestorResult {
    /// Builds an ancestor result.
    #[must_use]
    pub fn new(is_ancestor: bool) -> Self {
        Self { is_ancestor }
    }
}

/// A request for a lowest common ancestor of two changesets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MergeBaseQuery {
    /// One changeset.
    pub a: ObjectId,
    /// The other changeset.
    pub b: ObjectId,
}

impl MergeBaseQuery {
    /// Builds a merge-base query.
    #[must_use]
    pub fn new(a: ObjectId, b: ObjectId) -> Self {
        Self { a, b }
    }
}

/// What a `MergeBase` answers: a lowest common ancestor, or nothing when the
/// two changesets share none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MergeBaseResult {
    /// The merge base, or `None` when there is no common ancestor.
    pub base: Option<ObjectId>,
}

impl MergeBaseResult {
    /// Builds a merge-base result.
    #[must_use]
    pub fn new(base: Option<ObjectId>) -> Self {
        Self { base }
    }
}

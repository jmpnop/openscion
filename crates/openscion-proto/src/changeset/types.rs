//! The changeset payload types.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{EntryKind, ObjectId, RepoPath};

/// One file-level change relative to a changeset's first parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Change {
    /// Create or replace the file at `path`. `kind` is never [`EntryKind::Tree`].
    Upsert {
        /// The file's path.
        path: RepoPath,
        /// File, executable, or symlink.
        kind: EntryKind,
        /// The blob holding the new contents, already in the object store.
        blob: ObjectId,
    },
    /// Remove the file at `path`.
    Delete {
        /// The file's path.
        path: RepoPath,
    },
}

impl Change {
    /// The path the change touches.
    #[must_use]
    pub fn path(&self) -> &RepoPath {
        match self {
            Self::Upsert { path, .. } | Self::Delete { path } => path,
        }
    }
}

/// A request to create a changeset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CreateChangeset {
    /// Parent changeset ids, first parent first; empty only for the root.
    pub parents: Vec<ObjectId>,
    /// Changes relative to the first parent, in any order.
    pub changes: Vec<Change>,
    /// Who wrote it: a human or an agent.
    pub author: String,
    /// The description.
    pub message: String,
    /// Tool metadata.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

impl CreateChangeset {
    /// Builds a request with no metadata.
    #[must_use]
    pub fn new(
        parents: Vec<ObjectId>,
        changes: Vec<Change>,
        author: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            parents,
            changes,
            author: author.into(),
            message: message.into(),
            metadata: BTreeMap::new(),
        }
    }
}

/// A changeset as stored: the request plus its server-assigned time, with
/// changes sorted by path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Changeset {
    /// Parent changeset ids, first parent first.
    pub parents: Vec<ObjectId>,
    /// Changes relative to the first parent, sorted by path.
    pub changes: Vec<Change>,
    /// Who wrote it.
    pub author: String,
    /// The description.
    pub message: String,
    /// Creation time, milliseconds since the Unix epoch, set by the server.
    pub created_at: u64,
    /// Tool metadata, sorted by key.
    pub metadata: BTreeMap<String, String>,
}

impl Changeset {
    /// Builds a stored changeset from a request and its creation time,
    /// sorting the changes by path.
    #[must_use]
    pub fn from_request(request: CreateChangeset, created_at: u64) -> Self {
        let mut changes = request.changes;
        changes.sort_by(|a, b| a.path().cmp(b.path()));
        Self {
            parents: request.parents,
            changes,
            author: request.author,
            message: request.message,
            created_at,
            metadata: request.metadata,
        }
    }
}

/// One row of a `Log`: enough to describe a changeset without its changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ChangesetSummary {
    /// The changeset's id.
    pub id: ObjectId,
    /// Its parents, first parent first.
    pub parents: Vec<ObjectId>,
    /// Who wrote it.
    pub author: String,
    /// The description.
    pub message: String,
    /// Creation time, milliseconds since the Unix epoch.
    pub created_at: u64,
}

impl ChangesetSummary {
    /// Builds a summary.
    #[must_use]
    pub fn new(
        id: ObjectId,
        parents: Vec<ObjectId>,
        author: impl Into<String>,
        message: impl Into<String>,
        created_at: u64,
    ) -> Self {
        Self {
            id,
            parents,
            author: author.into(),
            message: message.into(),
            created_at,
        }
    }
}

/// What the server answers after appending a changeset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ChangesetInfo {
    /// The changeset's id.
    pub id: ObjectId,
    /// Its position in the log.
    pub lsn: u64,
    /// The root tree of its snapshot.
    pub root_tree: ObjectId,
}

impl ChangesetInfo {
    /// Builds the summary.
    #[must_use]
    pub fn new(id: ObjectId, lsn: u64, root_tree: ObjectId) -> Self {
        Self { id, lsn, root_tree }
    }
}

/// A changeset read back from the log, with its summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ChangesetRecord {
    /// Id, log position, and root tree.
    #[serde(flatten)]
    pub info: ChangesetInfo,
    /// The changeset itself.
    #[serde(flatten)]
    pub changeset: Changeset,
}

impl ChangesetRecord {
    /// Pairs a changeset with its summary.
    #[must_use]
    pub fn new(info: ChangesetInfo, changeset: Changeset) -> Self {
        Self { info, changeset }
    }
}

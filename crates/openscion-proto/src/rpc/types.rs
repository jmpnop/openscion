//! The forwarded-RPC envelope types.

use serde::{Deserialize, Serialize};

use crate::changeset::{ChangesetInfo, ChangesetRecord, CreateChangeset};
use crate::failure::ErrorBody;
use crate::landing::{LandRequest, LandResult, RebaseRequest, RebaseResult, RevertRequest};
use crate::object::ObjectId;
use crate::query::{
    AncestorQuery, AncestorResult, DiffRequest, DiffResult, LogPage, LogRequest, MergeBaseQuery,
    MergeBaseResult, StatResult, TreeListing,
};
use crate::refs::{RefValue, UpdateRef};

/// A non-object operation a tier forwards up to the server, where it is
/// answered. One request type carries every such operation, so a lower tier
/// forwards it without understanding it and the server dispatches on the
/// variant — the uniform object RPCs' counterpart for the mutable queries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Create a changeset.
    CreateChangeset(CreateChangeset),
    /// Read the changeset with this id.
    GetChangeset(ObjectId),
    /// The entry at a path in a changeset's snapshot.
    Stat {
        /// The changeset whose snapshot to read.
        changeset: ObjectId,
        /// The path to resolve, relative to the repository root.
        path: String,
    },
    /// The entries under a path in a changeset's snapshot.
    ListTree {
        /// The changeset whose snapshot to read.
        changeset: ObjectId,
        /// The directory to list, or the root when absent.
        path: Option<String>,
        /// List every entry beneath, not just the direct children.
        recursive: bool,
    },
    /// A file's bytes in a changeset's snapshot.
    ReadFile {
        /// The changeset whose snapshot to read.
        changeset: ObjectId,
        /// The file's path, relative to the repository root.
        path: String,
    },
    /// The changes that turn one snapshot into another.
    Diff(DiffRequest),
    /// A page of first-parent history.
    Log(LogRequest),
    /// Whether one changeset is an ancestor of another.
    Ancestor(AncestorQuery),
    /// A lowest common ancestor of two changesets.
    MergeBase(MergeBaseQuery),
    /// Read a ref by name.
    GetRef(String),
    /// Move a ref by compare-and-swap.
    UpdateRef {
        /// The ref's name.
        name: String,
        /// The expected current value and the new target.
        update: UpdateRef,
    },
    /// Move a changeset's changes onto another changeset.
    Rebase(RebaseRequest),
    /// Undo a changeset on top of another.
    Revert(RevertRequest),
    /// Move a ref to include a changeset by compare-and-swap, with retries.
    Land(LandRequest),
}

/// The server's answer to a [`Request`]. Each variant is the reply of the
/// operation with the matching name; an engine failure is [`Response::Error`],
/// carrying the same stable code and message the HTTP API returns, so a caller
/// reconstructs the error without a second taxonomy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    /// The changeset a create produced.
    Changeset(ChangesetInfo),
    /// A read changeset.
    ChangesetRecord(ChangesetRecord),
    /// The entry at a path, or `null`.
    Stat(StatResult),
    /// The entries under a path.
    Tree(TreeListing),
    /// A file's bytes, or `None` when the path names nothing or a directory.
    File(Option<Vec<u8>>),
    /// The changes between two snapshots.
    Diff(DiffResult),
    /// A page of history.
    Log(LogPage),
    /// Whether one changeset is an ancestor of another.
    Ancestor(AncestorResult),
    /// A lowest common ancestor.
    MergeBase(MergeBaseResult),
    /// A ref's value.
    Ref(RefValue),
    /// The changeset a rebase or revert produced.
    Rebase(RebaseResult),
    /// The result of a land.
    Land(LandResult),
    /// The operation failed; the body carries its stable code and message.
    Error(ErrorBody),
}

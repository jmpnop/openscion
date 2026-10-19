//! The landing payload types: rebase, revert, and land requests and results.

use serde::{Deserialize, Serialize};

use crate::ObjectId;

/// A request to rebase a changeset onto another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RebaseRequest {
    /// The changeset whose changes move.
    pub changeset: ObjectId,
    /// The changeset to rebase onto; the result's first parent.
    pub onto: ObjectId,
}

impl RebaseRequest {
    /// Builds a rebase request.
    #[must_use]
    pub fn new(changeset: ObjectId, onto: ObjectId) -> Self {
        Self { changeset, onto }
    }
}

/// A request to revert a changeset onto another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RevertRequest {
    /// The changeset to undo.
    pub changeset: ObjectId,
    /// The changeset to apply the undo onto; the result's first parent.
    pub onto: ObjectId,
    /// Who the revert is attributed to.
    pub author: String,
    /// The revert's message.
    pub message: String,
}

impl RevertRequest {
    /// Builds a revert request.
    #[must_use]
    pub fn new(
        changeset: ObjectId,
        onto: ObjectId,
        author: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            changeset,
            onto,
            author: author.into(),
            message: message.into(),
        }
    }
}

/// What a rebase or a revert produces: the changeset that now carries the
/// change onto its new base. It equals the request's `onto` when the change
/// was already present there and nothing new was created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RebaseResult {
    /// The resulting changeset.
    pub changeset: ObjectId,
}

impl RebaseResult {
    /// Builds a rebase or revert result.
    #[must_use]
    pub fn new(changeset: ObjectId) -> Self {
        Self { changeset }
    }
}

/// A request to land a changeset on a ref by compare-and-swap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LandRequest {
    /// The changeset to land.
    pub changeset: ObjectId,
    /// The ref to move.
    pub ref_name: String,
    /// How many times to rebase-and-retry before giving up to contention.
    pub max_attempts: u32,
}

impl LandRequest {
    /// Builds a land request.
    #[must_use]
    pub fn new(changeset: ObjectId, ref_name: impl Into<String>, max_attempts: u32) -> Self {
        Self {
            changeset,
            ref_name: ref_name.into(),
            max_attempts,
        }
    }
}

/// What a land produces: the changeset the ref now points at and the ref's new
/// version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LandResult {
    /// The landed changeset.
    pub changeset: ObjectId,
    /// The ref's version after the move.
    pub version: u64,
}

impl LandResult {
    /// Builds a land result.
    #[must_use]
    pub fn new(changeset: ObjectId, version: u64) -> Self {
        Self { changeset, version }
    }
}

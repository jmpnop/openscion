//! Changesets: the unit of history.
//!
//! A changeset names its parents and lists file-level [`Change`]s relative to
//! its first parent (`docs/specs/openscion-data-model.md#changesets`). A client
//! sends a [`CreateChangeset`]; the server validates it, stamps `created_at`,
//! derives the root tree, appends it to the log, and answers with a
//! [`ChangesetInfo`]. Reading one back returns a [`ChangesetRecord`].

mod types;

pub use types::{
    Change, Changeset, ChangesetInfo, ChangesetRecord, ChangesetSummary, CreateChangeset,
};

#[cfg(test)]
mod test;

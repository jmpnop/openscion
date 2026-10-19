//! The metadata store port: the changeset log, the refs, and derived data.
//!
//! A [`MetadataStore`] is the authoritative mutable state the engine keeps
//! beside the immutable [`ObjectStore`](crate::ObjectStore): an append-only
//! changeset log with monotonically increasing sequence numbers, point lookups
//! by id, and an atomic compare-and-swap on a single ref record. Nothing needs
//! a cross-record transaction, which is what lets the store shard
//! (`docs/specs/openscion-architecture.md`).
//!
//! [`MemoryMetadataStore`] keeps the log and refs in memory;
//! [`SqliteMetadataStore`] (behind the `sqlite` feature) keeps them in a SQLite
//! database. The engine holds one behind a `Box<dyn MetadataStore>`, so it
//! never names a backend.
//!
//! [`SqliteMetadataStore`]: crate::SqliteMetadataStore
//!
//! The read methods come in widths so a caller loads only what it needs:
//! [`MetadataStore::info`] is the summary (id, sequence number, root tree),
//! [`MetadataStore::ancestry`] is the generation and parents an ancestry walk
//! needs, [`MetadataStore::first_parent_skips`] is the skip list a first-parent
//! ancestry walk jumps along, and [`MetadataStore::record`] is the whole
//! changeset.

mod memory;
#[cfg(feature = "sqlite")]
mod sqlite;
mod types;

use openscion_proto::{ChangesetInfo, ObjectId, RefValue};

use crate::Result;

pub use memory::MemoryMetadataStore;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteMetadataStore;
pub use types::{Ancestry, NewChangeset, StoredChangeset};

/// The changeset log, the refs, and (later) derived-data values.
pub trait MetadataStore: Send + Sync + std::fmt::Debug {
    /// Appends a validated changeset, assigning the next log sequence number,
    /// and returns its summary. The caller has already deduplicated by id.
    ///
    /// # Errors
    ///
    /// Returns a backing-store error when the changeset cannot be persisted.
    fn append(&mut self, new: NewChangeset) -> Result<ChangesetInfo>;

    /// The summary of the changeset with this id, or `None` when none has it.
    ///
    /// # Errors
    ///
    /// Returns a backing-store error when the lookup fails.
    fn info(&self, id: ObjectId) -> Result<Option<ChangesetInfo>>;

    /// The generation and parents of the changeset with this id, or `None`.
    ///
    /// # Errors
    ///
    /// Returns a backing-store error when the lookup fails.
    fn ancestry(&self, id: ObjectId) -> Result<Option<Ancestry>>;

    /// The first-parent skip list of the changeset with this id: index `k` is
    /// the first-parent ancestor `2^k` changesets back, so first-parent
    /// ancestry runs in O(log n). Empty for a root. `None` when none has it.
    ///
    /// # Errors
    ///
    /// Returns a backing-store error when the lookup fails.
    fn first_parent_skips(&self, id: ObjectId) -> Result<Option<Vec<ObjectId>>>;

    /// The whole changeset with this id, or `None` when none has it.
    ///
    /// # Errors
    ///
    /// Returns a backing-store error when the lookup fails.
    fn record(&self, id: ObjectId) -> Result<Option<StoredChangeset>>;

    /// The number of changesets in the log.
    ///
    /// # Errors
    ///
    /// Returns a backing-store error when the count cannot be read.
    fn log_len(&self) -> Result<u64>;

    /// A ref's current value, or `None` when the ref does not exist.
    ///
    /// # Errors
    ///
    /// Returns a backing-store error when the lookup fails.
    fn get_ref(&self, name: &str) -> Result<Option<RefValue>>;

    /// Moves `name` from `expected` to `new` atomically, bumping its version.
    /// A ref that does not exist has an expected value of `None`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RefConflict`](crate::Error::RefConflict) when the ref
    /// does not currently point at `expected` (nothing changes), or a
    /// backing-store error when the swap cannot be persisted.
    fn update_ref(
        &mut self,
        name: &str,
        expected: Option<ObjectId>,
        new: ObjectId,
    ) -> Result<RefValue>;
}

#[cfg(test)]
mod test;

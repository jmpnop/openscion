//! The object store port: immutable, content-addressed blobs and trees.
//!
//! An [`ObjectStore`] holds objects keyed by their [`ObjectId`]. Objects are
//! immutable, so a put of an object already present is a no-op and the id is
//! the content's address — the store needs only put-if-absent and get-by-id
//! (`docs/specs/openscion-architecture.md`). [`MemoryObjectStore`] keeps every
//! object in memory; [`FilesystemObjectStore`] keeps each object in a file
//! under a directory tree. The engine holds one behind a `Box<dyn ObjectStore>`,
//! so it never names a backend.

mod filesystem;
mod memory;

use std::sync::Arc;

use openscion_proto::{ObjectId, ObjectKind};

use crate::Result;

pub use filesystem::FilesystemObjectStore;
pub use memory::MemoryObjectStore;

/// A store of immutable, content-addressed objects.
///
/// Implementations must be safe to share across threads and to print, matching
/// the rest of the engine's ports.
pub trait ObjectStore: Send + Sync + std::fmt::Debug {
    /// Hashes `payload` as an object of `kind`, stores it if absent, and
    /// returns its id. Storing an object already present is a no-op.
    ///
    /// # Errors
    ///
    /// Returns an error when the backing store cannot persist the object.
    fn put(&mut self, kind: ObjectKind, payload: &[u8]) -> Result<ObjectId>;

    /// Stores `payload` under `expected` after checking that it hashes to it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::CorruptObject`](crate::Error::CorruptObject) when the
    /// payload hashes to a different id (nothing is stored), or a backing-store
    /// error when it cannot persist.
    fn put_verified(&mut self, kind: ObjectKind, expected: ObjectId, payload: &[u8]) -> Result<()>;

    /// The object's bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`](crate::Error::NotFound) when no object has
    /// this id, or a backing-store error when it cannot be read.
    fn get(&self, id: ObjectId) -> Result<Arc<[u8]>>;

    /// Whether an object with this id is stored.
    ///
    /// # Errors
    ///
    /// Returns a backing-store error when the presence of the object cannot be
    /// determined.
    fn contains(&self, id: ObjectId) -> Result<bool>;
}

#[cfg(test)]
mod test;

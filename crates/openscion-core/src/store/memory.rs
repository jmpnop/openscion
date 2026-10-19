//! The in-memory object store.

use std::collections::HashMap;
use std::sync::Arc;

use openscion_proto::{ObjectId, ObjectKind};

use crate::{Error, Result, object_id};

use super::ObjectStore;

/// Every object the server holds, in memory.
#[derive(Debug, Default)]
pub struct MemoryObjectStore {
    objects: HashMap<ObjectId, Arc<[u8]>>,
    bytes: u64,
}

impl MemoryObjectStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn insert(&mut self, id: ObjectId, payload: &[u8]) {
        self.objects.entry(id).or_insert_with(|| {
            self.bytes += payload.len() as u64;
            Arc::from(payload)
        });
    }

    /// The number of distinct objects stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Whether the store holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// The total size of every stored payload, in bytes.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.bytes
    }
}

impl ObjectStore for MemoryObjectStore {
    fn put(&mut self, kind: ObjectKind, payload: &[u8]) -> Result<ObjectId> {
        let id = object_id(kind, payload);
        self.insert(id, payload);
        Ok(id)
    }

    fn put_verified(&mut self, kind: ObjectKind, expected: ObjectId, payload: &[u8]) -> Result<()> {
        let actual = object_id(kind, payload);
        if actual != expected {
            return Err(Error::CorruptObject { expected, actual });
        }
        self.insert(actual, payload);
        Ok(())
    }

    fn get(&self, id: ObjectId) -> Result<Arc<[u8]>> {
        self.objects.get(&id).cloned().ok_or(Error::NotFound(id))
    }

    fn contains(&self, id: ObjectId) -> Result<bool> {
        Ok(self.objects.contains_key(&id))
    }
}

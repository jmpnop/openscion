//! The cache's in-memory object backing: object bytes keyed by id in process
//! memory, the counterpart to the daemon's on-disk
//! [`ObjectCache`](openscion_tier::ObjectCache).
//!
//! [`MemoryBacking`] implements the tier's [`ObjectBacking`] port, so the same
//! read-through [`Store`](openscion_tier::Store) that fronts a disk cache on a
//! node fronts an in-memory cache in a region. It holds no durable state: it
//! starts empty and a restart loses only warmth, which is why the regional
//! cache needs no snapshot.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use openscion_core::ObjectId;
use openscion_tier::{ObjectBacking, Result};

/// An in-memory [`ObjectBacking`]: object bytes keyed by id, behind a
/// read-write lock.
///
/// A regional cache's load is overwhelmingly hits, and a hit only reads the
/// map, so reads take the lock in shared mode and run concurrently; only
/// admissions and evictions, which mutate the map, take it exclusively. Like
/// every backing it neither verifies nor interprets the bytes — the
/// [`Store`](openscion_tier::Store) verifies an object against its id before
/// admitting it here — and it supports [`remove`](ObjectBacking::remove), so the
/// store can evict within its byte budget.
#[derive(Debug, Default)]
pub struct MemoryBacking {
    objects: RwLock<HashMap<ObjectId, Arc<[u8]>>>,
}

impl MemoryBacking {
    /// An empty backing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the map, recovering from a poisoned lock: it holds only
    /// content-addressed bytes, so a panic elsewhere cannot make it
    /// inconsistent, and refusing to serve would be worse.
    fn shared(&self) -> RwLockReadGuard<'_, HashMap<ObjectId, Arc<[u8]>>> {
        self.objects.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes the map exclusively, recovering from a poisoned lock for the same
    /// reason as [`shared`](Self::shared).
    fn exclusive(&self) -> RwLockWriteGuard<'_, HashMap<ObjectId, Arc<[u8]>>> {
        self.objects.write().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ObjectBacking for MemoryBacking {
    fn contains(&self, id: ObjectId) -> Result<bool> {
        Ok(self.shared().contains_key(&id))
    }

    fn read(&self, id: ObjectId) -> Result<Option<Arc<[u8]>>> {
        Ok(self.shared().get(&id).cloned())
    }

    fn write(&self, id: ObjectId, payload: &[u8]) -> Result<()> {
        // Keep the first bytes admitted under an id: a repeated write of the
        // same content-addressed object is a no-op, never a reallocation.
        self.exclusive()
            .entry(id)
            .or_insert_with(|| Arc::from(payload));
        Ok(())
    }

    fn remove(&self, id: ObjectId) -> Result<()> {
        self.exclusive().remove(&id);
        Ok(())
    }
}

#[cfg(test)]
mod test;

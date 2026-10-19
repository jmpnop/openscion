//! The node-local object store: a bounded, read-through cache of immutable
//! objects shared by every job on one machine.
//!
//! [`NodeStore`] answers [`get`](NodeStore::get) by serving a local copy or, on
//! a miss, fetching the object from the tier above (an [`ObjectSource`]),
//! verifying it hashes to the requested id, caching it within a byte budget, and
//! returning it. [`snapshot`](NodeStore::snapshot) and [`warm`](NodeStore::warm)
//! let a fresh daemon start warm instead of cold.
//!
//! The byte budget, the least-recently-used eviction, recency tracking, the
//! kind-recovery that verifies fetched bytes, and the lock discipline are all
//! hidden: a caller — and every higher tier that treats daemon, cache, and
//! server interchangeably — sees only `get`, whose shape matches
//! [`ObjectStore::get`](openscion_core::ObjectStore). The store never holds its
//! index lock across an upstream fetch or a cache write, so a slow fetch for one
//! id does not block a `get` of another.

mod types;

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use openscion_core::{ObjectId, ObjectKind, object_id};

use crate::cache::ObjectCache;
use crate::{Error, Result};

use types::Lru;
pub use types::Warmed;

/// The tier above this node — the regional cache, or the server when there is
/// no cache. The node store asks it for objects it does not hold locally.
///
/// The store does not know whether the source is a network client, another
/// store, or a test fake; only that an id either resolves to bytes or exists
/// nowhere above.
pub trait ObjectSource: Send + Sync {
    /// The bytes of object `id` from the tier above, or `None` when no tier
    /// above holds it.
    ///
    /// # Errors
    ///
    /// Returns an error only when the upstream could not be reached or answered
    /// improperly — never for a plain absence, which is `Ok(None)`.
    fn fetch(&self, id: ObjectId) -> Result<Option<Arc<[u8]>>>;
}

/// A node-wide, bounded, read-through cache of immutable objects.
#[derive(Debug)]
pub struct NodeStore<S: ObjectSource> {
    cache: ObjectCache,
    upstream: S,
    index: Mutex<Lru>,
}

impl<S: ObjectSource> NodeStore<S> {
    /// Opens the node store rooted at `dir`, fronting `upstream`, holding at
    /// most `budget_bytes` of objects.
    ///
    /// Objects left on disk by a previous run are served when asked for and come
    /// under budget management the first time they are touched; recency begins
    /// empty and is re-established by [`warm`](Self::warm) or by traffic.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when `dir` cannot be created or read.
    pub fn open(dir: &Path, upstream: S, budget_bytes: u64) -> Result<Self> {
        Ok(Self {
            cache: ObjectCache::open(dir)?,
            upstream,
            index: Mutex::new(Lru::new(budget_bytes)),
        })
    }

    /// The bytes of object `id`: served locally if present, otherwise fetched
    /// from the tier above, verified against `id`, cached, and returned.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] when no tier holds `id`, [`Error::Corrupt`]
    /// when the upstream's bytes do not hash to `id` under any object kind
    /// (those bytes are not cached), and [`Error::Io`] when the cache cannot be
    /// read or written.
    pub fn get(&self, id: ObjectId) -> Result<Arc<[u8]>> {
        // A hit: mark it recently used, releasing the lock before touching the
        // disk (`present` is a plain bool; the guard is dropped at the `let`).
        // If a racing eviction removed it between the two, fall through to a
        // fetch rather than return nothing.
        let present = self.lock().touch(id);
        if present && let Some(bytes) = self.cache.read(id)? {
            return Ok(bytes);
        }
        let bytes = self.upstream.fetch(id)?.ok_or(Error::NotFound(id))?;
        if verify(id, &bytes).is_none() {
            return Err(Error::Corrupt { id });
        }
        self.store(id, &bytes)?;
        Ok(bytes)
    }

    /// The ids the store holds, most-recently-used first — the warm set to
    /// persist so another daemon can [`warm`](Self::warm) from it.
    #[must_use]
    pub fn snapshot(&self) -> Vec<ObjectId> {
        self.lock().ids_mru_first()
    }

    /// Fetches `ids` from the tier above into the store, so a fresh node does
    /// not start cold, and reports how many were fetched and how many no tier
    /// held.
    ///
    /// `ids` is taken in [`snapshot`](Self::snapshot) order — most-recently-used
    /// first — and fetched least-recent first, so a set larger than the budget
    /// leaves the most-recently-used resident. Ids already held are skipped.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Corrupt`] for bytes that fail verification, and the
    /// upstream or [`Error::Io`] error otherwise; an id no tier holds is counted
    /// in [`Warmed::missing`], not an error.
    pub fn warm(&self, ids: &[ObjectId]) -> Result<Warmed> {
        let mut fetched = 0;
        let mut missing = 0;
        for &id in ids.iter().rev() {
            if self.lock().contains(id) {
                continue;
            }
            match self.upstream.fetch(id)? {
                Some(bytes) => {
                    if verify(id, &bytes).is_none() {
                        return Err(Error::Corrupt { id });
                    }
                    self.store(id, &bytes)?;
                    fetched += 1;
                }
                None => missing += 1,
            }
        }
        Ok(Warmed { fetched, missing })
    }

    /// Writes `bytes` into the cache and records `id` as most-recently-used,
    /// evicting least-recently-used objects to stay within budget. The index
    /// lock is held only to update the index, never across the cache writes.
    fn store(&self, id: ObjectId, bytes: &[u8]) -> Result<()> {
        self.cache.write(id, bytes)?;
        let evicted = self.lock().admit(id, bytes.len() as u64);
        for id in evicted {
            self.cache.remove(id)?;
        }
        Ok(())
    }

    /// Locks the index, recovering from a poisoned lock: the index holds only
    /// recency bookkeeping, so a panic in another thread cannot leave it
    /// meaningfully inconsistent, and refusing to serve would be worse.
    fn lock(&self) -> MutexGuard<'_, Lru> {
        self.index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The object kind whose hash of `bytes` is `id`, or `None` when none matches —
/// which both authenticates the bytes against the content address and recovers
/// the kind the wire does not carry.
fn verify(id: ObjectId, bytes: &[u8]) -> Option<ObjectKind> {
    [ObjectKind::Blob, ObjectKind::Tree, ObjectKind::Changeset]
        .into_iter()
        .find(|&kind| object_id(kind, bytes) == id)
}

#[cfg(test)]
mod test;

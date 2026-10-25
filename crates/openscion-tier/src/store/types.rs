//! The read-through store's result type and its least-recently-used budget index.

use std::collections::{BTreeMap, HashMap};

use openscion_core::ObjectId;

/// The outcome of a [`warm`](super::Store::warm): how many ids were fetched
/// into the store, and how many no tier above held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Warmed {
    /// Objects fetched from the tier above and cached.
    pub fetched: usize,
    /// Ids no tier above held.
    pub missing: usize,
}

/// One tracked object: its recency sequence and byte size.
#[derive(Debug)]
struct Entry {
    seq: u64,
    size: u64,
}

/// A least-recently-used index over the cached objects, holding the running
/// byte total against a budget.
///
/// Recency is a monotonic sequence: the most-recently-touched id has the
/// largest sequence. `by_seq` orders ids least- to most-recently-used, so the
/// least-recently-used is the first entry — an eviction candidate — in
/// `O(log n)`.
#[derive(Debug)]
pub(crate) struct Lru {
    budget: u64,
    total: u64,
    next_seq: u64,
    by_seq: BTreeMap<u64, ObjectId>,
    entries: HashMap<ObjectId, Entry>,
}

impl Lru {
    /// An empty index holding at most `budget` bytes.
    pub(crate) fn new(budget: u64) -> Self {
        Self {
            budget,
            total: 0,
            next_seq: 0,
            by_seq: BTreeMap::new(),
            entries: HashMap::new(),
        }
    }

    /// Whether `id` is tracked.
    pub(crate) fn contains(&self, id: ObjectId) -> bool {
        self.entries.contains_key(&id)
    }

    /// Marks `id` most-recently-used if it is tracked, returning whether it was.
    pub(crate) fn touch(&mut self, id: ObjectId) -> bool {
        let Some(entry) = self.entries.get_mut(&id) else {
            return false;
        };
        self.by_seq.remove(&entry.seq);
        entry.seq = self.next_seq;
        self.next_seq += 1;
        self.by_seq.insert(entry.seq, id);
        true
    }

    /// Admits `id` of `size` bytes as most-recently-used, replacing any prior
    /// entry for it, then evicts least-recently-used ids until the total is
    /// within budget — keeping at least the just-admitted id, so an object
    /// larger than the whole budget is still held while it is the only one.
    /// Returns the evicted ids.
    pub(crate) fn admit(&mut self, id: ObjectId, size: u64) -> Vec<ObjectId> {
        if let Some(previous) = self.entries.remove(&id) {
            self.by_seq.remove(&previous.seq);
            self.total -= previous.size;
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.by_seq.insert(seq, id);
        self.entries.insert(id, Entry { seq, size });
        self.total += size;

        let mut evicted = Vec::new();
        while self.total > self.budget && self.entries.len() > 1 {
            let Some((&lru_seq, &lru_id)) = self.by_seq.iter().next() else {
                break;
            };
            self.by_seq.remove(&lru_seq);
            if let Some(entry) = self.entries.remove(&lru_id) {
                self.total -= entry.size;
            }
            evicted.push(lru_id);
        }
        evicted
    }

    /// The tracked ids, most-recently-used first.
    pub(crate) fn ids_mru_first(&self) -> Vec<ObjectId> {
        self.by_seq.values().rev().copied().collect()
    }
}

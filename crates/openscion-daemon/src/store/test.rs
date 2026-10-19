//! Unit tests for the read-through node store.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};

use openscion_core::{ObjectId, ObjectKind, object_id};

use super::{NodeStore, ObjectSource, verify};
use crate::test_support::{blob_id, scratch};

/// An in-memory tier-above that counts how many times it was asked to fetch.
#[derive(Clone, Default)]
struct Source {
    inner: Arc<SourceInner>,
}

#[derive(Default)]
struct SourceInner {
    objects: Mutex<HashMap<ObjectId, Arc<[u8]>>>,
    fetches: AtomicUsize,
}

impl Source {
    fn new() -> Self {
        Self::default()
    }

    /// Adds a blob and returns its id.
    fn add(&self, bytes: &[u8]) -> ObjectId {
        let id = blob_id(bytes);
        self.inner
            .objects
            .lock()
            .unwrap()
            .insert(id, Arc::from(bytes));
        id
    }

    /// Serves `served` under the id that `claimed` hashes to, so the bytes do
    /// not match the id — a corrupt upstream.
    fn add_corrupt(&self, claimed: &[u8], served: &[u8]) -> ObjectId {
        let id = blob_id(claimed);
        self.inner
            .objects
            .lock()
            .unwrap()
            .insert(id, Arc::from(served));
        id
    }

    fn fetches(&self) -> usize {
        self.inner.fetches.load(Ordering::SeqCst)
    }
}

impl ObjectSource for Source {
    fn fetch(&self, id: ObjectId) -> crate::Result<Option<Arc<[u8]>>> {
        self.inner.fetches.fetch_add(1, Ordering::SeqCst);
        Ok(self.inner.objects.lock().unwrap().get(&id).cloned())
    }
}

/// Opens a node store over `source` with the given budget.
fn store(name: &str, source: Source, budget: u64) -> NodeStore<Source> {
    NodeStore::open(&scratch(name), source, budget).unwrap()
}

#[test]
fn a_miss_fetches_once_then_serves_locally() {
    let source = Source::new();
    let id = source.add(b"hello");
    let store = store("hit", source.clone(), 1 << 20);

    assert_eq!(&*store.get(id).unwrap(), b"hello");
    assert_eq!(source.fetches(), 1);
    // A second get is served locally: no further fetch.
    assert_eq!(&*store.get(id).unwrap(), b"hello");
    assert_eq!(source.fetches(), 1);
}

#[test]
fn an_object_no_tier_holds_is_not_found() {
    let source = Source::new();
    let store = store("missing", source.clone(), 1 << 20);
    let absent = blob_id(b"nowhere");
    assert!(matches!(
        store.get(absent),
        Err(crate::Error::NotFound(id)) if id == absent
    ));
}

#[test]
fn corrupt_upstream_bytes_are_rejected_and_not_cached() {
    let source = Source::new();
    let id = source.add_corrupt(b"claimed", b"tampered");
    let store = store("corrupt", source.clone(), 1 << 20);

    assert!(matches!(store.get(id), Err(crate::Error::Corrupt { .. })));
    // Not cached: a second get fetches again rather than serving bad bytes.
    assert!(matches!(store.get(id), Err(crate::Error::Corrupt { .. })));
    assert_eq!(source.fetches(), 2);
    assert!(store.snapshot().is_empty());
}

#[test]
fn eviction_drops_the_least_recently_used() {
    let source = Source::new();
    let a = source.add(b"aaaa");
    let b = source.add(b"bbbb");
    let c = source.add(b"cccc");
    // Budget holds two four-byte objects.
    let store = store("evict", source.clone(), 8);

    store.get(a).unwrap();
    store.get(b).unwrap();
    store.get(c).unwrap(); // admits c, evicts a (least recently used)

    // c is most-recently-used, then b; a is gone.
    assert_eq!(store.snapshot(), vec![c, b]);
    // b is still a local hit (no fetch); check it before touching a, since the
    // budget holds only two and re-fetching a would in turn evict b.
    let before = source.fetches();
    store.get(b).unwrap();
    assert_eq!(source.fetches(), before, "b is still cached");
    // a was evicted, so it re-fetches.
    let before = source.fetches();
    store.get(a).unwrap();
    assert_eq!(source.fetches(), before + 1, "a was evicted and re-fetched");
}

#[test]
fn an_object_larger_than_the_budget_is_still_served() {
    let source = Source::new();
    let big = source.add(b"a much larger object than the budget");
    let store = store("over-budget", source.clone(), 4);
    assert_eq!(
        &*store.get(big).unwrap(),
        b"a much larger object than the budget"
    );
    assert_eq!(store.snapshot(), vec![big]);
}

#[test]
fn snapshot_lists_most_recently_used_first() {
    let source = Source::new();
    let a = source.add(b"aa");
    let b = source.add(b"bb");
    let store = store("snapshot", source.clone(), 1 << 20);
    store.get(a).unwrap();
    store.get(b).unwrap();
    store.get(a).unwrap(); // a is now most-recently-used
    assert_eq!(store.snapshot(), vec![a, b]);
}

#[test]
fn warm_fetches_a_snapshot_and_keeps_the_most_recent_within_budget() {
    let source = Source::new();
    let a = source.add(b"aaaa");
    let b = source.add(b"bbbb");
    let c = source.add(b"cccc");
    // Budget holds two of the three four-byte objects.
    let store = store("warm", source.clone(), 8);

    // `ids` in snapshot order (most-recently-used first).
    let warmed = store.warm(&[a, b, c]).unwrap();
    assert_eq!(warmed.fetched, 3);
    assert_eq!(warmed.missing, 0);
    // The most-recently-used two survive; the least-recently-used (c) is evicted.
    assert_eq!(store.snapshot(), vec![a, b]);
}

#[test]
fn warm_counts_ids_no_tier_holds_and_skips_those_already_held() {
    let source = Source::new();
    let a = source.add(b"held");
    let absent = blob_id(b"absent");
    let store = store("warm-missing", source.clone(), 1 << 20);

    store.get(a).unwrap(); // a is already held
    let fetched_before = source.fetches();
    let warmed = store.warm(&[a, absent]).unwrap();
    assert_eq!(warmed.fetched, 0, "a is skipped");
    assert_eq!(warmed.missing, 1, "absent is counted missing");
    // Only `absent` was fetched (a was skipped), so one more fetch than before.
    assert_eq!(source.fetches(), fetched_before + 1);
}

#[test]
fn warm_rejects_corrupt_bytes() {
    let source = Source::new();
    let id = source.add_corrupt(b"claimed", b"tampered");
    let store = store("warm-corrupt", source.clone(), 1 << 20);
    assert!(matches!(
        store.warm(&[id]),
        Err(crate::Error::Corrupt { .. })
    ));
}

#[test]
fn verify_recovers_each_kind_and_rejects_corruption() {
    for kind in [ObjectKind::Blob, ObjectKind::Tree, ObjectKind::Changeset] {
        let bytes = b"some object body";
        let id = object_id(kind, bytes);
        assert_eq!(verify(id, bytes), Some(kind));
    }
    // Bytes that match no kind for the id are rejected.
    let id = object_id(ObjectKind::Blob, b"original");
    assert_eq!(verify(id, b"different"), None);
}

/// A tier-above whose fetch of one designated id blocks until released, so a
/// test can prove the store does not hold its index lock across a fetch.
struct BlockingSource {
    objects: HashMap<ObjectId, Arc<[u8]>>,
    block_id: ObjectId,
    entered: Arc<Barrier>,
    release: Arc<Barrier>,
}

impl ObjectSource for BlockingSource {
    fn fetch(&self, id: ObjectId) -> crate::Result<Option<Arc<[u8]>>> {
        if id == self.block_id {
            self.entered.wait();
            self.release.wait();
        }
        Ok(self.objects.get(&id).cloned())
    }
}

#[test]
fn a_local_get_is_not_blocked_by_an_in_flight_fetch() {
    let a = blob_id(b"aaa");
    let b = blob_id(b"bbb");
    let mut objects = HashMap::new();
    objects.insert(a, Arc::from(&b"aaa"[..]));
    objects.insert(b, Arc::from(&b"bbb"[..]));
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let source = BlockingSource {
        objects,
        block_id: b,
        entered: entered.clone(),
        release: release.clone(),
    };
    let store = Arc::new(NodeStore::open(&scratch("concurrent"), source, 1 << 20).unwrap());

    // Cache `a` (its fetch does not block).
    assert_eq!(&*store.get(a).unwrap(), b"aaa");

    // Thread A gets `b`, whose fetch blocks inside the source.
    let getter = Arc::clone(&store);
    let handle = std::thread::spawn(move || getter.get(b));

    // Once A is inside the blocked fetch (holding no index lock), a local get of
    // `a` must complete rather than deadlock.
    entered.wait();
    assert_eq!(&*store.get(a).unwrap(), b"aaa");

    // Release A and confirm it got `b`.
    release.wait();
    assert_eq!(&*handle.join().unwrap().unwrap(), b"bbb");
}

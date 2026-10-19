# Node-local object store and snapshot boot

- **Status:** Accepted
- **Owner:** OpenScion daemon
- **Specifications it builds on:**
  [architecture](openscion-architecture.md) (phase 5, the daemon; the uniform
  object RPCs and strict tiering), [data model](openscion-data-model.md)
  (object ids and BLAKE3 hashing).
- **Design record:** the POSD Design-It-Twice analysis (two designs, depth test,
  18-principle evaluation) chose the deep read-through module below.

## Problem

Every job on a machine — an agent, a CI step, a `log`/`diff`/`blame` call —
needs the same immutable objects: the blobs and trees of the snapshots it reads.
Fetching each from the regional cache or the server, per job, wastes the network
and starts every fresh node cold. The architecture puts one daemon per machine
holding a node-wide object store shared by all of them, answering the same
object RPCs as the cache and the server, so a job talks to whatever is nearest
and never has to know the topology.

That store cannot be a full copy of history — no component holds one. It is a
*bounded* cache: it keeps the recently used objects and lets the rest fall back
to the tier above. Because objects are immutable and content-addressed, a cache
needs no invalidation and anything it drops is re-fetchable. The open questions
are how it decides what to keep, how it stays correct while many jobs hit it at
once, and how a freshly started daemon avoids serving nothing until traffic
slowly warms it.

## Goals and non-goals

### Goals

- A node-local store that answers `get(id)` by serving a local copy, or, on a
  miss, fetching the object from the tier above, verifying it, keeping it, and
  returning it — the read-through behaviour, with the same `get` shape as
  `ObjectStore::get` so tiers are interchangeable.
- A byte budget: the store holds at most a configured number of bytes, evicting
  least-recently-used objects to stay within it.
- Snapshot boot: `snapshot()` reports the warm set (ids, most-recently-used
  first) to persist, and `warm(ids)` fetches a saved set from the tier above so
  a fresh daemon starts warm.
- Content-addressed integrity end to end: bytes fetched from upstream are
  verified to hash to the requested id before they are cached or returned.
- Correctness under concurrent callers, and no caller blocked on another
  caller's slow upstream fetch.

### Non-goals

- The transport. This sub-chunk is an in-process API; `ObjectSource` abstracts
  the tier above. The framed `openscion-proto` client that implements it over a
  socket is a later sub-chunk (see [ADR 0004](../adr/0004-daemon-transport.md)).
- Forwarding mutable queries (refs, the log, derived-data lookups). Those are
  never cached; the daemon's RPC layer forwards them upstream, and that layer is
  a later sub-chunk.
- Pinning objects reachable from a ref against eviction, content-defined
  chunking of large blobs, and an `Unavailable` load-shedding response — all
  later phases.
- Scheduling the periodic snapshot write. The store exposes `snapshot()` and
  `warm()`; a periodic writer that persists the set to disk is an operational
  concern layered on top later.

## Why not `FilesystemObjectStore` directly

The engine's `ObjectStore` is an append-only, immutable store: its surface is
`get`, `put`, `put_verified`, and `contains` — no removal, no enumeration, no
sizes. A bounded cache must evict (remove) objects and must know their sizes to
hold a byte budget. Rather than add a `remove` the server and client never use
to the shared trait, the daemon owns a small **evictable object cache** keyed by
id, reusing the same content-addressed fanout layout and the atomic
temp+fsync+rename write discipline, and reusing the engine's `object_id` hashing
to verify fetched bytes. The cache is a private detail of the node store.

## Proposed behaviour

### The upstream

```rust
pub trait ObjectSource: Send + Sync {
    fn fetch(&self, id: ObjectId) -> Result<Option<Arc<[u8]>>>;
}
```

`fetch` returns the object's bytes, or `None` when no tier above holds it — a
plain absence, not an error. The node store does not know whether the source is
a network client, another store, or a test fake.

### The node store

```rust
pub struct NodeStore<S: ObjectSource> { /* private */ }

impl<S: ObjectSource> NodeStore<S> {
    pub fn open(dir: &Path, upstream: S, budget_bytes: u64) -> Result<Self>;
    pub fn get(&self, id: ObjectId) -> Result<Arc<[u8]>>;
    pub fn snapshot(&self) -> Vec<ObjectId>;
    pub fn warm(&self, ids: &[ObjectId]) -> Result<Warmed>;
}

pub struct Warmed { pub fetched: usize, pub missing: usize }
```

- **`get(id)`** returns the bytes of `id`. A local hit records the id as
  most-recently-used and returns it. A miss fetches from the tier above,
  verifies the bytes hash to `id`, caches them (evicting least-recently-used
  objects to stay within the budget), records recency, and returns them. When no
  tier holds `id`, it is `Error::NotFound`. When the upstream's bytes do not hash
  to `id` under any object kind, it is `Error::Corrupt`.
- **`snapshot()`** returns the ids the store holds, most-recently-used first —
  the set to persist so another daemon can `warm` from it.
- **`warm(ids)`** fetches `ids` from the tier above into the store, skipping ids
  already held, and reports how many it fetched and how many no tier held. It
  takes `ids` in the order `snapshot()` produces — most-recently-used first —
  and fetches them so that, when the set exceeds the budget, the most-recently
  used survive and the least-recently used are the ones evicted.
- **`open(dir, upstream, budget_bytes)`** opens the cache rooted at `dir`.
  Objects left on disk by a previous run are served when asked for, and are
  taken under budget management the first time they are touched; the store's
  recency begins empty and is re-established by `warm` or by traffic.

### Recency, eviction, and the budget

Recency is least-recently-used: `get` and the insert in `warm` move an id to
most-recently-used; eviction removes from the least-recently-used end until the
total held is within `budget_bytes`, deleting those objects from the cache. A
single object larger than the whole budget is still served and held while it is
the only resident rather than evicted to nothing.

## Invariants and constraints

- **Served bytes hash to their id.** Every object `get` returns — local or
  freshly fetched — hashes to the requested id. Upstream bytes are verified
  (and their kind recovered) before being cached; bytes that match no kind are
  rejected as `Error::Corrupt` and not stored.
- **The budget bounds the managed set.** After any `get` or `warm`, the total
  size of budget-managed objects is at most `budget_bytes`, unless a single
  object exceeds the budget, in which case exactly that object is held.
- **Uniform `get`.** The store's `get` has the same signature and meaning as
  `ObjectStore::get` from a caller's view, so a daemon, a cache, and a server
  are interchangeable behind it.
- **Idempotent under concurrency.** Because objects are immutable and
  content-addressed, two concurrent `get`s of the same missing id both return
  the correct bytes; at worst a redundant upstream fetch happens. The store
  never holds its index lock across an upstream fetch or a disk write, so a slow
  fetch for one id does not block a `get` of another.
- **Eviction is safe.** An evicted object is re-fetchable from the tier above,
  so eviction never loses data; it only costs a later fetch.

## Acceptance criteria

- A `get` hit returns the local bytes and makes no upstream fetch; a `get` miss
  fetches once, caches, and a second `get` of the same id makes no further
  fetch.
- `get` of an id no tier holds is `Error::NotFound`; `get` whose upstream bytes
  do not hash to the id is `Error::Corrupt` and the object is not cached.
- Inserting past the budget evicts least-recently-used ids: the evicted ids miss
  locally afterward (re-fetching), the recently used ones stay; a single
  over-budget object is still served.
- `snapshot()` lists held ids most-recently-used first, and `warm(snapshot())`
  on a fresh store fetches them and reports the counts; when the set exceeds the
  budget, the most-recently-used ids are the ones left resident.
- Verification recovers each of the three object kinds (a blob, a tree, a
  changeset round-trips), and a corrupt-bytes control is rejected.
- A `get` of a locally held id completes while an upstream fetch for a different,
  missing id is in flight (the index lock is not held across the fetch).

## Open questions

- **Reconstructing recency on boot** from the on-disk cache (by last-access
  time) instead of starting empty, so a restarted daemon is warm without a
  snapshot. Deferred; `warm` covers the cold-start case for now.
- **Pinning the ref-reachable set** against eviction, so a working set a job is
  actively using is never dropped mid-operation. Deferred to when landing runs
  through the daemon.
- **Load shedding.** Returning `Unavailable` when the upstream concurrency bound
  is saturated, rather than queueing, belongs with the network-tier phase.

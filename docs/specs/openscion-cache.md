# Regional object cache (`openscion-cache`)

- **Status:** Accepted
- **Owner:** OpenScion maintainers

## Problem

The [architecture](openscion-architecture.md#the-tiers) places a regional
read-through cache between a machine's daemon and the authoritative server:

> **`openscion-cache`** — a regional read-through cache for content-addressed
> objects. Because an object can never change, a cache needs no invalidation and
> no long-lived state; it can be restarted empty at any time. It forwards every
> non-object RPC upstream unchanged.

The daemon already serves the same object RPCs from a bounded, read-through
[`Store`](openscion-daemon-node-store.md), and the
[`openscion-tier`](openscion-tier.md) crate now holds that store, the
`ObjectBacking` port it is generic over, the upstream-concurrency
[bound](openscion-daemon-upstream-bound.md), and the framed
[transport](openscion-daemon-transport.md). What is missing is the tier that
keeps its objects **in memory** rather than on disk, and the process that
assembles it.

This specification defines that tier: the in-memory `ObjectBacking`, and the
`openscion-cache` binary that fronts it.

## Goals

- An in-memory `ObjectBacking` — object bytes keyed by id, held in process
  memory, supporting the removal the store's eviction needs.
- A regional cache process that opens a read-through
  [`Store`](openscion-tier.md) over that backing, bounds its upstream
  concurrency, binds a listener, and serves the object RPCs over the framed
  transport — the same wire the daemon and server speak.
- A required upstream: a cache exists to cache *something*, so it always reads
  through to the tier above.
- Reuse of `openscion-tier` for every piece of shared machinery; the cache adds
  only the in-memory backing and its own process assembly and configuration.

## Non-goals

- **Durability or a warm set.** The cache holds no long-lived state and starts
  empty every time; it has no snapshot file and no data directory, unlike the
  daemon. This is deliberate, not an omission: an immutable-object cache that
  restarts cold is correct, only briefly slower.
- **Non-object RPCs.** Forwarding refs, the log, and derived-data queries
  upstream unchanged is a later piece, arriving with those RPCs themselves; the
  framed transport today carries only object reads.
- **New store, bound, or transport behaviour.** Those live in `openscion-tier`
  and are unchanged here.

## Proposed behavior

### The in-memory backing

A `MemoryBacking` implements `ObjectBacking`, holding object bytes in a map
keyed by id behind a read-write lock, so concurrent cache hits — the common
case — read in parallel while a write or eviction takes the lock exclusively:

```rust
let backing = MemoryBacking::new();
backing.write(id, b"object bytes")?;
assert!(backing.contains(id)?);
assert_eq!(backing.read(id)?.as_deref(), Some(&b"object bytes"[..]));
backing.remove(id)?;
assert!(!backing.contains(id)?);
```

It neither verifies nor interprets the bytes — the `Store` verifies an object
against its id before admitting it, exactly as for the on-disk backing. Its
operations never fail in practice (there is no I/O), but it returns the port's
`Result` so it is substitutable for any backing.

A read-write lock rather than a plain mutex is the deliberate choice: a regional
cache's load is overwhelmingly hits, and hits only read the map, so they should
not serialize against one another; only admissions and evictions, which mutate
it, take the lock exclusively. The lock is poison-tolerant — the map holds only
content-addressed bytes, so a panic elsewhere cannot make it inconsistent, and
refusing to serve would be worse than serving.

### The cache store

`CacheStore<S>` is `Store<S, MemoryBacking>`: the shared read-through store over
the in-memory backing, named for its role, just as the daemon names its disk
store `NodeStore`. It is built with the budget constructor the tier exposes for
a non-disk backing:

```rust
let store = Store::with_backing(upstream, budget_bytes, MemoryBacking::new());
```

### The process

`CacheConfig` reads flags and `OPENSCION_CACHE_*` environment variables:

| Flag | Env | Default | Meaning |
| --- | --- | --- | --- |
| `--listen` | `OPENSCION_CACHE_LISTEN` | `tcp:127.0.0.1:7989` | Endpoint to accept connections on. |
| `--upstream` | `OPENSCION_CACHE_UPSTREAM` | *(required)* | Tier above to read through. |
| `--budget-bytes` | `OPENSCION_CACHE_BUDGET_BYTES` | `1 GiB` | Resident byte budget; least-recently-used objects evict past it. |
| `--upstream-concurrency` | `OPENSCION_CACHE_UPSTREAM_CONCURRENCY` | `32` | Maximum concurrent fetches to the tier above; past it, fetches shed with `Unavailable`. |

The `--listen`/`--upstream` `Endpoint` is the same scheme-prefixed `tcp:<addr>`
/ `unix:<path>` value the daemon takes; it now lives in `openscion-tier` beside
the transport it addresses, shared by both binaries so one operator-facing form
spans every tier. A regional cache is reached over TCP in practice; it accepts
either transport because the tier's wire does.

`RegionalCache::bind` dials the upstream into a `SocketSource`, wraps it in a
`BoundedSource` at the configured limit, opens the `CacheStore` over a fresh
`MemoryBacking`, and binds the listener. `RegionalCache::serve` serves each
connection on a thread until a `SIGINT`/`SIGTERM` (Unix) and then exits cleanly
— with nothing to persist.

### Load shedding

The cache's fetches to the tier above pass through the same `BoundedSource` the
daemon uses: past the limit they shed with `Error::Unavailable`, which crosses
the wire as a distinct response, so a daemon reading through a saturated cache
re-sheds to its own caller rather than stampeding the cache. This is the strict,
bounded tiering the architecture requires, inherited whole from `openscion-tier`.

## Invariants and constraints

- The cache holds no durable state: no file on disk, no snapshot, nothing that
  must survive a restart. Restarting it loses only warmth.
- Every object the cache serves is verified against its id by the store before
  admission; the backing stores only what the store admits.
- The cache calls only the one tier above it (its `--upstream`) and bounds the
  concurrency of those calls; it never queues unboundedly and never skips a
  tier.
- The crate adds no third-party dependency beyond those the daemon already
  uses (`clap`, and `signal-hook` on Unix); all tier machinery is reused.
- `unsafe` is forbidden, as workspace-wide.

## Acceptance criteria

- `MemoryBacking` satisfies the `ObjectBacking` contract: round-trips bytes by
  id, reports membership, is idempotent on a repeated write and on removing an
  absent id, and frees the bytes on removal.
- A `CacheStore` over a served upstream fetches an object on a miss, serves it
  locally on the next read, and reports it in its snapshot — proven over the
  real framed wire.
- A shed from a bounded upstream reaches a client of the cache as
  `Unavailable`, distinct from a generic upstream error.
- The `openscion-cache` binary binds, prints its listen line, answers an object
  read over the wire by reading through a running upstream, and exits cleanly on
  `SIGTERM`; a malformed or taken endpoint is a non-zero exit.
- The daemon's public API and behaviour are unchanged by relocating `Endpoint`:
  its suite, including the binary integration test, stays green.
- Every source file keeps at least 90% line coverage; the four contract
  commands are green; `cargo deny` passes.

## Open questions

None. The design reuses `openscion-tier` wholesale and mirrors the accepted
daemon assembly, differing only where the architecture dictates — in memory,
with no durable state, and with a required upstream.

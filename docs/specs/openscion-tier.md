# OpenScion shared tier crate (`openscion-tier`)

- **Status:** Accepted
- **Phase:** 7 (network tiers), enabling chunk
- **Depends on:** the daemon node store, framed transport, and upstream bound
  (phase 5 + phase 7 chunk 1), which this chunk relocates.

## Why

The daemon, the regional cache, and the server all answer the **same object
RPCs with the same semantics** over the **same framed wire**
([ADR 0004](../adr/0004-daemon-transport.md): *"The daemon, cache, and server
speak the same frames … Transport … live in the daemon and (later) cache/server
transport layers"*). Today that machinery — the `ObjectSource` port, the
read-through object store, the `BoundedSource` upstream bound, and the framed
transport — lives inside `openscion-daemon`. The regional cache tier cannot be
built without duplicating it or depending on the node-daemon crate.

This chunk extracts the shared tier machinery into a new crate,
`openscion-tier`, that the daemon depends on now and the cache (next chunk) and
the server's framed endpoint (later) depend on in turn. It is a relocation and a
single generalization, not a behaviour change: every existing daemon test and
gate stays green.

## What

### A new crate, `openscion-tier` (`publish = false`)

It holds the tier layer and nothing daemon-specific:

- **`Error` / `Result`** — the tier error: `NotFound`, `Corrupt`, `Io`,
  `Protocol`, `Upstream`, `Transport`, `Unavailable`. (Relocated verbatim.)
- **`ObjectSource`** — the port for the tier above: `fetch(id) -> Result<Option<Arc<[u8]>>>`.
- **`ObjectBacking`** — a new port for a tier's local byte store, keyed by id:
  `contains`, `read`, `write`, `remove`. The generalization that lets one
  read-through store serve both an on-disk daemon and an in-memory cache.
- **`ObjectCache`** — the on-disk `ObjectBacking` (git-style fanout, atomic
  temp+fsync+rename, crash sweep). Relocated verbatim; now implements
  `ObjectBacking`.
- **`Store<S: ObjectSource, B: ObjectBacking>`** — the read-through store
  (relocated `NodeStore`, generalized over its backing): `get` (local hit, else
  verified read-through fetch, cached under an LRU byte budget), `warm`,
  `snapshot`. The LRU index, recency, verification, and lock discipline are
  unchanged.
  - `Store<S, ObjectCache>::open(dir, upstream, budget)` — the disk constructor.
  - `Store::with_backing(upstream, budget, backing)` — the generic constructor a
    non-disk backing (the cache's in-memory one, next chunk) uses.
- **`BoundedSource`** — the upstream-concurrency bound and `Unavailable` shed.
  Relocated verbatim.
- **transport** — the framed codec, `serve_connection` / `serve_tcp` /
  `serve_unix` (generalized to `&Store<S, B>`), `SocketSource`, and the response
  tags including `TAG_UNAVAILABLE`. Relocated verbatim.
- **`Warmed`** — the `warm` outcome. Relocated.

### `openscion-daemon` becomes a consumer

The daemon keeps only what is daemon-specific — `DaemonConfig` and its scheme
`Endpoint`, the snapshot id-file codec, the `Daemon` assembly (`Upstream`,
listener, bind/warm/serve/signal shutdown), and the binary — and depends on
`openscion-tier` for the rest. It re-exports the tier types it exposed before so
its public surface is unchanged:

```rust
pub use openscion_tier::{
    BoundedSource, Error, ObjectSource, Result, SocketSource, Warmed,
    serve_connection, serve_tcp, serve_unix, // + serve_unix on unix
};
pub type NodeStore<S> = openscion_tier::Store<S, openscion_tier::ObjectCache>;
```

The daemon's `NodeStore::open`, `BoundedSource::new`, `SocketSource`, and the
`serve_*` functions keep their signatures and behaviour.

## Out of scope

- The `openscion-cache` crate and its in-memory `ObjectBacking` (next chunk,
  which this unblocks).
- The server's framed object endpoint (later).
- Any change to the wire format, the store's behaviour, the bound, or the
  snapshot — this is relocation plus the backing generalization only.

## Acceptance

- `openscion-tier` builds and its relocated tests pass (store, backing, bound,
  transport), each source file ≥90% line coverage.
- `openscion-daemon` compiles against it with its public API unchanged; every
  existing daemon test (unit, `tests/binary.rs`, doctests) still passes.
- The four contract commands are green across the workspace, per-file coverage
  stays ≥90%, and `cargo deny` passes with the new member (no new third-party
  dependency — `openscion-tier` uses only `openscion-core` and `thiserror`,
  exactly what the moved code already used).

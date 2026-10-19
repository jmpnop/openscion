# Plan: node-local object store and snapshot boot

- **Status:** Done
- **Specification:**
  [`../specs/openscion-daemon-node-store.md`](../specs/openscion-daemon-node-store.md)
- **Design record:** POSD Design-It-Twice (chose the deep read-through module).
- **Phase:** 5 (client and daemon), fourth chunk — the daemon's first piece.

## Goal

Create `crates/openscion-daemon/` with a `NodeStore<S: ObjectSource>`: a bounded,
node-local, read-through cache of immutable objects behind a single `get(id)`,
plus `snapshot()` / `warm()` for snapshot boot. In-process only; the framed
transport is a later sub-chunk ([ADR 0004](../adr/0004-daemon-transport.md)).

## Approach

`NodeStore` is one deep module. It owns an evictable object cache (id → bytes on
disk, content-addressed fanout + atomic temp+fsync+rename, with removal and
sizes — the capabilities `ObjectStore` lacks) and an in-memory LRU index with a
byte budget, behind a `Mutex`. `get` checks local under the lock and returns on a
hit; on a miss it releases the lock, fetches from the `ObjectSource`, verifies
the bytes against the id by recovering the kind (`object_id` over
Blob/Tree/Changeset), writes to the cache, then re-locks to record size/recency
and evict down to the budget. `snapshot` reads the LRU order; `warm` runs the
miss path per id, least-recent first. The lock is never held across upstream I/O
or a disk write.

## Tasks

Test-first: the failing test, then the code that passes it.

1. **Crate skeleton** — `crates/openscion-daemon/Cargo.toml` (inherits workspace
   package/lints; `publish = false`; deps `openscion-core`, `thiserror`),
   `src/lib.rs` (crate docs + public surface), `src/error/mod.rs` (crate-wide
   `Error`/`Result`: `NotFound`, `Corrupt`, `Io`, `Core`).
2. **Evictable cache** — `src/cache/mod.rs` + `test.rs`: a private
   `ObjectCache` over a dir — `write(id, &bytes)`, `read(id) -> Option<Arc<[u8]>>`,
   `remove(id)`, fanout path, atomic write. Test write/read/remove and the
   fanout round-trip.
3. **Verification helper** — in the store module: `verify(id, bytes) ->
   Option<ObjectKind>` trying each kind via `object_id`. Test each kind matches
   and corrupt bytes match none.
4. **The node store** — `src/store/mod.rs` + `types.rs` + `test.rs`:
   `ObjectSource` trait, `Warmed`, `NodeStore::{open, get, snapshot, warm}`, the
   LRU index and budget. Tests, with an in-memory fake `ObjectSource`: hit makes
   no fetch; miss fetches once then caches; `NotFound`; `Corrupt` not cached;
   eviction drops LRU and keeps MRU; over-budget single object served; snapshot
   order; warm counts and MRU-survives-overflow; and a blocking-source test
   proving a local `get` is not blocked by an in-flight miss.
5. **Public surface** — `lib.rs` re-exports `NodeStore`, `ObjectSource`,
   `Warmed`, `Error`, `Result`.
6. **Docs** — module `//!` docs; spec/plan/ADR indexed from their READMEs;
   `ROADMAP.md`.

## Verification

- Focused: `cargo test -p openscion-daemon`.
- Full contract: `cargo fmt --all -- --check`, `cargo clippy --all-targets
  --all-features -- -D warnings`, `cargo build --all-targets --all-features`,
  `cargo test --all-features`.
- Rustdoc: `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps --all-features`.
- Per-file coverage ≥90% on every new source file.

## Checklist

- [x] `get` reads through, verifies, caches; hit makes no fetch
- [x] byte budget evicts LRU; over-budget single object still served
- [x] `NotFound` and `Corrupt` (not cached) behave per spec
- [x] snapshot/warm round-trip; MRU survives a warm past the budget
- [x] the index lock is not held across an upstream fetch
- [x] fmt, clippy, build, full test suite, rustdoc green
- [x] per-file coverage ≥90%

# Plan: extract the shared `openscion-tier` crate

Implements [`openscion-tier.md`](../specs/openscion-tier.md).

## Goal

Relocate the shared tier machinery out of `openscion-daemon` into a new
`openscion-tier` crate, generalizing the read-through store over an
`ObjectBacking` port, with no behaviour change: all existing daemon tests and
gates stay green.

## Approach

A relocation, done so the workspace is green at the end. Type names are kept
(`ObjectSource`, `ObjectCache`, `BoundedSource`, `SocketSource`, `Warmed`,
`serve_*`); the read-through store is renamed `NodeStore` → `Store<S, B>` in the
tier crate and re-exported by the daemon as `type NodeStore<S> = Store<S, ObjectCache>`.

## Tasks

### 1. Create the crate
- `crates/openscion-tier/Cargo.toml`: `publish = false`; deps `openscion-core`,
  `thiserror` (workspace). `[lints] workspace = true`.
- Root `Cargo.toml`: `members = ["crates/*"]` already picks it up; add
  `openscion-tier = { workspace = true }`? No — add to `[workspace.dependencies]`
  so the daemon takes it with `{ workspace = true }`.

### 2. Move the shared modules into `openscion-tier/src`
- `error/mod.rs` — the `Error`/`Result` (verbatim).
- `source/mod.rs` — `ObjectSource` (moved from store).
- `backing/mod.rs` + `backing/test.rs` — new `ObjectBacking` trait; `ObjectCache`
  (moved from daemon `cache/`) implementing it; its tests.
- `store/mod.rs` + `store/types.rs` + `store/test.rs` — `Store<S, B>` (generalized
  `NodeStore`), `Lru`, `Warmed`, `verify`; `open` on `Store<S, ObjectCache>`,
  `with_backing` generic.
- `bound/mod.rs` + `bound/test.rs` — `BoundedSource` (verbatim).
- `transport/mod.rs` + `transport/test.rs` — codec, `serve_*` over `&Store<S, B>`,
  `SocketSource`, tags (verbatim but generic in the store param).
- `test_support/mod.rs` — `scratch`, `blob_id`, `MemorySource` (moved; the tier
  tests need them).
- `lib.rs` — crate docs + public surface: `Error`, `Result`, `ObjectSource`,
  `ObjectBacking`, `ObjectCache`, `Store`, `Warmed`, `BoundedSource`,
  `SocketSource`, `serve_connection`, `serve_tcp`, `serve_unix`.

### 3. Make the daemon a consumer
- Delete the moved modules from `openscion-daemon/src` (`bound`, `cache`,
  `store`, `transport`, `error`, and the moved `test_support` bits).
- `Cargo.toml`: add `openscion-tier = { workspace = true }`.
- `lib.rs`: `pub use openscion_tier::{…}` the previously-exposed surface;
  `pub type NodeStore<S> = openscion_tier::Store<S, openscion_tier::ObjectCache>;`
  keep `mod config; mod daemon; mod snapshot;` and their exports.
- `daemon/mod.rs`, `snapshot/mod.rs`, `config/mod.rs`: swap `crate::{Error,…}`,
  `crate::store::…`, `crate::bound::…`, `crate::transport::…` for
  `openscion_tier::…`; keep the daemon-specific `Endpoint`, `Upstream`, and
  snapshot codec. The daemon keeps a tiny `test_support` only if its own tests
  need `scratch`/`MemorySource` (else drop).

### 4. Verify
- Four contract commands green; per-file ≥90% coverage over both crates;
  `cargo deny` clean. Daemon `tests/binary.rs` and doctests pass unchanged.

## Completion checklist

- [ ] `openscion-tier` created, modules moved, `ObjectBacking` introduced,
      `Store<S, B>` generalized.
- [ ] daemon consumes it, public API unchanged, `NodeStore` alias in place.
- [ ] workspace members + `[workspace.dependencies]` updated.
- [ ] four contract commands green; per-file ≥90%; `cargo deny` clean; binary
      test + doctests pass.
- [ ] spec/plan READMEs + ROADMAP updated.

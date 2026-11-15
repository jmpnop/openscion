# Plan: regional object cache (`openscion-cache`)

Implements [`docs/specs/openscion-cache.md`](../specs/openscion-cache.md). The
specification is the source of truth for behavior; this plan is the test-first
build order.

## Goal

Add the `openscion-cache` crate — an in-memory `ObjectBacking` and the regional
cache process that fronts it — on top of `openscion-tier`, and relocate the
shared `Endpoint` address type into `openscion-tier` so the daemon and the cache
share one parser. No new store, bound, or transport behavior; no third-party
dependency the daemon does not already use.

## Non-goals

- Non-object RPC forwarding (refs/log/derived data) — arrives with those RPCs.
- A warm set, snapshot, or on-disk state for the cache — it starts empty.
- Any change to store, bound, or transport semantics.

## Assumptions

- `openscion-tier` exposes `ObjectBacking`, `Store` + `Store::with_backing`,
  `BoundedSource`, `SocketSource`, `serve_tcp`/`serve_unix`, `Error`/`Result`.
- A regional cache is reached over TCP in practice but, like the daemon, accepts
  either transport through the shared `Endpoint`.

## Task 0 — relocate `Endpoint` into `openscion-tier`

A behaviour-preserving move so the cache does not duplicate the daemon's parser.

1. **Test first.** Add `crates/openscion-tier/src/endpoint/test.rs` with the
   daemon's endpoint cases (tcp parse/display, ipv6, unix parse/display, unix
   needs a path, malformed) plus a `dial` case proving `Endpoint::dial` yields a
   `SocketSource` for each transport. Wire `#[cfg(test)] mod test;` from the
   module root.
2. **Move the type.** Create `crates/openscion-tier/src/endpoint/mod.rs` holding
   `Endpoint` and `EndpointParseError` verbatim from
   `crates/openscion-daemon/src/config/types.rs`, plus
   `impl Endpoint { pub fn dial(&self) -> SocketSource }` mapping
   `Tcp`/`Unix` to `SocketSource::tcp`/`unix`. Add `mod endpoint;` and
   `pub use endpoint::{Endpoint, EndpointParseError};` to the tier `lib.rs`, and
   document the type in the crate overview.
3. **Make the daemon a consumer.** Delete
   `crates/openscion-daemon/src/config/types.rs`; in
   `crates/openscion-daemon/src/config/mod.rs` drop `mod types;` and re-export
   `Endpoint`/`EndpointParseError` from `openscion_tier`; the daemon `lib.rs`
   keeps exporting them, so its public surface is unchanged. Move the daemon's
   endpoint-only unit tests out of `config/test.rs` (they now live in the tier),
   leaving the `DaemonConfig` flag/builder tests.
4. **Simplify `Upstream::from_endpoint`** in
   `crates/openscion-daemon/src/daemon/mod.rs` to call `endpoint.dial()` for the
   `Socket` case — identical behavior, one mapping.
5. **Verify** the daemon suite (unit + `tests/binary.rs` + doctests) stays green.

## Task 1 — the crate skeleton

1. `crates/openscion-cache/Cargo.toml`: `publish = false`; deps
   `openscion-tier`, `openscion-core`, `clap`; `signal-hook` under
   `cfg(unix)`; `[lints] workspace = true`. Add `openscion-cache` to the root
   `[workspace.dependencies]`.
2. `crates/openscion-cache/src/lib.rs`: crate overview; `mod backing; mod
   config; mod cache; #[cfg(test)] mod test_support;`. Re-export the tier surface
   the binary and tests use (`BoundedSource`, `Error`, `ObjectSource`, `Result`,
   `SocketSource`, `serve_tcp`, `#[cfg(unix)] serve_unix`). Export
   `MemoryBacking`, `CacheConfig`, `Endpoint`/`EndpointParseError` (from tier),
   `RegionalCache`, the `CacheStore<S>` alias, and the `OPENSCION_CACHE_*`
   default constants.

## Task 2 — the in-memory backing (`src/backing/`)

Test first in `src/backing/test.rs`:

- a written object round-trips by id (`contains`/`read`);
- an absent id is `None`/`false`;
- a repeated write of the same id is idempotent and keeps the first bytes;
- `remove` frees the bytes and removing an absent id is a success.

Then `src/backing/mod.rs`: `MemoryBacking { objects: RwLock<HashMap<ObjectId,
Arc<[u8]>>> }` with `new`/`Default`, implementing `ObjectBacking`
(`contains`/`read`/`write`/`remove`), poison-tolerant like the store's index
lock. `read` clones the `Arc`; `write` inserts only if absent.

## Task 3 — the configuration (`src/config/`)

Test first in `src/config/test.rs`: only `--upstream` required takes every
default; every flag is read; a missing `--upstream` is a usage error; a bogus
`--listen`/`--upstream` is a usage error; the builders set each field.

Then `src/config/mod.rs`: `CacheConfig` (clap `Parser`) with `listen: Endpoint`
(default `tcp:127.0.0.1:7989`), `upstream: Endpoint` (required), `budget_bytes:
u64` (default `1 << 30`), `upstream_concurrency: usize` (default 32); the
`OPENSCION_CACHE_*` envs; `new(upstream)` + `with_listen`/`with_budget_bytes`/
`with_upstream_concurrency` builders; `DEFAULT_LISTEN`/`DEFAULT_BUDGET_BYTES`/
`DEFAULT_UPSTREAM_CONCURRENCY` constants.

## Task 4 — the assembly (`src/cache/`)

Test first in `src/cache/test.rs`, using a served in-process upstream (a tiny
`ObjectSource` holding one object, served with `serve_tcp` over a `Store`):

- `bind` then a client fetch over the wire reads the object through the upstream,
  serves it locally on the next read, and reports it in the store snapshot;
- a bind on a taken address is an `Error::Transport`;
- `listening()` reports the resolved `tcp:<addr>`.

Then `src/cache/mod.rs`: `CacheStore<S> = Store<S, MemoryBacking>` alias;
`RegionalCache { store: Arc<CacheStore<BoundedSource<SocketSource>>>, listener,
listening }`; `bind(&CacheConfig)` dialing the upstream through `dial()`,
wrapping in `BoundedSource`, opening the store over `MemoryBacking::new()`, and
binding the listener (`tcp`/`unix`); `listening()`; and `serve` — a
thread-per-connection accept loop, with a Unix `SIGINT`/`SIGTERM` clean exit and
a non-Unix serve-until-terminated, mirroring the daemon minus the snapshot.

## Task 5 — the binary and its integration test

1. `crates/openscion-cache/src/main.rs`: parse `CacheConfig`, `RegionalCache::
   bind`, print `openscion-cache listening on <endpoint>`, `serve`; map an error
   to `eprintln!` + `ExitCode::FAILURE`.
2. `crates/openscion-cache/tests/binary.rs`: stand up an in-process upstream
   server holding one object; spawn the binary with `--upstream` pointed at it;
   read the listen line; fetch the object through the cache over the wire;
   `SIGTERM` and assert a clean exit; assert a taken/malformed endpoint is a
   non-zero exit.

## Task 6 — docs and roadmap

- `docs/specs/README.md`: add the accepted spec row.
- `docs/plans/README.md`: add the plan bullet.
- `ROADMAP.md`: add the Done entry for the regional cache.

## Verification

Focused while iterating:

```sh
cargo test -p openscion-cache --all-features
cargo test -p openscion-tier --all-features
cargo test -p openscion-daemon --all-features
```

Full contract before completion:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features
cargo test --all-features
env RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps --all-features
.github/scripts/check-file-coverage.sh 90 <report>
cargo deny check
```

## Completion checklist

- [ ] Task 0 — `Endpoint` relocated to tier; daemon a consumer; daemon suite green.
- [ ] Task 1 — crate skeleton and workspace wiring.
- [ ] Task 2 — `MemoryBacking` with tests.
- [ ] Task 3 — `CacheConfig` with tests.
- [ ] Task 4 — `RegionalCache` assembly with tests.
- [ ] Task 5 — binary and integration test.
- [ ] Task 6 — docs and roadmap.
- [ ] Full contract green; per-file coverage ≥ 90%; `cargo deny` clean.

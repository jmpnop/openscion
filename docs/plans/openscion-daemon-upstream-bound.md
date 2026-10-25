# Plan: daemon upstream-concurrency bound and `Unavailable` load shedding

Implements [`openscion-daemon-upstream-bound.md`](../specs/openscion-daemon-upstream-bound.md).

## Goal

Cap the node store's concurrent upstream fetches, shed the overflow with
`Error::Unavailable`, and carry that shed across the wire so a lower tier
re-sheds instead of stampeding. One new decorator module, a new error variant,
a new transport tag, and one config option. The node store, cache, and snapshot
are untouched.

## Non-goals

Inbound admission limits, queueing/backpressure, retry policy, and the later
phase-7 tiers (regional cache, metadata store, propagation bus).

## Assumptions

- Concurrency is real: the daemon serves each connection on its own thread, so
  multiple `NodeStore::get` calls (hence `upstream.fetch` calls) run at once.
- No new dependency: the bound is a lock-free `AtomicUsize` slot count; the std
  library has no semaphore and does not need one here.

## Tasks

Each task is a failing test first, then the minimal code to pass it.

### 1. `Error::Unavailable`
- `crates/openscion-daemon/src/error/mod.rs`: add
  `#[error("unavailable: {0}")] Unavailable(String)`. It is the shed signal,
  distinct from `Upstream`.

### 2. `BoundedSource` — the bounded upstream
- New module `crates/openscion-daemon/src/bound/mod.rs` with `mod test;`.
- `pub struct BoundedSource<S> { inner: S, in_flight: AtomicUsize, limit: usize }`.
- `pub fn new(inner: S, limit: usize) -> Self`; `pub fn limit(&self) -> usize`.
- `impl<S: ObjectSource> ObjectSource for BoundedSource<S>`: `fetch` acquires a
  slot or sheds.
- `fn acquire(&self) -> Option<Permit<'_>>` via
  `in_flight.fetch_update(…, |n| (n < limit).then_some(n + 1))`.
- `struct Permit<'a>` whose `Drop` does `in_flight.fetch_sub(1, …)`.
- `fetch`: `let _permit = self.acquire().ok_or_else(|| Error::Unavailable(…limit…))?;`
  then `self.inner.fetch(id)`.
- `crates/openscion-daemon/src/bound/test.rs`:
  - within limit: a `MemorySource`-backed bound returns bytes / absence, slot
    freed each time (many sequential fetches under limit 1 all succeed);
  - inner error passes through and frees the slot;
  - at limit: with slots held, the next fetch returns `Unavailable` and does not
    touch the inner source; a released slot lets the next through;
  - concurrency: `limit` blocked fetches held across threads, the `limit + 1`-th
    sheds immediately; releasing unblocks and the held fetches complete;
  - the `Unavailable` message names the limit; `limit()` getter.
  - A `GateSource` test fake (blocks in `fetch` until released) added here or to
    `test_support`.

### 3. Carry the shed across the wire
- `crates/openscion-daemon/src/transport/mod.rs`:
  - `const TAG_UNAVAILABLE: u8 = 5;`
  - `serve_connection`: add arm
    `Err(Error::Unavailable(reason)) => write_frame(TAG_UNAVAILABLE, reason.as_bytes())`
    before the generic `Err(error)` arm.
  - `SocketSource::fetch`: add
    `Some((TAG_UNAVAILABLE, body)) => Err(Error::Unavailable(from_utf8_lossy))`.
- `crates/openscion-daemon/src/transport/test.rs`:
  - serve a store whose upstream sheds (a fake source returning
    `Error::Unavailable`, or a `BoundedSource` at limit 0), dial it, assert the
    client sees `Error::Unavailable` and not `Error::Upstream`.

### 4. Configure and wire the bound
- `crates/openscion-daemon/src/config/mod.rs`:
  - `pub const DEFAULT_UPSTREAM_CONCURRENCY: usize = 32;`
  - field `upstream_concurrency: usize` with
    `#[arg(long, env = "OPENSCION_DAEMON_UPSTREAM_CONCURRENCY", default_value_t = DEFAULT_UPSTREAM_CONCURRENCY)]`;
  - set it in `new`; add `with_upstream_concurrency`.
- `crates/openscion-daemon/src/config/test.rs`: default and builder set it;
  parsing the flag/env sets it.
- `crates/openscion-daemon/src/daemon/mod.rs`:
  - `store: Arc<NodeStore<BoundedSource<Upstream>>>`;
  - in `bind`: `let upstream = BoundedSource::new(Upstream::from_endpoint(…), config.upstream_concurrency);`.

### 5. Surface and docs
- `crates/openscion-daemon/src/lib.rs`: `mod bound;`,
  `pub use bound::BoundedSource;`, and
  `pub use config::… DEFAULT_UPSTREAM_CONCURRENCY`. Note the bound in the crate
  docs.
- `ROADMAP.md`: record the chunk under Done.
- `docs/specs/README.md`, `docs/plans/README.md`: add the rows.

## Verification

Focused: `cargo test -p openscion-daemon --all-features`.
Full contract: `cargo fmt --all -- --check`,
`cargo clippy --all-targets --all-features -- -D warnings`,
`cargo build --all-targets --all-features`, `cargo test --all-features`.
Coverage: `.github/scripts/check-file-coverage.sh 90 <report>` — `bound/mod.rs`
and every changed file ≥90%. Supply chain: `cargo deny check` (no new dep).

## Completion checklist

- [ ] `Error::Unavailable` added.
- [ ] `BoundedSource` + `Permit` with full unit tests (pass-through, shed,
      slot release on every outcome, concurrency, message).
- [ ] `TAG_UNAVAILABLE` on both ends with a wire round-trip test.
- [ ] config option + tests; `bind` wraps the upstream.
- [ ] exports, crate docs, ROADMAP, spec/plan READMEs.
- [ ] four contract commands green; per-file ≥90%; `cargo deny` clean.

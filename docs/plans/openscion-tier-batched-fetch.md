# Plan: batched object fetch over the framed wire

Implements
[`docs/specs/openscion-tier-batched-fetch.md`](../specs/openscion-tier-batched-fetch.md).
The specification is the source of truth; this plan is the test-first build
order. Inside `openscion-tier`; the single-object path is unchanged.

## Goal

Add a `get-many` framed request and an `ObjectSource::fetch_many`, have
`SocketSource` pipeline it over one pooled connection and `BoundedSource` count it
as one operation, and make `Store::warm` use it. No behaviour change elsewhere.

## Task 0 — the `get-many` request and batch serving (transport)

Test first in `crates/openscion-tier/src/transport/test.rs`:

- `fetches_a_batch_of_found_and_absent_objects`: a served store; one
  `SocketSource::fetch_many` of several ids (some present, some not) returns the
  bytes and `None`s in request order.
- `a_malformed_get_many_body_is_a_protocol_error`: a `get-many` whose body is not
  a whole number of ids is rejected.
- `a_batch_reuses_and_retries_a_pooled_connection`: a batch over a pooled
  connection works, and a stale pooled connection is retried (shared with the
  single-fetch retry path).

Then in `crates/openscion-tier/src/transport/mod.rs`:

- `const TAG_GET_MANY: u8 = 6;`.
- Factor `with_connection<T>(&self, impl Fn(&mut Conn) -> Result<T>) -> Result<T>`
  holding the pool checkout / stale-retry / check-in, and route both `fetch`
  (via `exchange`) and the new `fetch_many` through it.
- `exchange_many(&mut Conn, &[ObjectId]) -> Result<Vec<Response>>`: write one
  `get-many` (ids concatenated), read `N` reply frames into `Response`s; a
  transport/protocol failure is `Err` (connection unusable).
  `collect_outcomes(Vec<Response>) -> Result<Vec<Option<Arc<[u8]>>>>` maps them,
  short-circuiting on the first per-id error.
- `serve_connection`: dispatch on the request tag — `TAG_GET` as today,
  `TAG_GET_MANY` parses the ids (a `respond`/`ids_from` helper) and writes one
  reply per id with the same found/absent/unavailable/error logic.

## Task 1 — `fetch_many` on the trait, `SocketSource`, and `BoundedSource`

- `crates/openscion-tier/src/source/mod.rs`: add `fetch_many` to `ObjectSource`
  with the loop-`fetch` default and its docs.
- `SocketSource::fetch_many` (in transport): `with_connection(|c| exchange_many(c,
  ids))` then `collect_outcomes`; empty ids short-circuit to `Ok(vec![])`.
- `crates/openscion-tier/src/bound/mod.rs`: override `BoundedSource::fetch_many`
  to take one permit (shed with `Unavailable` when none) and call
  `inner.fetch_many`. Test first in `bound/test.rs`: a batch past the limit is
  shed as a unit; within the limit it passes results through.

## Task 2 — `Store::warm` fetches in batches

In `crates/openscion-tier/src/store/mod.rs`: rewrite `warm` to collect the
not-already-held ids least-recently-used first, `fetch_many` them in chunks of a
`WARM_BATCH` constant, and verify/store each found object in order. Keep the
`Warmed` counts, eviction order, and corrupt rejection. Test first in
`store/test.rs`: a source that counts `fetch_many` calls proves `warm` batches
(one call per chunk, not one per id); the existing `warm` tests stay green.

## Task 3 — docs and roadmap

- `docs/specs/README.md`, `docs/plans/README.md`: add the rows.
- `ROADMAP.md`: add the Done entry (object-path performance: batched fetch).

## Verification

```sh
cargo test -p openscion-tier -p openscion-daemon -p openscion-cache -p openscion-server --all-features
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features
cargo test --all-features
env RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps --all-features
.github/scripts/check-file-coverage.sh 90 <report>
cargo deny check
```

## Completion checklist

- [ ] Task 0 — `get-many` request, batch serving, `with_connection`, tests.
- [ ] Task 1 — `fetch_many` on the trait, `SocketSource`, `BoundedSource`, tests.
- [ ] Task 2 — `warm` batches, with its behaviour preserved and proven.
- [ ] Task 3 — docs and roadmap.
- [ ] Full contract green; per-file coverage ≥ 90%; `cargo deny` clean.

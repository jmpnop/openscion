# Plan: connection pooling for the framed client

Implements
[`docs/specs/openscion-tier-connection-pooling.md`](../specs/openscion-tier-connection-pooling.md).
The specification is the source of truth; this plan is the test-first build
order. Entirely inside `SocketSource`; the wire is unchanged.

## Goal

Make `SocketSource` reuse connections across fetches, with a one-shot retry when
a pooled connection was closed while idle. No protocol or caller change.

## Non-goals

- Batched fetch, pipelining, idle eviction, or a connection cap.

## Task 0 — pool and reuse in `SocketSource`

Test first in `crates/openscion-tier/src/transport/test.rs`:

- `reuses_a_pooled_connection_across_fetches`: a server whose accept loop counts
  accepted connections and serves each with `serve_connection`; three serial
  fetches through one `SocketSource` leave the accept count at 1.
- `a_stale_pooled_connection_is_retried`: a server that serves exactly one
  request per connection then drops it; two fetches both return the object, and
  the server accepts twice (the second fetch retried on a fresh connection after
  the pooled one was stale).
- `concurrent_fetches_through_a_shared_source_all_succeed`: several threads fetch
  through one cloned `SocketSource`; every result is correct.
- Keep the existing found/absent/unavailable/upstream/protocol cases green.

Then in `crates/openscion-tier/src/transport/mod.rs`:

- `SocketSource { endpoint: Endpoint, idle: Arc<Mutex<Vec<Conn>>> }`; the `tcp`
  and `unix` constructors start an empty pool; `#[derive(Clone)]` shares it.
- `#[derive(Debug)]` on `Conn` so `SocketSource`'s derive still holds.
- A private `Response` enum (`Found`/`Absent`/`Unavailable`/`Upstream`) with
  `outcome(self) -> Result<Option<Arc<[u8]>>>`, and a free `exchange(&mut Conn,
  ObjectId) -> Result<Response>` that writes the `get` and reads the reply,
  returning `Err` only on a transport/protocol failure (the connection is then
  unusable).
- `take_idle(&self) -> Option<Conn>` and `checkin(&self, Conn)` over the
  poison-tolerant mutex.
- `fetch`: take an idle connection and `exchange`; on success check it in and
  return `outcome()`; on failure drop it and fall through. Then dial a fresh
  connection, `exchange` (propagating its error), check in, and return
  `outcome()`.
- Update the `SocketSource` doc: connections are pooled and reused, with a
  one-shot retry for a stale pooled connection.

## Task 1 — docs and roadmap

- `docs/specs/README.md`, `docs/plans/README.md`: add the rows.
- `ROADMAP.md`: add the Done entry (object-path performance: connection reuse).

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

- [ ] Task 0 — pooling with reuse, stale-retry, and concurrency tests.
- [ ] Task 1 — docs and roadmap.
- [ ] Full contract green; per-file coverage ≥ 90%; `cargo deny` clean.

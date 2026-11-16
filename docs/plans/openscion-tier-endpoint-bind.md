# Plan: `Endpoint::bind` and `BoundListener`

Implements
[`docs/specs/openscion-tier-endpoint-bind.md`](../specs/openscion-tier-endpoint-bind.md).
The specification is the source of truth; this plan is the test-first build
order. Behaviour-preserving throughout.

## Goal

Lift the `Listener` enum, `bind_listener`, and serve-dispatch duplicated in the
daemon, the cache, and the server into `openscion-tier` as `Endpoint::bind ->
BoundListener`, symmetric with `Endpoint::dial`. Migrate all three consumers.

## Non-goals

- Any wire, transport, serving, or shutdown change.
- Any change to how each tier reports a bind failure.

## Task 0 — `BoundListener` and `Endpoint::bind` in the tier

Test first in `crates/openscion-tier/src/endpoint/test.rs`:

- `bind` on `tcp:127.0.0.1:0` yields a `BoundListener` whose `listening()`
  starts `tcp:127.0.0.1:` and is not `:0` (the port resolved);
- `bind` on a `unix:<path>` yields `listening()` equal to `unix:<path>`
  (`#[cfg(unix)]`);
- a bind on a taken TCP address is an `io::Error`;
- a `BoundListener` serves an `ObjectService`: spawn `bound.serve(&service)`,
  then a `SocketSource` fetch returns a stored object and *absent* for a miss,
  over TCP and (`#[cfg(unix)]`) over a Unix socket.

Then, in `crates/openscion-tier/src/endpoint/mod.rs`:

- `pub struct BoundListener` wrapping an inner `enum { Tcp(TcpListener),
  #[cfg(unix)] Unix(UnixListener) }` and the resolved `listening: String`,
  `#[derive(Debug)]`.
- `Endpoint::bind(&self) -> std::io::Result<BoundListener>`: bind the matching
  listener, compute `listening` (`tcp:{local_addr}` resolving port 0, or
  `unix:{path}`).
- `BoundListener::listening(&self) -> &str`.
- `BoundListener::serve<Svc: ObjectService + 'static>(&self, service: &Arc<Svc>)
  -> Result<()>`: match the inner listener and call `serve_tcp`/`serve_unix`.
- Export `BoundListener` from `lib.rs` and note it in the crate overview beside
  `Endpoint`/`dial`. The endpoint module gains the `transport`/`service`/`Arc`
  imports it now needs.

## Task 1 — migrate the daemon

In `crates/openscion-daemon/src/daemon/mod.rs`: delete the private `Listener`
enum and `bind_listener`; the `Daemon` holds `listener: BoundListener`. `bind`
calls `config.listen.bind().map_err(Error::Transport)?` and stores
`bound.listening()` as before. Both `serve` arms become
`thread::spawn(move || { let _ = bound.serve(&serving); })`; the Unix arm keeps
its signal-driven snapshot persistence, the non-Unix arm serves until the accept
loop ends. Verify the daemon suite (unit + `tests/binary.rs`) stays green.

## Task 2 — migrate the cache

In `crates/openscion-cache/src/cache/mod.rs`: the same removal and switch to
`BoundListener`; `RegionalCache::bind` uses `config.listen.bind()`, `serve`
spawns `bound.serve(&store)`. Nothing to persist, as today. Verify the cache
suite stays green.

## Task 3 — migrate the server

In `crates/openscion-server/src/object/mod.rs`: delete the `Listener` enum,
`bind_object_listener`, and `serve_objects`'s listener match. `Server` holds
`object: Option<BoundListener>`. `Server::bind` calls
`config.object_listen` → `endpoint.bind().map_err(|source| Error::ObjectListen {
endpoint: endpoint.clone(), source })?`, keeping `Error::ObjectListen`.
`object_addr` returns `bound.listening()`. `Server::serve` spawns
`bound.serve(&service)`. The `object` module keeps `RepositoryObjects` and
`map_object`; its listener plumbing is gone. Verify the server suite, including
`tests/object_endpoint.rs`, stays green.

## Task 4 — docs and roadmap

- `docs/specs/README.md`, `docs/plans/README.md`: add the rows.
- `ROADMAP.md`: add the Done entry (a tier cleanup resolving the deferred item
  from the server object-endpoint chunk).

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

- [ ] Task 0 — `Endpoint::bind`/`BoundListener` with tests.
- [ ] Task 1 — daemon migrated, suite green.
- [ ] Task 2 — cache migrated, suite green.
- [ ] Task 3 — server migrated, suite green.
- [ ] Task 4 — docs and roadmap.
- [ ] Full contract green; per-file coverage ≥ 90%; `cargo deny` clean.

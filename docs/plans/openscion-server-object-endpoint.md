# Plan: the server's framed object endpoint

Implements
[`docs/specs/openscion-server-object-endpoint.md`](../specs/openscion-server-object-endpoint.md).
The specification is the source of truth for behavior; this plan is the
test-first build order.

## Goal

Let `openscion-server` serve the framed object RPCs (ADR 0004) from its
authoritative `Repository`, reusing `openscion-tier`, so the read-through chain
(client → daemon → cache → server) has an origin. Widen the tier transport to
serve any object service rather than only a cache `Store`; add the repository
adapter, the `--object-listen` option, and the second listener. No new
third-party dependency; the daemon and cache are unchanged.

## Non-goals

- Non-object RPC forwarding, and any write path over the framed wire.
- Replacing the HTTP `/v1/objects` route.
- Any change to the wire form or the per-connection serving semantics.
- Consolidating the per-tier `bind_listener` into `Endpoint::bind` — a later
  cleanup; this change follows the existing per-tier convention.

## Assumptions

- `openscion-tier` exposes the framed transport (`serve_tcp`/`serve_unix`/
  `serve_connection`), `SocketSource`, `Endpoint`, `Store`, `Error`/`Result`.
- `openscion_core::Repository::object(id) -> Result<Arc<[u8]>>` returns the
  authoritative bytes, and `openscion_core::Error::NotFound(ObjectId)` is its
  absent case.
- The server already holds its repository as `SharedRepository`
  (`Arc<RwLock<Repository>>`), shared with the HTTP handlers.

## Task 0 — widen the transport to an `ObjectService` (`openscion-tier`)

A behaviour-preserving generalization so the server can serve the same wire.

1. **Test first.** In `crates/openscion-tier/src/transport/test.rs`, add a case
   that serves a hand-written `ObjectService` (not a `Store`) — one holding a
   single object, returning `NotFound` otherwise — over a connection and proves a
   `SocketSource` fetch returns the bytes and *absent* for a miss. Keep the
   existing `Store`-served cases: they prove `Store` still serves.
2. **Add the trait.** In `crates/openscion-tier/src/source/` (beside
   `ObjectSource`) or a new `service` module, define
   `pub trait ObjectService: Send + Sync { fn get(&self, id: ObjectId) -> Result<Arc<[u8]>>; }`
   with the serving contract documented (Ok → found, `NotFound` → absent,
   `Unavailable` → shed, else → error). Implement it for `Store<S, B>` by
   forwarding to its inherent `get`.
3. **Generalize the serve functions.** Change `serve_connection`, `serve_tcp`,
   and `serve_unix` from `&Store<S, B>` / `&Arc<Store<S, B>>` to a generic
   `Svc: ObjectService` (`+ 'static` for the spawning variants). The match on
   `get`'s result is unchanged. Export `ObjectService` from the tier `lib.rs` and
   mention it in the crate overview beside the transport.
4. **Verify** the tier suite, then the daemon and cache suites, stay green: their
   `serve_tcp(&listener, &Arc<Store<…>>)` call sites still compile, since
   `Store: ObjectService`.

## Task 1 — the repository object service and listener (`src/object/`)

Test first in `crates/openscion-server/src/object/test.rs`:

- `RepositoryObjects::get` returns the exact bytes for a stored id;
- returns `tier::Error::NotFound` for an absent id;
- (assembly) `bind_object_listener` on a taken address is an error, and reports
  the resolved `tcp:<addr>`.

Then `crates/openscion-server/src/object/mod.rs`:

- `RepositoryObjects(SharedRepository)` with `new`, implementing
  `openscion_tier::ObjectService`: `get` takes the read lock (the same
  poison-tolerant `read` the API uses), calls `repository.object(id)`, maps
  `Ok(bytes) -> Ok(bytes)`, core `Err(NotFound(_)) -> Err(tier NotFound(id))`,
  and any other engine error to `Err(tier Upstream(message))`.
- A `Listener` enum (`Tcp`/`#[cfg(unix)] Unix`) and `bind_object_listener(&Endpoint)
  -> Result<(Listener, String)>`, mirroring the daemon/cache convention, mapping
  a bind failure to `Error::Bind`.
- `serve_objects(listener, service)` spawning the tier accept loop
  (`serve_tcp`/`serve_unix`) on the given listener and service.

## Task 2 — the configuration (`src/config/`)

Test first in `src/config/test.rs`: the default config has no object endpoint; a
valid `--object-listen tcp:…`/`unix:…` parses to an `Endpoint`; a bogus value is
a usage error; the existing `--listen`/`--data-dir` cases are unchanged.

Then `src/config/mod.rs`: add `pub object_listen: Option<Endpoint>` to
`ServerConfig` with `#[arg(long, env = "OPENSCION_OBJECT_LISTEN")]`, and a
`with_object_listen(endpoint)` builder. `Endpoint` comes from `openscion_tier`.
Keep `#[non_exhaustive]` and the existing fields/defaults.

## Task 3 — the assembly (`src/http/`, `src/error/`, `src/main.rs`)

1. `src/error/mod.rs`: ensure a `Bind`-shaped variant covers the framed
   listener's bind failure (reuse the HTTP `Bind` variant or add an object-listen
   arm), with a test producing it.
2. `src/http/mod.rs`: `Server` gains an optional bound framed listener and the
   `RepositoryObjects` service. `Server::bind` binds it when
   `config.object_listen` is set (fail-fast); add `object_addr(&self) ->
   Option<&str>`. `Server::serve` spawns `serve_objects` on a thread before
   `axum::serve`, so the framed endpoint runs alongside the HTTP server and is
   torn down with the process. A server with no object endpoint is unchanged.
   Test first in `src/http/test.rs`: a bound server with an object endpoint
   reports both addresses; a framed fetch against a serving instance returns a
   stored object and *absent* for a miss; a server without the option still
   serves HTTP.
3. `src/main.rs`: print the framed listen line when configured, beside the HTTP
   one.

## Task 4 — the binary integration test (`tests/`)

`crates/openscion-server/tests/object_endpoint.rs`: bind a server with an
in-memory repository and `--object-listen tcp:127.0.0.1:0`; serve it under a
shutdown trigger; upload a blob over the HTTP API (`/v1/blobs`); then dial the
framed endpoint with a tier `SocketSource` and assert `fetch(id)` returns the
exact bytes and `fetch(absent)` is `None`. This proves a cache/daemon can read
through the server over the real wire, end to end, using the real HTTP ingest and
the new framed egress.

## Task 5 — docs and roadmap

- `docs/specs/README.md`: add the accepted spec row.
- `docs/plans/README.md`: add the plan bullet.
- `ROADMAP.md`: add the Done entry for the server's framed object endpoint.

## Verification

Focused while iterating:

```sh
cargo test -p openscion-server --all-features
cargo test -p openscion-tier --all-features
cargo test -p openscion-daemon -p openscion-cache --all-features
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

- [ ] Task 0 — transport serves `ObjectService`; `Store` impls it; tier, daemon,
      cache suites green.
- [ ] Task 1 — `RepositoryObjects` service and object listener with tests.
- [ ] Task 2 — `--object-listen` configuration with tests.
- [ ] Task 3 — `Server` binds and serves the framed endpoint; `main` prints it.
- [ ] Task 4 — end-to-end binary integration test over the framed wire.
- [ ] Task 5 — docs and roadmap.
- [ ] Full contract green; per-file coverage ≥ 90%; `cargo deny` clean.

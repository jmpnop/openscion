# The server's framed object endpoint

- **Status:** Accepted
- **Owner:** OpenScion maintainers

## Problem

The [architecture](openscion-architecture.md#uniform-rpcs-and-strict-tiering)
requires that every tier of the object system speak the same wire:

> The daemon, the cache, and the server answer the **same object RPCs with the
> same semantics**, so a client never needs to know the topology: it talks to
> whatever is nearest.

and places the server at the top of that chain:

> **`openscion-server`** — the authority. [...] It reads objects from the object
> store when no cache above it has them.

The daemon and the regional cache already serve the framed object RPCs
([ADR 0004](../adr/0004-daemon-transport.md)) from a read-through
[`Store`](openscion-tier.md), and the [`openscion-tier`](openscion-tier.md)
crate holds the transport they share. The **server does not yet speak that
wire** — it answers objects only over its HTTP JSON API
([`/v1/objects`](openscion-server.md)). So the read-through chain
(client → daemon → cache → server) has no origin to terminate at: a cache's
`--upstream` cannot point at the server over the framed transport, because the
server does not listen for it.

This specification adds that endpoint: the server serves the framed object RPCs
from its authoritative [`Repository`](openscion-architecture.md#storage-ports),
reusing `openscion-tier`, so it becomes the origin every lower tier reads
through.

## Goals

- The server answers the framed object-get RPC (ADR 0004) over a listener of its
  own, serving object bytes straight from its authoritative `Repository` — the
  same wire the daemon and cache already speak, so a cache can read through the
  server with no new code.
- Reuse of `openscion-tier`'s transport for the framing, the listener, and the
  per-connection serving; the server adds only the adapter from its repository
  and its own listener assembly.
- The framed endpoint is opt-in and independent of the HTTP API: a server with
  no framed endpoint configured behaves exactly as today.
- No new third-party dependency: the framed endpoint reuses the tier crate the
  server can already depend on.

## Non-goals

- **Non-object RPCs.** Refs, the log, and derived-data queries are answered only
  over HTTP, by the server alone; the framed transport carries object reads, as
  it does at every tier today. The architecture's forwarding of mutable queries
  is a later piece arriving with those RPCs.
- **Replacing the HTTP object route.** `/v1/objects` stays; it is how `opnscn`
  and the GUI fetch batches today. The framed endpoint is the tier-facing egress
  the daemon and cache dial, not a replacement for the client-facing API.
- **A write path over the framed wire.** The framed transport is get-only at
  every tier; blobs are still uploaded over HTTP (`/v1/blobs`). This endpoint is
  read-only.
- **New store, bound, or transport behaviour.** The wire form and the
  per-connection serving live in `openscion-tier` and are unchanged in substance;
  this spec only widens what the transport can serve (below).

## Proposed behavior

### Widening the transport to serve any object service

The tier transport today serves one concrete type — a
[`Store<S, B>`](openscion-tier.md), the bounded read-through cache — and calls
its `get`. The server is **not** a cache `Store`: its objects are authoritative,
held in a `Repository`, and must never be evicted under a byte budget. A `Store`
built over the repository would be wrong twice — its budgeted LRU would evict
authoritative objects, and its read path only consults the backing for ids it
has itself admitted on a miss, so it could not serve an object the repository
already holds.

So the transport is widened to serve any **object service** — anything that
resolves an id to bytes with the serving contract the wire already encodes:

```rust
/// A tier's object-read surface, as the framed transport serves it.
pub trait ObjectService: Send + Sync {
    /// The bytes of `id`. `Ok` answers found; `Err(NotFound)` answers absent;
    /// `Err(Unavailable)` sheds; any other `Err` answers a protocol error.
    fn get(&self, id: ObjectId) -> Result<Arc<[u8]>>;
}
```

`Store<S, B>` implements `ObjectService` by forwarding to its existing `get`, so
the daemon and the cache — which serve an `Arc<Store<…>>` — are unchanged: the
`serve_tcp`/`serve_unix`/`serve_connection` signatures widen from `&Store<S, B>`
to a generic `&Svc: ObjectService`, and an `Arc<Store<…>>` still satisfies them.
This is the minimal change that lets the server reuse the transport the ADR
always intended all three tiers to share.

### The repository as an object service

The server holds its repository as a `SharedRepository`
(`Arc<RwLock<Repository>>`), shared with the HTTP handlers. A small adapter makes
it an `ObjectService`:

```rust
let objects = RepositoryObjects::new(repository);
// get takes the read lock, reads the authoritative object store, and maps:
//   Repository::object(id) == Ok(bytes)            -> Ok(bytes)        (found)
//   Repository::object(id) == Err(core NotFound)   -> Err(NotFound id) (absent)
//   any other engine error                         -> Err(Upstream …)  (error)
```

It takes the read lock the HTTP read handlers take, so a framed get and an HTTP
request see the same repository under the same lock discipline. The origin has
**no tier above and sheds nothing**: it never returns `Unavailable`, because
there is no upstream fetch to bound — it reads its own store.

### The configuration

`ServerConfig` gains one option; the existing `--listen`/`--data-dir` are
unchanged:

| Flag | Env | Default | Meaning |
| --- | --- | --- | --- |
| `--object-listen` | `OPENSCION_OBJECT_LISTEN` | *(unset: no framed endpoint)* | `Endpoint` to serve the framed object RPCs on, for caches and daemons to read through. |

The value is the tier's scheme-prefixed [`Endpoint`](openscion-tier.md) —
`tcp:<addr>` or `unix:<path>` — the same form the daemon's and cache's
`--listen`/`--upstream` take, so one operator-facing spelling spans every tier.
When it is unset the server serves HTTP only, exactly as today; the object
endpoint is a deliberate choice, like persistence (`--data-dir`) and non-loopback
exposure (`--listen`).

### The assembly

`Server::bind` opens the stores and binds the HTTP listener as today, and, when
`--object-listen` is set, also binds the framed listener — so a bad or taken
object endpoint fails fast, before serving. `Server::serve` spawns the framed
transport's accept loop on a dedicated thread serving the `RepositoryObjects`
over the shared repository, then runs the async HTTP server until its shutdown
future resolves. The framed accept loop blocks for the life of the process and
is torn down with it; because objects are immutable and each framed connection
is stateless, a connection dropped at shutdown costs a caller only a retry, so
the endpoint needs no drain of its own. The bound framed address is exposed so
the binary can print it beside the HTTP line.

The sync transport runs on a thread rather than in the async runtime because the
tier transport is deliberately runtime-free (`std::net`, a thread per
connection) and the server is the one tier that also runs an async HTTP surface;
bridging the two with a thread keeps the tier crate free of an async runtime, as
its charter requires.

## Invariants and constraints

- The framed endpoint is read-only and get-only: it serves object bytes by id
  and nothing else, as the wire does at every tier.
- A server with no `--object-listen` is byte-for-byte the server of today: the
  HTTP API, its routes, and its behaviour are unchanged.
- The framed get and the HTTP handlers share one `Repository` under one lock; a
  framed read takes the read lock, never the write lock.
- The origin sheds nothing and has no upstream: it never returns `Unavailable`
  and never dials a tier above.
- The server adds no third-party dependency: the framed endpoint is built on
  `openscion-tier`, and the adapter and assembly are the server's own.
- The daemon and the cache are unchanged: widening the transport to
  `ObjectService` keeps their `Store`-served endpoints compiling and behaving as
  before.
- `unsafe` is forbidden, as workspace-wide.

## Acceptance criteria

- `ObjectService` is served by the transport, and `Store` implements it, so the
  tier, daemon, and cache suites stay green with no change to their serving code.
- A `RepositoryObjects` over a repository answers `get`: found bytes for a stored
  id, `NotFound` for an absent id, and an error for an engine failure.
- Over the real framed wire, a tier client (`SocketSource`) fetches an object the
  server holds and receives its exact bytes, and receives *absent* for an id the
  server does not hold — proving a cache can read through the server.
- The `openscion-server` binary, given `--object-listen`, binds and prints both
  its HTTP and framed listen lines, answers a framed object read, and still
  serves its HTTP API; a bad or taken object endpoint is a non-zero exit before
  serving.
- A server with no `--object-listen` serves HTTP exactly as before.
- Every source file keeps at least 90% line coverage; the four contract commands
  are green; `cargo deny` passes.

## Open questions

None. The design reuses `openscion-tier`'s transport and mirrors the accepted
daemon and cache assemblies, differing only where the server's role dictates:
it serves its authoritative repository rather than a budgeted cache, and it has
no tier above to shed toward.

A later cleanup, out of this change's scope, is to lift the per-tier
`bind_listener` (duplicated now in the daemon, the cache, and here) into an
`Endpoint::bind` on `openscion-tier`, symmetric with the existing
`Endpoint::dial`; this spec deliberately follows the established per-tier
convention rather than refactor the daemon and cache in the same change.

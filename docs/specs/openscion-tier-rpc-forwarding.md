# Forwarding non-object RPCs over the framed wire

- **Status:** Accepted
- **Owner:** OpenScion maintainers

## Problem

The [architecture](openscion-architecture.md#uniform-rpcs-and-strict-tiering)
answers the object RPCs at every tier but routes the rest to the server:

> Mutable queries — refs, the log, derived-data lookups — are always answered by
> the server. Lower tiers forward them; they never cache them.

and [ADR 0004](../adr/0004-daemon-transport.md) fixes the wire for them:

> The framed request/response set is the object RPCs (`get` by id, batched
> fetch) plus the forwarded mutable queries, identical at every tier.

The framed transport carried only object reads. So a tier that received a
non-object request had nothing to do with it: the daemon and the cache could not
forward refs, the log, or a changeset creation to the server over the wire they
already speak. This specification adds that — the forwarded-RPC envelope, the
server answering it, and the lower tiers relaying it.

## Goals

- One framed request carries any non-object operation, and the server answers it
  from its engine, so a caller reaches the full API over the framed wire.
- The daemon and the cache forward such a request to the tier above unchanged,
  without understanding it, and relay the reply — strict tiering for the mutable
  queries.
- The envelope is one request type and one response type covering every
  operation, so adding an operation is adding a variant and a dispatch arm, never
  a second schema (ADR 0004).

## Non-goals

- A client that routes through the daemon: `opnscn` still calls the server over
  HTTP. This builds the forwarding path the architecture prescribes; wiring the
  client onto it is a later change. The path is proven end-to-end by tests.
- Caching mutable queries at a lower tier: they are always forwarded, never
  cached, as the architecture requires.
- New engine behaviour: the server answers each RPC with the same engine call
  its HTTP handler already makes.

## Proposed behavior

### The envelope

`openscion_core::rpc` adds two types — a `Request` enum with a variant per
non-object operation (create/read a changeset; stat, tree, file, diff, log,
ancestor, merge-base; read/update a ref; rebase, revert, land) and a `Response`
enum with the matching reply per operation plus an `Error` variant. `Error`
carries the same stable `code` and message the HTTP API returns (`ErrorBody`), so
a caller reconstructs the failure without a second error taxonomy. Both derive
serde, like every protocol payload.

### The wire

The framed transport gains a request frame `get-many`'s successor, `rpc`, whose
body is the `Request` encoded with **postcard** — a compact binary serde form,
added as a tier dependency — and a reply frame `rpc-reply` whose body is the
encoded `Response`. Objects stay on their own raw frames; only the structured
mutable queries use the envelope. The client encodes a `Request`, sends one
`rpc` frame, and decodes the one `rpc-reply`, over a pooled connection with the
same stale-connection retry as a `get`. An engine failure is a decoded
`Response::Error`, not a transport fault, so the connection stays reusable.

### Serving and forwarding

`ObjectService` — what the transport serves — gains an `rpc(Request) ->
Response` method with a default that declines (`Response::Error`, so the
connection survives), because a tier that only serves objects answers no mutable
query itself. Two implementations override it:

- The **server** (`RepositoryObjects`) dispatches each `Request` variant to its
  `Repository`, mirroring the HTTP handler of the same name — one engine call,
  its result shaped into the matching `Response` — and renders any engine error
  as `Response::Error`. It is the origin: it answers, it does not forward.
- A **`Forwarding`** service wraps a tier's read-through `Store` (for objects)
  and an optional upstream `SocketSource` (for RPCs): `get` serves from the
  store, `rpc` relays to the tier above and returns its reply. The daemon and the
  cache serve a `Forwarding` instead of their bare store, so they answer objects
  locally and forward mutable queries upstream. A detached daemon has no upstream
  and declines.

The forwarding tier never decodes the envelope's meaning: it relays the request
and the reply, so a new operation reaches the server through an unchanged daemon
and cache.

## Invariants and constraints

- The object `get`/`get-many` path and its raw frames are unchanged; the `rpc`
  frames are added alongside.
- A tier forwards a mutable query to exactly the one tier above it, and never
  caches the reply.
- An engine failure crosses the wire as `Response::Error` with the HTTP code and
  message; only a transport or protocol fault is a tier error that drops the
  connection.
- The only new third-party dependency is `postcard` (pure-Rust binary serde, no
  transport or runtime), in the tier crate; `unsafe` remains forbidden.

## Acceptance criteria

- A `Request`/`Response` round-trips over the framed wire, and the server answers
  each operation from its repository, rendering a missing ref or changeset as a
  `Response::Error` with code `NotFound`.
- A `Forwarding` tier relays an RPC to the tier above and returns its reply, and
  a detached tier (no upstream) declines; a plain `Store` declines by default.
- The daemon and the cache serve objects and forward RPCs, with their object
  serving, warm boot, snapshot, and load-shedding unchanged.
- The tier, daemon, cache, and server suites stay green; the four contract
  commands pass; every source file keeps at least 90% line coverage; `cargo deny`
  passes.

## Open questions

None. The envelope follows ADR 0004, and the only consumer today is the test
suite; the client moves onto the forwarding path in a later change.

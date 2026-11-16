# Connection pooling for the framed client

- **Status:** Accepted
- **Owner:** OpenScion maintainers

## Problem

[`SocketSource`](openscion-tier.md) — the framed client a tier uses to read
through the tier above — **opens a fresh connection on every fetch**:

> Each fetch opens a fresh connection.

The framed transport already supports many requests on one connection (the
server's accept loop serves a connection until the peer closes it), but the
client never reuses one. So an operation that reads many objects — a clone or a
checkout fetching thousands of blobs through the daemon — pays a full TCP or
Unix connect, and its handshake, **per object**. That is the dominant avoidable
cost on the hot read path, and it falls on exactly the bulk operations the tier
exists to make fast.

## Goals

- `SocketSource` reuses connections across fetches instead of dialing one per
  fetch, so a burst of reads amortizes one connect over many requests.
- A connection the peer closed while it sat idle never surfaces as an error: a
  stale reuse is retried once on a fresh connection.
- No change to the wire, the request/response semantics, or any caller: pooling
  is entirely inside `SocketSource`.

## Non-goals

- Batching several object ids into one request — a separate change over the same
  connection reuse.
- A connection limit or idle eviction timer: the pool is self-bounding (below),
  and the tier above already bounds upstream concurrency with `BoundedSource`.
- Pipelining (multiple in-flight requests on one connection): each fetch uses a
  connection exclusively for its single request/response round-trip.

## Proposed behavior

`SocketSource` keeps a pool of idle connections behind a mutex, shared across its
clones (an `Arc`), so the one `SocketSource` a store holds reuses connections no
matter how many threads fetch through it:

- **Checkout.** A fetch takes an idle connection from the pool, or dials a fresh
  one when the pool is empty.
- **The exchange.** It writes the `get` frame and reads the response frame on
  that connection — the existing protocol, unchanged.
- **Check-in.** After a *complete* frame exchange the connection is healthy and
  returns to the pool for the next fetch — whether the response was found,
  absent, unavailable, or an upstream error, since all four are well-formed
  replies. Only a transport or protocol failure, which means the connection is
  unusable, drops it.
- **Stale reuse.** A pooled connection the peer closed while idle fails the
  exchange at the transport. A fetch that took its connection *from the pool*
  therefore retries once on a freshly dialed connection, so a closed idle
  connection costs a retry, not an error. A failure on a *freshly dialed*
  connection is a real error and is returned.

The pool is **self-bounding**: a connection enters it only by being checked out
first, so the idle count never exceeds the peak number of concurrent fetches —
which the `BoundedSource` above a real tier already caps. No separate limit is
needed.

Exactly-once semantics are unaffected: a retry re-sends an object `get`, which is
idempotent, and the request only ever reaches a fresh connection after the
pooled one failed before producing a reply.

## Invariants and constraints

- A connection is used by at most one fetch at a time: it is checked out for the
  duration of one request/response round-trip and only then returned.
- Only a cleanly completed exchange returns a connection to the pool; a
  transport or protocol failure discards it.
- The retry happens only for a connection taken from the pool, never for a freshly
  dialed one, so a genuine connect or server failure is reported, not masked.
- Behaviour over the wire is identical to dialing per fetch; pooling is an
  internal optimization with no observable protocol change.
- No new third-party dependency; `unsafe` remains forbidden.

## Acceptance criteria

- Several serial fetches through one `SocketSource` cause the server to accept
  **one** connection, not one per fetch.
- A fetch whose pooled connection was closed by the server still returns the
  correct result, by retrying on a fresh connection.
- Concurrent fetches through a shared `SocketSource` all return correct results.
- Found, absent, unavailable, and upstream-error responses are unchanged, and the
  tier, daemon, cache, and server suites stay green.
- The four contract commands are green, every source file keeps at least 90%
  line coverage, and `cargo deny` passes.

## Open questions

None. The change is internal to `SocketSource` and preserves the wire protocol.

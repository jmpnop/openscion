# Batched object fetch over the framed wire

- **Status:** Accepted
- **Owner:** OpenScion maintainers

## Problem

A tier that needs many objects from the tier above fetches them **one request at
a time**: [`Store::warm`](openscion-tier.md) loops `ObjectSource::fetch` over a
snapshot, and even with [connection reuse](openscion-tier-connection-pooling.md)
each id is a separate request that waits for its reply before the next is sent.
Over a cross-machine link that serializes the round-trip latency across every
object — a daemon warming thousands of ids pays thousands of sequential
round-trips. The HTTP API already fetches a batch in one request
(`POST /v1/objects`); the framed wire does not.

## Goals

- One framed request carries many object ids and is answered with their results
  in order, so a bulk read pays one round-trip of requests rather than one per
  object.
- `Store::warm` uses it, so warming a snapshot is a handful of batched
  round-trips instead of one per id — the concrete win today.
- The batch composes with the existing tiering: it reuses a pooled connection,
  and the upstream-concurrency bound counts a batch as one upstream operation.
- No change to the single-object path or to any existing behaviour; batching is
  additive.

## Non-goals

- A client-facing batched read-through (`Store::get_many`): the engine has no
  caller for it yet (the client still reads over HTTP), so it is not added
  speculatively. `warm` is the consumer.
- Pipelining unrelated requests, or streaming a batch larger than memory: a batch
  is bounded by `warm`'s chunk size and its results are held together.

## Proposed behavior

### The wire

A new request carries a list of ids; the reply is one response frame per id, in
request order, reusing the existing response frames:

- **Request** `get-many`: body is the ids concatenated, each a 32-byte
  [`ObjectId`] (so `N` ids are `32N` bytes; a body that is not a whole number of
  ids is a protocol error).
- **Reply**: `N` response frames — `found`/`absent`/`unavailable`/`error`, the
  same frames a single `get` answers — written in the order the ids were
  requested. The client reads exactly `N`.

The serving side answers each id from its [`ObjectService`] exactly as it answers
a single `get`, so a tier that serves single gets serves a batch with the same
semantics; only the request framing is new. Reading all `N` replies keeps the
connection usable for reuse: a per-id *unavailable* or *error* is a value in the
batch, not a transport fault, so the connection returns to the pool and the batch
call reports the first such error.

### The client

`ObjectSource` gains:

```rust
/// The bytes of each id, in order: `Some` when found, `None` when no tier holds
/// it. A default implementation fetches them one at a time.
fn fetch_many(&self, ids: &[ObjectId]) -> Result<Vec<Option<Arc<[u8]>>>> {
    ids.iter().map(|&id| self.fetch(id)).collect()
}
```

so every existing source keeps working unchanged. [`SocketSource`] overrides it
to send one `get-many` and read the `N` replies over a single pooled connection,
with the same stale-connection retry as `fetch`. [`BoundedSource`] overrides it
to take **one** slot for the whole batch — a batch is one upstream operation — so
`N` ids do not consume `N` slots.

### The consumer

`Store::warm` collects the ids it does not already hold, least-recently-used
first, and fetches them with `fetch_many` in chunks, verifying and storing each
found object in order so a set larger than the budget still leaves the
most-recently-used resident. Its result — how many were fetched and how many no
tier held — and its corrupt-byte rejection are unchanged; only the fetches are
batched.

## Invariants and constraints

- The single-object `fetch`/`get` path and its wire are unchanged; `get-many` is
  an added request, not a replacement.
- A batch reply is `N` frames for `N` ids, in request order; the client reads all
  `N`, so a pooled connection stays consistent.
- The upstream-concurrency bound treats a batch as one operation: a batch is shed
  or admitted as a whole, never partway.
- `warm` fetches least-recently-used first and skips ids already held, exactly as
  before, so its counts, eviction order, and corrupt rejection are preserved.
- No new third-party dependency; `unsafe` remains forbidden.

## Acceptance criteria

- A `get-many` over the wire returns each id's bytes or absence in request order,
  mixing found and absent, and a malformed (not-id-aligned) request is a protocol
  error.
- `fetch_many`'s default loops `fetch`; `SocketSource` answers a batch in one
  round-trip over one pooled connection; `BoundedSource` admits or sheds a batch
  as one operation.
- `Store::warm` issues batched fetches (one call for a chunk, not one per id) and
  keeps its existing counts, eviction order, and corrupt rejection — its tests
  stay green.
- The tier, daemon, cache, and server suites stay green; the four contract
  commands pass; every source file keeps at least 90% line coverage; `cargo deny`
  passes.

## Open questions

None. The batch reuses the existing response frames and the single-get serving
semantics, and its only consumer today is `warm`.

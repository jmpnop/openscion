# OpenScion daemon: upstream-concurrency bound and `Unavailable` load shedding

- **Status:** Accepted
- **Phase:** 7 (network tiers), first chunk
- **Depends on:** the daemon node store and framed transport (phase 5).

## Why

The architecture pins strict tiering
([`openscion-architecture.md`](openscion-architecture.md)):

> Tiering is strict: each tier calls only the tier directly above it, and each
> **bounds its upstream concurrency**. When a tier is saturated it **sheds load
> with `Unavailable`** rather than queueing unboundedly or skipping a tier, so a
> failed regional cache cannot turn into a stampede on the server.

The node store already reads through to the tier above on a miss
([`NodeStore::get`](../../crates/openscion-daemon/src/store/mod.rs)), but nothing
caps how many of those upstream fetches run at once. Under a burst of misses,
every serving thread can call the tier above simultaneously; a slow or failing
upstream then collects an unbounded fan-in, and a lower tier that keeps retrying
turns one tier's trouble into a stampede on the one above it.

This chunk adds the bound and the shed, and makes the shed legible across a tier
boundary so a lower tier backs off instead of amplifying.

## What

### A bounded upstream

The node store's upstream fetches are capped at a configured maximum number in
flight at once. While that many fetches are outstanding, a further fetch does
**not** queue or block: it is **shed** — it returns `Unavailable` immediately.
A fetch that completes (with bytes, with an absence, or with an error) frees its
slot for the next.

The bound sits between the node store and whatever is above it, as a decorator
over the upstream `ObjectSource` — `BoundedSource`. It is general over any
source, holds no store or transport knowledge, and is the only new behavior; the
node store, cache, and snapshot are unchanged.

- `BoundedSource::new(inner, limit)` caps concurrent `inner.fetch` calls at
  `limit`. A `limit` of 0 sheds every fetch (a deliberate "serve only what is
  local" setting); the daemon never defaults to 0.
- The slot accounting is lock-free and exact: at most `limit` fetches are ever
  in `inner.fetch` at once, a shed never consumes a slot, and a slot is released
  when the fetch returns regardless of outcome (including a panic-free early
  return on an inner error).

### The shed signal

A new error, `Error::Unavailable`, is what a saturated tier returns. It carries
a short reason (for logs and tests), and it is distinct from `Error::Upstream`
(a tier-above failure relayed over the wire): a shed is this tier declining
work, not the tier above failing.

### The shed crosses the wire

The framed transport gains a response tag, `TAG_UNAVAILABLE`, whose body is the
UTF-8 reason. `serve_connection` maps `Err(Error::Unavailable)` to it;
`SocketSource::fetch` maps it back to `Error::Unavailable`. So when a daemon
sheds, the daemon dialing it as its upstream receives `Unavailable` — not a
generic upstream error — and, because that daemon's own upstream is itself a
`BoundedSource`, the `Unavailable` propagates out of its `get` and is re-shed to
*its* caller. A shed at one tier becomes a shed at the tier below, with no
retry and no stampede. Load shedding needs no new negotiation beyond the tag:
existing peers that never shed simply never send it.

### Configuration

A new option, with a flag and an environment variable like every other
([`config`](../../crates/openscion-daemon/src/config/mod.rs)):

- `--upstream-concurrency <N>` / `OPENSCION_DAEMON_UPSTREAM_CONCURRENCY`, the
  maximum concurrent upstream fetches, defaulting to
  `DEFAULT_UPSTREAM_CONCURRENCY` (a conservative bound well under the inbound
  connection count, so the node protects the tier above rather than mirroring
  its own load onto it).

`Daemon::bind` wraps the configured upstream in a `BoundedSource` with this
limit before opening the node store.

## Out of scope

- Inbound (serve-side) concurrency limits — the bound here is on *upstream*
  concurrency, the architecture's stated lever. Thread-per-connection already
  bounds inbound work by the OS; a serve-side admission limit is a later tuning.
- Queueing, backpressure signaling, or retry/backoff policy. Shedding is
  immediate and the caller decides; this chunk adds no waiting.
- The regional cache tier, the distributed metadata store, and the propagation
  bus — later phase-7 chunks.

## Acceptance

- Within the limit, `BoundedSource` is transparent: it returns exactly what the
  inner source returns (bytes, absence, or error), releasing the slot each time.
- At the limit, a further fetch returns `Error::Unavailable` immediately without
  calling the inner source, and a freed slot lets the next fetch through.
- At most `limit` inner fetches run concurrently, proven with blocked fetches
  held across threads.
- A daemon whose upstream is saturated answers a client with `Unavailable` over
  the wire, and a `SocketSource` surfaces it as `Error::Unavailable`, distinct
  from `Error::Upstream`.
- `--upstream-concurrency` / `OPENSCION_DAEMON_UPSTREAM_CONCURRENCY` sets the
  bound; the default is `DEFAULT_UPSTREAM_CONCURRENCY`.
- The four contract commands are green, every changed source file keeps ≥90%
  line coverage, and the supply-chain gate passes (no new dependency).

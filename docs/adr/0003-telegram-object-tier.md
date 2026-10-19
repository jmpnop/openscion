# 3. Telegram as a cold object tier, with ref CAS by ordered-log replay

- **Status:** Proposed
- **Date:** 2026-10-18

## Context

A technical question: can Telegram/MTProto back OpenScion's storage? This ADR
records the design analysis, independently of any terms-of-service question,
which is out of scope here.

Telegram exposes two relevant primitives: documents in a channel (files up to
2 GiB, ranged `download.getFile`), and a strictly monotonic, server-assigned
`message_id` per channel — a total order over appends.

## Decision

Treat Telegram, if adopted, as an **optional cold `ObjectStore` tier only**,
behind the node daemon and regional cache, never on the hot serving path and
never as the `MetadataStore`.

- **Objects as documents.** Each content-addressed object is a document; large
  blobs use content-defined chunking into per-chunk objects with a manifest,
  which sidesteps the size cap, dedups, and lets a sparse read fetch only the
  chunks it names. Every object is BLAKE3-verified on receipt, so the untrusted,
  eventually-consistent medium cannot corrupt state silently.
- **Index as a log.** The `id -> message_id` map is an append-only log posted as
  messages; `message_id` is the LSN. Cold boot reads the latest compacted
  snapshot document and replays the tail. The log stores the stable
  `message_id`, never the volatile `file_reference`, which is re-resolved lazily
  at read time.
- **Ref CAS by ordered-log replay.** Compare-and-swap does **not** require an
  external store: model refs as a state machine whose updates are proposals
  appended to a dedicated channel. A proposal commits iff, at its position in
  the total order, the ref's committed value equals its `expected`. Every reader
  folds the same order to the same decision, so `message_id` is the atomic
  arbiter and the land loop's retry-on-conflict falls out directly.

## Consequences

- A fully Telegram-backed deployment is technically coherent: immutable objects
  plus a messaging-level consensus log for refs and landing.
- It is correct only under append-only discipline (an admin deleting a log
  message corrupts replay), with periodic snapshot/compaction to bound replay
  cost, and at low land throughput (each CAS is a network round trip). Object
  reads never go through the log; they are served from caches and verified by
  hash.
- Telegram's per-channel ordering guarantee becomes the consistency root. This
  remains `Proposed`: no implementation is committed, and the earlier assumption
  that CAS must live outside Telegram is revised by the ordered-log construction.

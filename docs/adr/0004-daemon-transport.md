# 4. Daemon and tier transport: framed openscion-proto over a stream

- **Status:** Accepted
- **Date:** 2026-10-19

## Context

The architecture ([delivery phase 5](../specs/openscion-architecture.md)) adds a
per-machine daemon and a later regional cache. A client reaches the daemon over
a local Unix socket; the daemon reaches the cache, and the cache the server,
between machines. The architecture's open question names three candidate
transports: gRPC, a plain HTTP API, or a framed protocol carrying the
`openscion-proto` payloads directly. The transport must work over both a local
Unix socket and TCP, and all tiers answer the *same* object RPCs with the same
semantics.

A load-bearing rule of the workspace: `openscion-proto` is the single set of
payload types, re-exported by `openscion-core`, so `openscion_core::ObjectId`
and `openscion_proto::ObjectId` are the same type. A parallel set of payload
types for a transport would reintroduce a conversion at every call site that
nothing checks. CI forbids `openscion-proto` from depending on a transport, an
async runtime, an HTTP client, or a native library.

## Decision

Use a **length-framed binary protocol that carries the `openscion-proto`
payloads directly**, spoken over any byte stream — a Unix socket locally, TCP
between machines.

- **One wire form, one type set.** Each message is a length prefix plus a
  serialized `openscion-proto` value. The daemon, cache, and server speak the
  same frames; there is no second schema and no payload conversion. This extends
  the length-framing the engine already uses to stream objects
  (`FrameWriter` / `decode_frames`).
- **Uniform object RPCs.** The framed request/response set is the object RPCs
  (`get` by id, batched fetch) plus the forwarded mutable queries, identical at
  every tier, so a caller talks to whichever tier is nearest without knowing the
  topology.
- **Transport stays out of `openscion-proto`.** The framing, the socket/TCP
  listener, and the client live in the daemon and (later) cache/server transport
  layers, never in the protocol crate.

Rejected alternatives:

- **gRPC (tonic/prost).** Introduces a second, generated set of payload types
  alongside `openscion-proto` — exactly the conversion-at-every-call-site the
  workspace rule forbids — plus `tonic`/`prost`/protobuf codegen in crates
  published to crates.io, for streaming and cross-language reach a single-client
  Rust workspace does not need.
- **HTTP.** Workable for the inter-machine edge, but `ureq` (the existing
  blocking client) is TCP-only and does not speak Unix sockets, so the local
  edge would need a second client, and HTTP framing adds overhead on the
  high-volume local path where it buys nothing.

## Consequences

- The daemon is built in sub-chunks. The **node-local object store and snapshot
  boot** ([spec](../specs/openscion-daemon-node-store.md)) come first behind an
  in-process API, with an `ObjectSource` trait standing in for the tier above,
  so no wire format is frozen until an RPC actually crosses a socket.
- The framed transport is then implemented over `ObjectSource`/the store: a
  stream codec (length-prefixed `openscion-proto` frames), a Unix-socket
  listener for clients, and a client the daemon uses to reach the tier above.
- Because the frame body is an `openscion-proto` payload, adding an RPC is
  adding a payload type to the protocol crate and a match arm to the dispatch —
  no schema duplication.
- Strict tiering (each tier calls only the one above, bounds upstream
  concurrency, and sheds load with `Unavailable` rather than queueing) is
  enforced in the transport layer when it lands; this ADR fixes the wire form,
  not the concurrency policy.

# Daemon framed transport

- **Status:** Accepted
- **Owner:** OpenScion daemon
- **Specifications it builds on:**
  [architecture](openscion-architecture.md) (phase 5; uniform object RPCs and
  strict tiering), [node-local store](openscion-daemon-node-store.md) (what the
  transport serves).
- **Decision record:** [ADR 0004](../adr/0004-daemon-transport.md) — the wire is
  framed `openscion-proto` over a stream.

## Problem

The [node store](openscion-daemon-node-store.md) answers object reads behind an
in-process `ObjectSource`. For the daemon to be a daemon, those reads must cross
a process boundary: a client on the same machine reaches it over a Unix socket,
and the daemon reaches the tier above between machines over TCP — the same
object RPCs at every tier, so a caller talks to whichever tier is nearest. The
wire must move raw object bytes without a text encoding, and a store on one tier
must be able to use the tier above through exactly the same `ObjectSource` it
uses in-process.

## Goals and non-goals

### Goals

- A length-framed codec over any byte stream carrying the object-get RPC: a
  request names an id, a response is the object's raw bytes, "absent", or an
  error.
- A server that answers those RPCs from a `NodeStore` over a connection, and
  accept loops that serve a Unix-socket and a TCP listener, each connection on
  its own thread.
- A `SocketSource` that implements `ObjectSource` by dialing a daemon, so a node
  store composes over the wire: `NodeStore` → `SocketSource` → daemon →
  `NodeStore` → `SocketSource` → … up the tiers.
- Platform independence for the codec and the server logic (generic over the
  stream); only the Unix socket is Unix-gated.

### Non-goals

- Batched fetch, and forwarding mutable queries (refs, log, derived data). The
  object-get RPC is the tiering primitive; the batched and mutable RPCs extend
  the same frame set later.
- Upstream-concurrency bounds and `Unavailable` load shedding under saturation —
  the network-tier phase.
- A connection pool. Each `SocketSource::fetch` opens a fresh connection; a
  served connection itself handles any number of sequential requests, so pooling
  is an additive optimization.
- The daemon binary (a `main` that binds the socket and boots from a snapshot);
  this delivers the library the binary will call.
- TLS or authentication; the local Unix socket and a private network are the
  supported setup, as for the server today.

## Proposed behaviour

### Frame

Every message is a one-byte **tag**, an eight-byte big-endian **body length**,
and that many **body** bytes — the length-framing the engine already uses to
stream objects, over a socket:

| Message | Tag | Body |
| --- | --- | --- |
| Get request | 1 | the 32-byte object id |
| Found response | 2 | the object's raw bytes |
| Absent response | 3 | empty |
| Error response | 4 | a UTF-8 message |

A body longer than a fixed maximum (4 GiB — objects beyond it are chunked by the
data model) is refused before it is allocated, so a peer cannot demand unbounded
memory. A stream that ends cleanly between frames is a normal close; a stream
that ends mid-frame is a protocol error.

### Server

- `serve_connection(stream, store)` reads a get request, answers it from the
  store, and loops until the peer closes the connection — so one connection
  carries many requests. A store hit is a found response, a `NotFound` is an
  absent response, and any other store error (a corrupt object, an I/O failure)
  is an error response carrying the message.
- `serve_tcp(listener, store)` and `serve_unix(listener, store)` accept
  connections and serve each on its own thread from a shared store, until
  accepting fails.

### Client

`SocketSource::tcp(addr)` / `SocketSource::unix(path)` build a client.
`ObjectSource::fetch(id)` dials the daemon, sends a get request, and maps the
reply: found → the bytes, absent → `None`, error → an upstream error, anything
else or an early close → a protocol error, a failed dial → a transport error.
Because it is an `ObjectSource`, a `NodeStore` can take it as its tier above.

## Invariants and constraints

- **Raw object bytes on the wire.** A found response carries the object bytes
  verbatim; no base64 or JSON. Integrity is still enforced where an object is
  ingested: the receiving node store verifies fetched bytes against the id.
- **Uniform and composable.** The client is an `ObjectSource`, so the tiers
  compose over the wire with no new concept, and the served RPC is the same
  `get` at daemon, cache, and server.
- **Bounded frames.** The codec refuses an over-maximum body before allocating,
  and treats a mid-frame end as a protocol error rather than hang.
- **Transport-independent core.** The codec, `serve_connection`, and the client
  request logic are generic over `Read + Write`; the Unix socket is the only
  `#[cfg(unix)]` surface, and TCP works everywhere.
- **A connection is sequential.** One request, one response, in order, per
  connection; concurrency is across connections (a thread each), not pipelined
  within one.

## Acceptance criteria

- A client fetches an object the daemon holds and gets its exact bytes, over
  both TCP and a Unix socket; an id no tier holds comes back absent.
- A node store configured with a `SocketSource` upstream reads an object through
  a daemon over the wire and then serves it locally.
- A store error at the daemon reaches the client as an upstream error.
- The codec round-trips frames, reports a truncated frame and an over-maximum
  frame as protocol errors, and reads a clean end of stream as "no more".
- A refused dial is a transport error; an unexpected response tag or a peer that
  closes before replying is a protocol error.

## Open questions

- **Connection pooling / multiplexing** to avoid a dial per fetch, once the call
  volume warrants it.
- **Batched fetch** (many ids in one round trip), mirroring the server's
  `POST /v1/objects`, as the next frame type.
- **Backpressure and shedding.** Bounding in-flight connections and returning an
  `Unavailable`-style response under load belongs with the network-tier phase.

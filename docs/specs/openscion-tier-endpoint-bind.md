# `Endpoint::bind`: one listener for every tier

- **Status:** Accepted
- **Owner:** OpenScion maintainers

## Problem

The [`openscion-tier`](openscion-tier.md) `Endpoint` already turns a
scheme-prefixed `tcp:<addr>`/`unix:<path>` into the client side of the wire with
[`dial`](openscion-tier.md) (`Endpoint::dial -> SocketSource`). The *server*
side has no counterpart, so each tier that listens — the daemon, the regional
cache, and now the server's framed object endpoint — carries its own private
copy of the same three things:

- a `Listener` enum wrapping `TcpListener`/`#[cfg(unix)] UnixListener`,
- a `bind_listener(&Endpoint)` that binds it and formats the resolved
  `tcp:<addr>`/`unix:<path>` string, and
- a `serve` match that dispatches to `serve_tcp`/`serve_unix`.

That is three identical copies of the listener plumbing and its `#[cfg(unix)]`
handling, drifting apart one bug fix at a time. The asymmetry is the tell: the
tier owns `dial` but not `bind`, though both are the tier's wire.

## Goals

- `Endpoint::bind(&self)` yields a bound listener over whichever transport the
  endpoint names, symmetric with `Endpoint::dial`.
- A `BoundListener` that reports its resolved `tcp:<addr>`/`unix:<path>` address
  and serves an `ObjectService` over the framed transport, hiding the
  `#[cfg(unix)]` dispatch in one place.
- The daemon, the cache, and the server drop their private `Listener`,
  `bind_listener`, and serve-dispatch in favour of it, with no change in
  behaviour.

## Non-goals

- Any change to the wire form, the framed transport, or the serving semantics —
  `BoundListener::serve` is the existing `serve_tcp`/`serve_unix` behind one
  method.
- Any change to how a bind failure is reported to the operator: each tier keeps
  wrapping the bind's `io::Error` in its own domain error.
- Graceful-shutdown behaviour: the accept loop still blocks for the life of the
  process, as it does today.

## Proposed behavior

`Endpoint::bind` returns a bound listener or the bind's `io::Error`, so each
caller wraps the failure in its own error type exactly as it does now — the
daemon and cache in their `Error::Transport`, the server in its
`Error::ObjectListen`:

```rust
/// The server side of an endpoint: a bound listener ready to serve.
#[derive(Debug)]
pub struct BoundListener { /* a TCP or Unix listener, and its resolved address */ }

impl Endpoint {
    /// Binds a listener on this endpoint.
    ///
    /// # Errors
    ///
    /// Returns the bind's `io::Error`, which the caller wraps in its own error.
    pub fn bind(&self) -> std::io::Result<BoundListener>;
}

impl BoundListener {
    /// The resolved `tcp:<addr>`/`unix:<path>` this is listening on — for
    /// `tcp:…:0`, the port the OS chose.
    pub fn listening(&self) -> &str;

    /// Serves `service` on this listener, each connection on its own thread,
    /// until accepting fails. Dispatches to the TCP or Unix transport.
    pub fn serve<Svc: ObjectService + 'static>(&self, service: &Arc<Svc>) -> Result<()>;
}
```

`listening()` is computed once at bind time — for a TCP endpoint it resolves any
port-0 to the chosen port; for a Unix endpoint it echoes the path — so a tier
reports where it actually listens. `serve` is `serve_tcp`/`serve_unix` chosen by
the bound transport, so a tier's `serve` becomes
`thread::spawn(move || { let _ = bound.serve(&service); })` with no `#[cfg]` of
its own.

The daemon, the cache, and the server each replace their `Listener` field with a
`BoundListener`, their `bind_listener` call with `endpoint.bind()`, their
`listening` string with `bound.listening()`, and their serve match with
`bound.serve(&service)`. Each tier's own behaviour — warm boot and snapshot
persistence in the daemon, nothing-to-persist in the cache, the framed endpoint
beside HTTP in the server — is untouched.

## Invariants and constraints

- Behaviour-preserving: every tier binds, reports, serves, and fails exactly as
  before; only the duplicated plumbing moves into the tier.
- `Endpoint::bind` performs only the bind; it adds no tier-`Error` variant, since
  binding is an I/O operation each caller already classifies itself.
- The `#[cfg(unix)]` split lives in exactly one place, the tier, rather than
  three.
- No new third-party dependency; `unsafe` remains forbidden.

## Acceptance criteria

- `Endpoint::bind` binds a TCP endpoint and a Unix endpoint, `listening()`
  reports the resolved address in each case, and a bind on a taken address or
  path is an `io::Error`.
- A `BoundListener` serves an `ObjectService` over the framed wire: a
  `SocketSource` fetch returns a stored object and *absent* for a miss, over both
  TCP and Unix.
- The daemon, cache, and server suites — including their binary integration
  tests — stay green, with their private `Listener`/`bind_listener` removed.
- The four contract commands are green, every source file keeps at least 90%
  line coverage, and `cargo deny` passes.

## Open questions

None. The change is the symmetric counterpart to the accepted `Endpoint::dial`
and is behaviour-preserving by construction.

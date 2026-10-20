# Plan: daemon framed transport

- **Status:** Done
- **Specification:**
  [`../specs/openscion-daemon-transport.md`](../specs/openscion-daemon-transport.md)
- **Decision record:** [ADR 0004](../adr/0004-daemon-transport.md).
- **Phase:** 5 (client and daemon), fifth chunk — concluding the phase.

## Goal

Carry the daemon's object RPCs over a byte stream: a length-framed codec, a
server that answers from a `NodeStore` over Unix and TCP listeners, and a
`SocketSource` client that is itself an `ObjectSource`, so tiers compose over
the wire.

## Approach

A `transport` module in `openscion-daemon`. The codec is two free functions over
`Read`/`Write` — `write_frame(tag, body)` and `read_frame -> Option<(tag,
body)>` — framing a one-byte tag, an eight-byte length, and the body, bounded by
a maximum and treating a mid-frame end as a protocol error. `serve_connection`
loops reading a get request and writing the store's answer (found / absent /
error). `serve_tcp` and `serve_unix` accept and spawn a thread per connection
from a shared `Arc<NodeStore>`. `SocketSource` holds an endpoint (TCP addr or
Unix path), dials per fetch over a `Conn` enum that delegates `Read`/`Write` to
the chosen stream, and maps the reply. The codec and server are generic over the
stream; the Unix socket is `#[cfg(unix)]`.

## Tasks

Test-first: the failing test, then the code that passes it.

1. **Errors** — `src/error/mod.rs`: `Protocol(String)`, `Upstream(String)`,
   `Transport(std::io::Error)`.
2. **Test source** — `src/test_support/mod.rs`: a reusable `MemorySource`
   (`ObjectSource` over a map, with `add` / `add_corrupt`).
3. **Codec** — `src/transport/mod.rs`: tags, `MAX_BODY`, `write_frame`,
   `read_frame`, `read_exact`, `id_from`. Test round-trip, clean end, truncated,
   oversized.
4. **Server** — same file: `serve_connection`, `serve_tcp`, `serve_unix`. Test
   found/absent over TCP and (`#[cfg(unix)]`) a Unix socket, and a relayed
   upstream error, with a `NodeStore<MemorySource>` behind a loopback listener.
5. **Client** — same file: `SocketSource` (`tcp` / `unix`), `Endpoint`, `Conn`
   (Read/Write delegation), `dial`, and the `ObjectSource` impl. Test a node
   store fronting a daemon over the wire, a refused dial (transport error), an
   unexpected tag and an early close (protocol errors).
6. **Surface** — `lib.rs` declares `mod transport` and re-exports
   `serve_connection`, `serve_tcp`, `SocketSource`, and (`#[cfg(unix)]`)
   `serve_unix`; crate doc updated.
7. **Docs** — spec/plan/ADR indexed; `ROADMAP.md`.

## Verification

- Focused: `cargo test -p openscion-daemon transport::`.
- Full contract: `cargo fmt --all -- --check`, `cargo clippy --all-targets
  --all-features -- -D warnings`, `cargo build --all-targets --all-features`,
  `cargo test --all-features`.
- Rustdoc: `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps --all-features`.
- Per-file coverage ≥90% on every touched source file.

## Checklist

- [x] framed codec: round-trip, bounded, truncated/oversized rejected
- [x] server answers found / absent / error from a `NodeStore`
- [x] `SocketSource` is an `ObjectSource`; a node store fronts a daemon
- [x] served over both TCP and a Unix socket; Unix is `#[cfg(unix)]`
- [x] refused dial, unexpected tag, early close are the right errors
- [x] fmt, clippy, build, full test suite, rustdoc green
- [x] per-file coverage ≥90%

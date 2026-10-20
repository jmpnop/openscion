# Daemon binary

- **Status:** Accepted
- **Owner:** OpenScion daemon
- **Specifications it builds on:**
  [architecture](openscion-architecture.md) (phase 5; one daemon per node,
  strict tiering), [node-local store](openscion-daemon-node-store.md) (the
  bounded read-through store and its snapshot/warm), [framed
  transport](openscion-daemon-transport.md) (the object RPCs over Unix and TCP).
- **Decision record:** [ADR 0004](../adr/0004-daemon-transport.md) — the wire,
  already decided; this spec adds no new wire decision.

## Problem

The daemon library is complete: a [`NodeStore`](openscion-daemon-node-store.md)
answers object reads behind an `ObjectSource`, and the [framed
transport](openscion-daemon-transport.md) serves those reads over a Unix socket
or TCP and dials a tier above over the same wire. What is missing is the process
itself — a `main` that an operator or a service manager runs: it must read its
configuration, open the store against a data directory and (optionally) a tier
above, start *warm* from a saved set instead of cold, bind its listener, serve,
and on a clean stop persist the warm set so the next start is warm again.

This is the piece the transport spec deferred: "the daemon binary (a `main` that
binds the socket and boots from a snapshot)". It delivers no new protocol and no
new store behaviour — only the wiring, the configuration surface, and the
snapshot file format that make the library a runnable node daemon.

## Goals and non-goals

### Goals

- A `openscion-daemon` binary configured from flags and `OPENSCION_*`
  environment variables, so the same binary runs unchanged from a shell, a
  container, or a service manager — matching `openscion-server`.
- A listener chosen by the configuration: a Unix socket for local jobs or a TCP
  address between machines, named by a scheme-prefixed value (`unix:/path`,
  `tcp:127.0.0.1:7979`).
- An optional tier above, named the same way; with none, the daemon is a *leaf*
  that serves only what it already holds and reports every miss as absent.
- Boot from a snapshot: on start, read the object ids in a snapshot file and
  [`warm`](openscion-daemon-node-store.md) the store from the tier above; a
  missing snapshot file is an empty warm set, not an error.
- Persist the snapshot on a clean shutdown (`SIGINT`/`SIGTERM`): write the
  store's current [`snapshot`](openscion-daemon-node-store.md) — most-recently
  used first — back to the file, so the next start warms from real recent
  traffic.
- A thin `main` that parses the configuration, runs the daemon, and maps a
  startup failure to a non-zero exit with a one-line `openscion-daemon: …`
  message — again matching the server.

### Non-goals

- Any change to the wire protocol, the store's eviction or verification, or the
  set of RPCs. This is wiring only.
- Upstream-concurrency bounds, `Unavailable` load shedding, a regional cache
  tier, and the distributed `MetadataStore` — the phase-7 network-tier work.
- A connection pool or batched fetch — additive, and already out of scope for
  the transport.
- Periodic or continuous snapshotting. The snapshot is written once, at a clean
  shutdown; a crash loses at most the warmth since the last clean stop, which a
  cache tolerates.
- Serving mutable queries (refs, log, derived data). The daemon forwards those
  upstream unchanged; it is an object cache, not a second source of truth.
- TLS or authentication; the local Unix socket and a private network are the
  supported setup, as for the server today.

## Proposed behaviour

### Configuration

`DaemonConfig` is parsed with `clap`. Every option has a flag and an
`OPENSCION_*` environment variable.

| Flag | Env | Meaning | Default |
| --- | --- | --- | --- |
| `--listen <endpoint>` | `OPENSCION_DAEMON_LISTEN` | Where to accept connections. | `tcp:127.0.0.1:7979` |
| `--data-dir <dir>` | `OPENSCION_DAEMON_DATA_DIR` | The cache directory the store reads and writes. | required |
| `--budget-bytes <n>` | `OPENSCION_DAEMON_BUDGET_BYTES` | The resident byte budget. | `1073741824` (1 GiB) |
| `--upstream <endpoint>` | `OPENSCION_DAEMON_UPSTREAM` | The tier above to read through, if any. | none (a leaf) |
| `--snapshot <file>` | `OPENSCION_DAEMON_SNAPSHOT` | The warm-set file to boot from and persist to. | none |

An **endpoint** is a scheme-prefixed string: `tcp:<socket-addr>` (for example
`tcp:127.0.0.1:7979` or `tcp:[::1]:7979`) or `unix:<path>`. Any other form, a
TCP value whose address does not parse, or an empty Unix path is a configuration
error reported before the daemon starts. The default listen address is loopback
only: exposing the daemon beyond the machine is a deliberate `--listen` choice,
never a default, as with the server.

The listen default is TCP so the binary runs with only `--data-dir`; a Unix
socket, the usual local-job endpoint, is one `--listen unix:/…` away.

### Boot

On start, the daemon:

1. builds the tier above — a `SocketSource` dialing `--upstream`, or a detached
   source that holds nothing when `--upstream` is absent;
2. opens the `NodeStore` on `--data-dir` against that source within
   `--budget-bytes`;
3. if `--snapshot` names a file that exists, reads its object ids and warms the
   store from the tier above, counting objects fetched and objects no tier held;
   a missing file is an empty warm set;
4. binds the listener named by `--listen`;
5. prints one line to stdout naming where it listens, in the form
   `openscion-daemon listening on <endpoint>` (for TCP, the resolved address,
   so `tcp:127.0.0.1:0` reports the chosen port);
6. serves connections, each on its own thread, until a shutdown signal.

A leaf daemon (no `--upstream`) with a snapshot still boots: warming fetches
nothing, so the warm set is empty and every object it does not already hold on
disk reads as absent.

### Snapshot file

The snapshot file is a UTF-8 text file, one lowercase hex object id per line, in
most-recently-used-first order — the order
[`snapshot`](openscion-daemon-node-store.md) returns. Blank lines and lines
beginning with `#` are ignored on read, so the file stays inspectable and
hand-editable. A line that is not a blank, a comment, or a 64-character hex id is
a read error naming the file, so a corrupt snapshot fails the boot loudly rather
than silently dropping ids. Reading a file that does not exist yields the empty
set.

### Shutdown

On `SIGINT` or `SIGTERM`, the daemon stops serving and, if `--snapshot` was
given, writes the store's current snapshot to that file (replacing it
atomically — a temporary file in the same directory, then a rename), then exits
successfully. A daemon started without `--snapshot` simply exits. Signal-driven
shutdown is Unix-only, matching the daemon's Unix-socket support and the
server's `SIGTERM` behaviour; the portable boot, store, and transport logic is
not gated.

### Errors

Startup failures surface as the daemon's existing `Error` and, in `main`, as a
`openscion-daemon: <error>` line on stderr with a failure exit code:

- a malformed `--listen`/`--upstream` endpoint, or an unparseable TCP address;
- `--data-dir` cannot be created or read (`Error::Io`);
- the listener's address is already in use, or a Unix socket path cannot be
  bound (`Error::Transport`);
- a corrupt snapshot file (`Error::Io` naming the file);
- the tier above cannot be reached during warm (`Error::Transport`/`Upstream`).

## Invariants and constraints

- **Wiring only.** The binary adds no RPC, no wire change, and no new store
  behaviour; it composes `NodeStore`, the transport, and a snapshot file.
- **Tokio-free.** The daemon keeps the library's blocking, thread-per-connection
  model ([ADR 0004](../adr/0004-daemon-transport.md)); the binary adds no async
  runtime. Signal handling is a lightweight synchronous facility, not a runtime.
- **A snapshot round-trips.** Ids written by a shutdown are read back exactly and
  in the same order by the next boot.
- **A missing snapshot is normal.** First boot, or a daemon with no `--snapshot`,
  is not an error.
- **Loopback by default.** The default listen endpoint is loopback; reaching
  beyond the machine is an explicit choice.
- **Determinism in tests.** Behaviour is proven without sleeps: in-process tests
  cover configuration, endpoint parsing, the snapshot codec, and boot-warm
  against a fake tier above; a spawned-binary test covers bind, serve, a client
  request, and a signalled shutdown that writes the snapshot.

## Acceptance criteria

- `openscion-daemon --data-dir <dir>` binds the default loopback TCP endpoint,
  prints its resolved listen line, and answers object-get requests from a client
  over the wire.
- `--listen unix:<path>` serves the same RPCs over a Unix socket.
- `--upstream <endpoint>` makes the daemon read through that tier above for
  objects it does not hold; without it, misses read as absent.
- With `--snapshot <file>`, a boot warms from the file's ids, and a
  `SIGINT`/`SIGTERM` writes the current snapshot back; the ids round-trip across
  a restart.
- A malformed endpoint, an unparseable TCP address, an unreadable `--data-dir`,
  a bound-address conflict, and a corrupt snapshot each fail with a
  `openscion-daemon: …` message and a non-zero exit.
- The four CI commands pass, every new source file keeps at least 90% line
  coverage, and `cargo deny check` stays green.

## Open questions

None. The wire and store decisions are settled; this spec only wires them into a
process and fixes the snapshot file format.

# Daemon binary — implementation plan

Builds the [daemon binary spec](../specs/openscion-daemon-binary.md): a
`openscion-daemon` process that configures a [`NodeStore`](../specs/openscion-daemon-node-store.md),
boots warm from a snapshot file, binds a Unix or TCP listener using the [framed
transport](../specs/openscion-daemon-transport.md), serves, and persists the
snapshot on a clean shutdown. Wiring only — no new wire or store behaviour.

## Goal, non-goals, assumptions

- **Goal:** a runnable, configured node daemon with warm boot and warm-on-stop.
- **Non-goals:** network-tier concurrency bounds and load shedding, a regional
  cache tier, the distributed `MetadataStore`, batched/mutable RPCs, a
  connection pool, periodic snapshotting, TLS/auth. (Phase 7 and later.)
- **Assumptions:** the library's `NodeStore`, `serve_tcp`/`serve_unix`,
  `SocketSource`, and `snapshot`/`warm` are in place and unchanged; the binary
  keeps the blocking, thread-per-connection, tokio-free model.

## Modules and files

```
crates/openscion-daemon/
├── Cargo.toml                       # + clap, signal-hook; a [[bin]] stays implicit (src/main.rs)
├── src/
│   ├── lib.rs                       # wire new modules; export DaemonConfig, Endpoint, Daemon
│   ├── config/
│   │   ├── mod.rs                   # DaemonConfig (clap Parser), DEFAULT_LISTEN
│   │   ├── types.rs                 # Endpoint enum + FromStr + Display + EndpointParseError
│   │   └── test.rs
│   ├── snapshot/
│   │   ├── mod.rs                   # read_ids / write_ids (hex lines, atomic replace)
│   │   └── test.rs
│   ├── daemon/
│   │   ├── mod.rs                   # Upstream (ObjectSource), Listener, Daemon::bind/listening/warmed/serve
│   │   └── test.rs
│   └── main.rs                      # thin: parse, run, map error to ExitCode
└── tests/
    └── binary.rs                    # spawn the real binary: serve + client + SIGTERM + snapshot
```

## Dependencies

Add to the root `[workspace.dependencies]` with why-comments, then take them in
`openscion-daemon/Cargo.toml` with `{ workspace = true }`:

- `clap` (already present) — the binary's flag/env configuration, as the server.
- `signal-hook` — synchronous `SIGINT`/`SIGTERM` handling for graceful shutdown
  without pulling in an async runtime; the standard library has no signal
  primitive and `unsafe` is forbidden workspace-wide, so a maintained,
  permissively licensed (MIT/Apache-2.0) crate is the first-party choice. Unix
  only, matching the daemon's Unix-socket support.

`cargo deny check` must stay green after the lockfile updates.

## Ordered, test-first tasks

1. **Endpoint parsing.** Test first in `config/test.rs`: `"tcp:127.0.0.1:7979"`
   and `"tcp:[::1]:7979"` parse to `Endpoint::Tcp`; `"unix:/run/d.sock"` to
   `Endpoint::Unix` (Unix); `""`, `"http:x"`, `"tcp:not-an-addr"`, and
   `"unix:"` (empty path) are errors; `Display` round-trips each scheme. Then
   implement `Endpoint` and `EndpointParseError` in `config/types.rs`.

2. **Configuration.** Test first: `DaemonConfig::try_parse_from` with only
   `--data-dir` yields the default listen (`DEFAULT_LISTEN`), 1 GiB budget, no
   upstream, no snapshot; `--listen`, `--upstream`, `--budget-bytes`, and
   `--snapshot` are read; the `OPENSCION_DAEMON_*` env vars back each flag;
   `--listen bogus` is a parse error. Then implement `DaemonConfig` in
   `config/mod.rs` (`#[non_exhaustive]`, `clap::Parser`, a `new(data_dir)`
   builder and `with_*` setters for tests), and `DEFAULT_LISTEN`.

3. **Snapshot codec.** Test first in `snapshot/test.rs`: `write_ids` then
   `read_ids` round-trips a list in order; a missing file reads as empty; blank
   and `#`-comment lines are ignored; a non-hex / wrong-length line is an
   `Error::Io` naming the file; the write replaces an existing file atomically.
   Then implement `read_ids`/`write_ids` in `snapshot/mod.rs` (lowercase hex via
   `ObjectId` `Display`/`FromStr`, temp-file + rename).

4. **Upstream source.** Test first in `daemon/test.rs`: `Upstream::Detached`
   fetches `Ok(None)` for any id; `Upstream::Socket` fetches an object from an
   in-process served `NodeStore` (stand one up with `serve_tcp` on a thread over
   a `MemorySource` holding an object). Then implement the `Upstream` enum and
   its `ObjectSource` impl in `daemon/mod.rs`, and `Upstream::from(Option<&Endpoint>)`.

5. **Bind and warm.** Test first: stand up an in-process upstream daemon over
   TCP holding two objects; write a snapshot file naming both ids;
   `Daemon::bind` with `--upstream`, `--snapshot`, and a temp `--data-dir`
   reports `warmed().fetched == 2`; `listening()` returns `tcp:<resolved addr>`
   for `tcp:127.0.0.1:0`; a leaf (`--upstream` absent) with the same snapshot
   reports `fetched == 0`. Then implement `Endpoint::bind -> Listener`,
   `Daemon::bind`, `listening`, and `warmed`.

6. **Serve and shutdown.** Implement `Daemon::serve`: spawn the matching accept
   loop (`serve_tcp`/`serve_unix`) on a thread, then (Unix) block on a
   `signal-hook` `Signals` iterator for `SIGINT`/`SIGTERM`, persist the snapshot
   if configured, and return; non-Unix serves until the accept loop ends. No
   unit test (process-wide signals); covered by the spawned-binary test.

7. **Binary.** Implement `src/main.rs` mirroring `openscion-server`: parse
   `DaemonConfig`, `run` = `Daemon::bind` → print `openscion-daemon listening on
   <endpoint>` → `serve`, mapping an error to `eprintln!("openscion-daemon:
   {error}")` and `ExitCode::FAILURE`.

8. **Binary integration test** `tests/binary.rs` (`#![allow]` test lints):
   - Unix: spawn `--data-dir <tmp> --listen tcp:127.0.0.1:0 --snapshot <tmp>`,
     read the listening line, dial with `SocketSource::tcp`, fetch an absent id
     (→ `None`), `kill -TERM`, assert a clean exit and that the snapshot file
     now exists.
   - Failure: `--listen tcp:<address already bound>` exits non-zero with a
     `openscion-daemon: …` stderr line; `--listen bogus` exits non-zero.

9. **Docs and wiring.** Update `lib.rs` module docs and the crate example to
   mention the binary and snapshot boot; export `DaemonConfig`, `Endpoint`,
   `Daemon`. Mark the spec Implemented, add both docs to the spec/plan READMEs,
   and advance `ROADMAP.md` (daemon binary done; phase-7 network tiers remain).

## Verification

Focused while iterating:

```sh
cargo test -p openscion-daemon --all-features
cargo clippy -p openscion-daemon --all-targets --all-features -- -D warnings
```

Full contract and gates before the commit:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features
cargo test --all-features
env RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps --all-features
.github/scripts/check-file-coverage.sh 90 target/cov-daemon-binary-gate.json
cargo deny check
```

## Completion checklist

- [ ] Endpoint parse + Display, with error cases.
- [ ] `DaemonConfig` flags/env/defaults.
- [ ] Snapshot codec round-trip, missing, comment/blank, corrupt.
- [ ] `Upstream` detached and socket fetch.
- [ ] `Daemon::bind` warms from snapshot; `listening`/`warmed` correct.
- [ ] `serve` + signalled shutdown persists the snapshot (binary test).
- [ ] `main` maps failure to a `openscion-daemon: …` non-zero exit.
- [ ] Four CI commands green; every new file ≥90% coverage; `cargo deny` green.
- [ ] Spec marked Implemented; READMEs and ROADMAP updated.

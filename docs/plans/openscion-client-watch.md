# Plan: watcher-backed status

- **Status:** Done
- **Specification:**
  [`../specs/openscion-client-watch.md`](../specs/openscion-client-watch.md)
- **Phase:** 5 (client and daemon), second chunk.

## Goal

Add `opnscn watch`: a long-running command that renders a working copy's pending
changes and re-renders them as files change, re-examining only the paths an
event named. Its answer is always identical to a full `opnscn status`; the
watcher changes only the cost of reaching it.

## Approach

Correctness by construction, not by catching every event:

- The first render is a full scan (`commands::status`), so a copy that was
  already dirty is reported in full.
- Each later burst of events adds *candidate* paths; the next render re-examines
  only those, through the shared [`classify`](../specs/openscion-client-status.md),
  and carries every other path forward. A candidate that re-reads to the base
  drops off; a tracked candidate now absent is a deletion.
- Any event the watcher cannot attribute to specific files — an overflow
  (dropped events), a rename, a directory-level change, a path outside the root
  — escalates the next render to a full rescan. A missed or coarse event costs a
  scan, never a wrong answer.

The event-folding and rendering core is pure and synchronous (`WatchSession`,
`run`, `render`), driven by a channel of `WatchEvent`s, so it is tested with
synthetic events and no real filesystem. The operating-system watcher is a thin
adapter: `notify`'s recommended backend, a `translate` that turns a native event
into a `WatchEvent` (or ignores a `.opnscn` event), and a `spawn_fs_watcher`
that feeds the channel. `run` stops when the sender is dropped, so a test ends
it by dropping the channel; the real command runs until the process is
interrupted.

## Dependency

`notify` (workspace dependency, caret range), the maintained cross-platform
filesystem-watch crate (FSEvents, inotify, …). The standard library has no
filesystem-watch primitive, so this is the first-party choice. It enters the
published `openscion` CLI crate only.

## Tasks

Test-first: the failing test, then the code that passes it.

1. **Single-path scan** — `crates/openscion/src/workcopy/mod.rs`: factor the
   metadata→kind decision out of `walk` into `entry_kind`, and add
   `scan_one(root, path) -> Result<Option<Scanned>>` (a file/exec/symlink
   becomes `Some`; absent or a directory, `None`). Unit-test both.
2. **Session core** — `crates/openscion/src/watch/types.rs`: `WatchEvent`
   (`Changed(Vec<String>)`, `Rescan`) and `WatchSession` holding the baseline
   state, the carried-forward report, the candidate set, and a rescan flag, with
   `open`, `apply`, and `refresh`. Unit-test: first refresh is a full status;
   a `Changed` burst updates exactly the named paths; a touch (same bytes) drops
   a candidate; a tracked candidate gone is a deletion; `Rescan` recomputes in
   full and matches `commands::status`.
3. **Loop and render** — `crates/openscion/src/watch/mod.rs`: `render` (writes
   the status block plus a blank separator, then flushes) and `run` (initial
   render, then re-render per drained burst until the channel closes). Unit-test
   with a `mpsc` channel and a `Vec` writer, ending by dropping the sender.
4. **OS adapter** — same file: `relativize` (absolute path → root-relative
   `/`-path, or ignored/outside), `translate` (native event → `Option<WatchEvent>`),
   and `watch(root, out)` wiring `notify` into `run`. Unit-test `relativize` and
   `translate` with constructed `notify::Event`s; integration-test `watch`
   end-to-end in `crates/openscion/tests/watch.rs` by running the binary against
   a temp dir, creating a file, and polling its piped stdout for the change.
5. **Wire and re-export** — `commands/mod.rs` re-exports `StatusCode` (now used
   by `watch`); `cli/mod.rs` gains `Command::Watch { dir }` and dispatch;
   `lib.rs` declares the `watch` module.
6. **Docs** — module comments; `ROADMAP.md`; spec and plan indexed.

## Verification

- Focused: `cargo test -p openscion watch`,
  `cargo test -p openscion --lib workcopy`.
- Full contract: `cargo fmt --all -- --check`, `cargo clippy --all-targets
  --all-features -- -D warnings`, `cargo build --all-targets --all-features`,
  `cargo test --all-features`.
- Rustdoc: `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps --all-features`.
- Per-file coverage ≥90% on every touched source file.

## Checklist

- [x] first render is a full scan; later renders re-examine only named paths
- [x] an ambiguous or overflow event forces a full rescan
- [x] a render equals a full status over the same files (shared classifier)
- [x] pure core tested with synthetic events; OS adapter integration-tested
- [x] `watch` is read-only and makes no server request
- [x] fmt, clippy, build, full test suite, rustdoc green
- [x] per-file coverage ≥90%

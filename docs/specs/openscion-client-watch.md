# Watcher-backed status

- **Status:** Accepted
- **Owner:** OpenScion client
- **Specifications it builds on:**
  [architecture](openscion-architecture.md) (phase 5, the client),
  [working-copy status](openscion-client-status.md) (the one-shot report and
  the classifier both commands share).

## Problem

[`opnscn status`](openscion-client-status.md) walks the whole working copy on
every run. For an agent that edits a few files in an agent-scale monorepo, most
of that walk re-stats millions of unchanged files to find the handful that
moved. An agent that wants to *watch* its own pending change set — to see, as it
edits, what a commit would record — has to poll `status` in a loop, each poll
paying for a full scan.

A filesystem already knows which files changed. The operating system can tell a
long-running process the moment a file under a directory is written, created, or
removed. A watcher that listens to those events can keep a working copy's
pending-change set current by re-examining only the paths that actually moved,
instead of the whole tree.

The hazard is correctness. A watcher that misses an event, or that trusts an
event it should not, would show a change set that disagrees with the truth — the
very failure the one-shot `status` was designed to avoid. The watcher must be
unable to show a different answer than a full scan would; it may only reach that
answer faster.

## Goals and non-goals

### Goals

- A long-running `opnscn watch` that renders a working copy's pending changes
  and re-renders them as files change, re-examining only the paths an event
  named rather than rescanning the tree.
- An answer identical to a full `opnscn status`: the watcher narrows *which*
  paths it looks at, never *what* it concludes about them. It decides every path
  through the same classifier `status` and `commit` use, so it reports a file as
  modified only when its bytes or kind differ.
- Correctness that does not depend on catching every event perfectly. The first
  render is always a full scan, and any event the watcher cannot attribute to
  specific files forces a full rescan, so a missed or coarse event costs a scan,
  never a wrong answer.
- A deterministic, OS-independent core: the logic that folds events into a
  pending set and renders it is pure and testable without a real filesystem; the
  operating-system watcher is a thin adapter over it.

### Non-goals

- Accelerating the *one-shot* `opnscn status` command across separate process
  invocations. That needs a resident daemon holding the watch state between
  commands and a client-to-daemon channel; it is a later phase-5 chunk, and the
  daemon's transport is an open architecture question. `opnscn watch` keeps all
  of its state in one process and needs neither.
- A server connection. Like `status`, `watch` reads only local state and the
  files on disk.
- Persisting the watch state, a change journal, or a stat-cache refresh. `watch`
  is read-only toward the working copy and its `state.json`.
- Reacting to changes the working copy did not make to itself — concurrent
  commits on the server that move the ref. `watch` reports divergence of the
  files from their base, not from the server tip.

## Proposed behavior

### Command

```text
opnscn watch [--dir <path>]
```

`--dir` is the working-copy root and defaults to the current directory. `watch`
runs until interrupted (Ctrl-C), rendering the pending change set to standard
output: once at startup, then again after each burst of filesystem activity.

### Rendering

Each render writes the same block `opnscn status` would print — `A`, `M`, or `D`
followed by the path, one line per change, sorted by path, or the single line
`clean` — followed by one blank line that separates it from the next render. An
agent reading the stream sees a fresh, complete pending-change set per render,
delimited by blank lines.

### What a render reflects

- **The first render is a full scan.** It reports the working copy's complete
  pending change set against its base, exactly as `opnscn status` would, so a
  copy that was already dirty when `watch` started is reported in full.
- **Later renders re-examine only the paths events named.** When a burst of
  events names a set of paths, the next render classifies just those paths — a
  named path that now differs from the base is `A` or `M`, a named path that was
  tracked and is now gone is `D`, a named path that matches the base again drops
  off — and carries every other path's state forward unchanged.
- **An ambiguous event forces a full rescan.** If the operating system reports
  that it may have dropped events, or reports a change the watcher cannot
  attribute to specific files (a directory moved or removed, a rename it cannot
  resolve to both endpoints), the next render is a full scan. Correctness never
  rests on the event stream being complete.

The `.opnscn` state directory is ignored, as it is by `status` and the scan.

### Example

```console
$ opnscn watch
clean

A notes.txt

A notes.txt
M README.md

M README.md
```

The copy starts clean; a new `notes.txt` appears; `README.md` is edited; then
`notes.txt` is deleted before it was ever committed, so it drops off and only
the `README.md` edit remains.

## Invariants and constraints

- **A render equals a full status over the same files.** For any sequence of
  filesystem changes, what a render reports is exactly what `opnscn status`
  would report run against the working copy at that moment. The watcher changes
  the cost of reaching that answer, never the answer. This holds because the
  first render is a full scan, every later change is either folded in as a named
  path or escalated to a full rescan, and each named path is re-verified by the
  shared classifier rather than trusted from the event alone.
- **One classifier.** `watch` decides a path's state through the same
  [`classify`](openscion-client-status.md) that backs `status` and `commit`;
  it re-reads a named file and compares blob ids, so an event on a file whose
  bytes did not change (a touch) produces no reported change.
- **Read-only.** A render reads local state and the named files; it writes
  nothing to disk or to `state.json`.
- **Pure core, thin adapter.** Folding events into the pending set and rendering
  it are pure and deterministic, exercised without a real filesystem. The
  operating-system watcher is a small adapter that turns native events into the
  core's event type and feeds them in; it is the only part that touches the OS.
- **Local.** `watch` makes no server request and runs with no server reachable.

## Acceptance criteria

- The first render on a clean working copy is `clean`; the first render on a
  copy with pending edits reports them in full, matching `opnscn status`.
- After a file is created, edited, or deleted, the next render reflects exactly
  that change and leaves every other path's reported state unchanged.
- A file rewritten with its original bytes (a touch) produces no reported change
  even though it raised a filesystem event.
- An overflow or un-attributable event produces a render identical to a full
  `opnscn status` — a rescan, not a stale or partial answer.
- The event-folding and rendering core is covered by tests that drive synthetic
  events, with no dependence on a real filesystem or on wall-clock timing.
- `watch` makes no server request and does not modify `state.json` or any file.

## Open questions

- **A quiet/diff mode** that prints only the lines that changed since the last
  render, for a terminal UI, rather than the whole block each time. Additive;
  deferred until a consumer needs it.
- **Coalescing window.** Renders currently fire once per drained burst of
  events. A short debounce could merge a rapid sequence of saves into one
  render; it trades latency for fewer renders and is a tuning choice, not a
  correctness one, so it is deferred.
- **Daemon-served one-shot status.** Letting `opnscn status` consult a resident
  watcher so a single invocation skips the scan belongs with the node daemon and
  its client channel, where the resident process and transport are defined.

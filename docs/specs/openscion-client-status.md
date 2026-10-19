# Working-copy status

- **Status:** Accepted
- **Owner:** OpenScion client
- **Specifications it builds on:**
  [architecture](openscion-architecture.md) (phase 5, the client),
  [data model](openscion-data-model.md) (changesets, trees, blobs).

## Problem

An agent working in a cloned directory needs to know, before it commits, what
its edits will record: which files it added, which it changed, and which it
removed, relative to the changeset the working copy is based on. Today the only
way to find out is to run `opnscn commit`, which is destructive — it writes a
changeset and moves the ref. There is no read-only way to preview the pending
change set.

The answer must be trustworthy. A report that calls a file "modified" when only
its timestamp moved trains the reader to ignore it; a report that calls a file
"clean" when its bytes changed loses work. Status must reflect content, not
stat metadata, and it must never depend on the network: an agent offline from
the server still knows what it has changed.

## Goals and non-goals

### Goals

- A read-only `opnscn status` that reports the working copy's pending changes
  against its base changeset: additions, modifications, and deletions.
- Content truth: a file is "modified" only when its stored bytes (contents, or a
  symlink's target) or its kind differ from the base. A file whose timestamp
  moved but whose bytes did not is clean.
- Entirely local: status reads `.opnscn/state.json` and the files on disk. It
  makes no server request and works with no server reachable.
- One authority for "what changed": `status` and `commit` classify a scanned
  file against the base through the same code, so the two can never disagree
  about whether a file changed.

### Non-goals

- Accelerating the scan with a filesystem watcher. Status walks the tree each
  run; the watcher that makes it incremental is a later phase-5 chunk, and it
  must not change what status reports, only how fast.
- Reporting divergence from the *server's* current ref tip (whether the base is
  behind `main`). That is a server question (`log`, `ancestor`) and stays out of
  the local status report.
- Managing the sparse set, ignore rules, or a staging area. Status reports every
  tracked path under the working copy exactly as `commit` would record it; it
  adds no selection model of its own.
- Persisting a refreshed stat cache. Status is read-only and never writes
  `state.json`; a file whose bytes are unchanged but whose stat data moved is
  re-hashed on each status until the next commit refreshes the cache.

## Proposed behavior

### Command

```text
opnscn status [--dir <path>]
```

`--dir` is the working-copy root and defaults to the current directory, matching
`commit`. The global `--server` flag is accepted but unused: status never
contacts a server.

### Classification

For each path, status reports exactly one state, decided against the base
changeset recorded in `state.json`:

| State | Shown | Meaning |
| --- | --- | --- |
| Added | `A <path>` | On disk, not tracked in the base. |
| Modified | `M <path>` | Tracked, but its bytes or kind differ from the base. |
| Deleted | `D <path>` | Tracked in the base, absent from disk. |
| Clean | (not shown) | Tracked and identical to the base. |

A file's kind is one of file, executable, or symlink; a change of kind alone
(for example a file gaining its execute bit) is a modification even when the
bytes are unchanged. Symlinks are compared by their target bytes, as they are
stored.

Lines are sorted by path. A working copy with no pending change prints a single
line, `clean`. Status exits `0` whether or not there are changes; it is a
report, not a gate. Its only non-zero exits are the existing working-copy
errors: the directory is not a working copy, a file cannot be read, or a file
name is not UTF-8.

### Example

A clone of five files, then an edit, a new file, and a removal:

```console
$ opnscn status
A src/new.rs
D docs/guide.md
M README.md
```

Rewriting `README.md` with its original bytes and running status again prints
`clean`: the timestamp moved, the content did not.

## Invariants and constraints

- **Content decides, not mtime.** A file counts as unchanged only when its stat
  data proves it (same kind, size, modification time, and inode change time,
  modified strictly before the state was written) or, failing that, when it
  re-reads to the same blob id and kind as the base. The inode change time is
  what makes the stat fast path safe; a modification time alone can be restored
  by user space and is never trusted on its own. This is the rule
  [`unchanged`](../plans/workcopy-stat-cache-change-time.md) already enforces for
  `commit`.
- **One classifier.** `status` and `commit` decide a file's state through the
  same function. `commit` additionally uploads the changed blobs, writes a
  changeset, and refreshes the stat cache; `status` does none of that. Neither
  may re-derive the add/modify/delete decision independently.
- **Read-only.** A status run leaves every byte on disk and `state.json`
  unchanged.
- **Local and total.** Status depends on no network call. It reports every
  tracked path and every path on disk under the root (excluding `.opnscn/`), so
  no pending change is omitted.
- **Deterministic.** Output is sorted by path and is independent of filesystem
  iteration order.

## Acceptance criteria

- `opnscn status` on a fresh clone prints `clean`.
- After adding one file, editing one, and deleting one, status prints exactly
  those three lines with codes `A`, `M`, and `D`, sorted by path, and nothing
  for the untouched files.
- Rewriting a tracked file with identical bytes (a new timestamp, same content)
  leaves status reporting `clean`.
- Giving a tracked file a different kind with identical bytes reports it `M`.
- Status makes no server request: it succeeds against an unreachable or
  unset server.
- Running status does not change `state.json` or any file on disk.
- `status` in a directory that is not a working copy fails with the existing
  "not an opnscn working copy" error.
- The set of changes status reports equals the set `commit` records from the
  same working-copy state.

## Open questions

- **A `--clean`/verbose mode** that also lists unchanged files, for callers that
  want the whole tree's state in one pass. Deferred until a caller needs it; the
  classifier already distinguishes clean from changed, so it is additive.
- **Porcelain vs. plain output.** The `A`/`M`/`D` form is stable and greppable.
  A machine-readable (JSON) form can be added behind a flag without changing the
  default, when a client consumes status programmatically.

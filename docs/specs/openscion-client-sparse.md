# Sparse checkout management

- **Status:** Accepted
- **Owner:** OpenScion client
- **Specifications it builds on:**
  [architecture](openscion-architecture.md) (phase 5, the client),
  [data model](openscion-data-model.md) (trees, changesets, blobs),
  [working-copy status](openscion-client-status.md) (what a working copy tracks).

## Problem

An agent working in an agent-scale monorepo does not want the whole tree on
disk — only the directories it is editing. `opnscn clone --path <dir>` already
fetches one directory of a snapshot into a working copy, but the choice is
frozen at clone time. To widen the checkout to another directory, or to drop one
it no longer needs, the only recourse today is to throw the working copy away
and clone again, re-fetching everything.

The working copy records the sparse directory it was limited to — a single
string — but nothing reads it, nothing can show it, and nothing can change it.
The sparse checkout is not a first-class, editable part of the working copy; it
is a clone-time footnote.

## Goals and non-goals

### Goals

- A sparse set the working copy owns and can report: the directories (or files)
  currently materialized, or "full" when the whole tree is present.
- `opnscn sparse` to show the current set.
- `opnscn sparse add <path>…` to widen the checkout: fetch the named paths from
  the server at the working copy's base changeset, write them, and track them.
- `opnscn sparse remove <path>…` to narrow it: delete the named paths' files
  from disk and stop tracking them, locally, without touching the repository.
- A single membership rule — "does this path belong to the set?" — that clone,
  add, and remove all share, so a path is in the checkout for exactly one
  reason.
- Backward compatibility: a working copy written before the sparse set was a
  set (a lone string, or nothing) still loads, with the same meaning.

### Non-goals

- Converting a full checkout into a sparse one, or the reverse, in place.
  `sparse add` and `sparse remove` edit an existing sparse set; a full checkout
  stays full (re-clone with `--path` to start sparse). This keeps the
  "materialize everything-except" case, which a full-to-sparse narrowing would
  need, out of scope.
- Removing part of a broader entry. `remove` takes whole entries that `add` (or
  the clone) put in the set; it does not split `src` to drop `src/bin`.
- Constraining which paths a commit may record. The sparse set governs what is
  materialized, not what may be written; a file present on disk is committed
  whether or not it is under the set, and a tracked file absent from disk is a
  deletion, exactly as without a sparse set. (`status`, `commit`, and `watch`
  already key off what the working copy tracks, which `add` and `remove` keep
  correct.)
- A server round-trip for `remove`. Narrowing only deletes local files; the
  objects remain in the repository.

## Proposed behavior

### The sparse set

A working copy is either a **full** checkout (the whole tree) or a **cone**: a
set of path prefixes, each a directory or file, that are materialized. A repo
path belongs to a cone when it equals one of the prefixes or lies under one
(`src` covers `src`, `src/lib.rs`, and `src/bin/main.rs`, but not `srcfoo`). The
set is normalized: no duplicates, and no entry nested under another (adding
`src` when `src/bin` is present replaces it with `src`).

`opnscn clone` with no `--path` makes a full checkout; with `--path <dir>` it
makes a cone of that one path. The set is recorded in `.opnscn/state.json`. A
state written with the earlier single-string form (`"sparse": "docs"`) loads as
the cone `{docs}`, and the earlier absent/null form as full.

### Commands

```text
opnscn sparse
opnscn sparse add <path>…
opnscn sparse remove <path>…
```

- **`opnscn sparse`** prints the current set: `full checkout` for a full
  checkout, otherwise one line per prefix, sorted; a cone of no paths prints
  `sparse checkout of no paths`.
- **`opnscn sparse add`** requires a cone (a full checkout has nothing to add
  to, and errors). For each path not already covered, it fetches that path's
  files from the server — at the working copy's base changeset, from the server
  the working copy came from — writes them, records them as tracked, and adds
  the path to the set. It reports the number of files materialized. A path that
  names nothing in the snapshot adds the prefix and zero files.
- **`opnscn sparse remove`** requires a cone. For each path, which must be an
  entry in the set, it deletes that entry's materialized files from disk, stops
  tracking them, prunes directories left empty, and removes the entry from the
  set. It reports the number of files removed, and makes no server request.

`add` and `remove` use the server and base recorded in the working copy's
state, like `commit`; the global `--server` flag does not apply.

### Example

```console
$ opnscn clone work --path docs
cloned 2 files of 3f… into work
$ cd work && opnscn sparse
docs
$ opnscn sparse add src
added 5 files
$ opnscn sparse
docs
src
$ opnscn sparse remove docs
removed 2 files
$ opnscn sparse
src
```

## Invariants and constraints

- **One membership rule.** Whether a path belongs to the set is decided in one
  place and used by clone, add, and remove alike. A path belongs iff it equals,
  or lies under, one of the set's prefixes.
- **The set and the tracked files agree.** After any sparse command, the
  working copy tracks exactly the files it materialized: `add` adds the fetched
  files, `remove` drops the deleted ones. A narrowing deletes from disk and from
  the tracked set together, so `status` never reports a removed path as a
  pending deletion.
- **Normalized set.** The set never holds a duplicate or an entry nested under
  another, so each materialized path belongs to the set for exactly one reason
  and a `remove` of an entry deletes a well-defined slice.
- **Content-addressed fetch.** `add` verifies every object it fetches against
  its id, like `clone`; a server cannot return bytes that do not hash to the id
  requested.
- **Backward compatible.** A `state.json` from before the set existed loads with
  its original meaning (a string becomes a one-entry cone, absent becomes full).
- **`remove` is local.** It deletes only files under the working-copy root and
  changes only local state; the repository is untouched.

## Acceptance criteria

- `opnscn sparse` on a full checkout prints `full checkout`; on a cone it prints
  the prefixes, sorted.
- `sparse add` fetches and writes the named path's files, which then appear in
  `ls`-equivalent scans and are clean in `status`; a second clone of the same
  ref reproduces the same bytes.
- `sparse remove` deletes the named entry's files from disk and from tracking,
  prunes emptied directories, and leaves `status` clean (not reporting
  deletions).
- `sparse add` and `sparse remove` on a full checkout fail with a clear error;
  `sparse remove` of a path that is not an entry fails.
- A `state.json` written with the single-string sparse form loads as the
  one-entry cone, and a round-trip of the set preserves it.
- The set is normalized: adding an entry that covers an existing one collapses
  them.

## Open questions

- **Full-to-sparse narrowing.** Letting `sparse remove` on a full checkout carve
  out a cone (materialize everything but the removed paths) is useful but needs
  the "everything-except" materialization; deferred.
- **Reconciling the set with a moved base.** After `commit` or a server-side
  landing moves the base, `add` fetches at the new base, but already-materialized
  paths are not refreshed. A `sparse sync` that re-materializes the whole set at
  the current base is a natural follow-up.
- **Committing outside the cone.** Whether a commit should warn or refuse when it
  records a path outside the sparse set is a policy question left open; today it
  records it.

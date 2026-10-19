# Plan: derived data and queries (phase 2)

- **Status:** Done (in-memory engine)
- **Specifications:**
  [`../specs/openscion-api.md`](../specs/openscion-api.md),
  [`../specs/openscion-architecture.md`](../specs/openscion-architecture.md#delivery-phases)

## Goal

Answer the questions a tool asks without a checkout, on the in-memory
[`Repository`]: resolve paths in a snapshot, diff two snapshots, and walk
history and ancestry. Generation numbers bound the ancestry walks.

## Non-goals

First-parent skip lists as a separate structure (the generation numbers already
bound the walks), persistent derived indexes, and the HTTP routes over these
methods (a thin follow-on in `openscion-server`).

## Tasks, test-first

1. **Generation numbers** — `repo/types.rs`, `repo/mod.rs`. Each `LogEntry`
   stores `1 + max(parent generations)`, set once at append. `generation`
   exposes it. Test: a merge's generation is one past its deepest parent.
2. **Snapshot reads** — `repo/query.rs`: `stat`, `read_file`, `list_tree`.
   A shared `resolve` walks the root tree, loading only the trees on the path.
3. **Diff** — `repo/query.rs`: `diff` walks two root trees in lockstep,
   pruning subtrees whose ids are equal, emitting `Upsert`/`Delete` filtered by
   path prefix. Test: applying the diff to `from` reproduces `to`'s root tree.
4. **History** — `repo/query.rs`: `log` walks the first-parent line, newest
   first, excluding `until`, capped by `limit`; with a path it emits only the
   changesets whose snapshot of that path differs from their first parent's.
5. **Ancestry** — `repo/query.rs`: `is_ancestor` (generation-pruned walk over
   all parents) and `merge_base` (greatest-generation common ancestor, found by
   a generation-ordered frontier). Both are checked against a brute-force walk
   on randomized histories.

## New types and errors

- `openscion-proto`: `ChangesetSummary` (one `Log` row), serde form pinned.

## Verification

- Focused: `cargo test -p openscion-core --lib`
- Acceptance (API spec): `Log` with a path returns exactly the touching
  changesets; `MergeBase` and `IsAncestor` agree with a brute-force walk on
  randomized histories.

## Checklist

- [x] generation numbers stored and monotonic
- [x] stat / list_tree / read_file resolve without a checkout
- [x] diff reproduces the target snapshot; prefixes narrow it
- [x] log first-parent + path history
- [x] is_ancestor and merge_base agree with brute force
- [x] fmt, clippy, full suite, 90% per-file coverage green

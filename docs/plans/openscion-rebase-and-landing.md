# Plan: rebase, revert, and landing (phase 3)

- **Status:** Done (in-memory engine)
- **Specifications:**
  [`../specs/openscion-api.md`](../specs/openscion-api.md#rebase-revert-and-landing)

## Goal

Move work onto a ref without a global lock: rebase a changeset's changes onto
another changeset with file-level conflict detection, revert a changeset, and
land through a compare-and-swap retry loop.

## Non-goals

Line-level three-way merge (file-level conflicts only, per the spec), and the
HTTP routes over these methods.

## Approach

`replay(base, changes, onto)` is the shared conflict engine. For each path the
changes touch it compares the `base` snapshot the changes are relative to
against `onto`:

- unchanged between `base` and `onto` → the change applies as-is;
- `onto` already holds the change's result → drop it (already there);
- otherwise → conflict.

Any conflict returns [`Error::MergeConflict`] with every conflicting path and
creates nothing.

## Tasks, test-first

1. **Rebase** — `repo/rebase.rs`: `rebase(changeset, onto)` replays
   `changeset`'s changes (base = its first parent) onto `onto`; a new changeset
   with first parent `onto` and `rebased-from` metadata, or `onto` when every
   change is already present.
2. **Revert** — `repo/rebase.rs`: `revert` replays the inverse changes (each
   touched path restored to the first parent's entry), base = `changeset`'s own
   snapshot, with `reverts` metadata.
3. **Land** — `repo/rebase.rs`: `land(changeset, ref, max_attempts)` runs the
   spec's loop — resolve the tip, rebase onto it when it moved, compare-and-swap
   the ref, retry on `RefConflict`. The result always has the prior tip as an
   ancestor by construction.
4. **Error** — `error/mod.rs`: `MergeConflict { paths }`, with a stable code.

## Verification

- Focused: `cargo test -p openscion-core --lib rebase revert land`
- Acceptance (API spec): two concurrent lands on disjoint files both succeed
  with the ref ending on a changeset that contains both; concurrent lands on
  the same file produce exactly one `MergeConflict`.

## Checklist

- [x] rebase applies, drops identical, conflicts; creates one changeset or none
- [x] revert restores the first parent's content
- [x] land: disjoint files both succeed; same file conflicts
- [x] MergeConflict error with a stable code
- [x] fmt, clippy, full suite, 90% per-file coverage green

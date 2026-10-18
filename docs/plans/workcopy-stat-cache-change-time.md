# Plan: Working-copy stat cache validity

- **Status:** Done
- **Issue:** [#6](https://github.com/tinyhumansai/openscion/issues/6) —
  the stat cache could silently classify a changed file as clean.
- **Specifications:**
  [`../specs/openscion-architecture.md`](../specs/openscion-architecture.md)

## Goal

Make the working-copy stat cache unable to classify a content change as clean.
`opnscn commit` re-reads a file only when `workcopy::unchanged` says its stat
data is still trustworthy; that decision must never produce a wrong "clean".

## Problem

`unchanged` trusted `kind`, `size`, and `mtime`, plus the racy-clean rule
`mtime < written_at`. The modification time is the only content-sensitive field,
and user space can restore it. Rewriting a file with different bytes of the same
length and restoring its `mtime` (an archive extraction, a filesystem restore,
or `touch -r`) satisfied the predicate, so `commit` skipped hashing it and the
edit was dropped from the changeset.

## Non-goals

A forced-refresh / always-hash mode, inode-identity tracking, and closing the
residual on platforms without an inode change time. Each can be added later
without changing the `unchanged` interface.

## Approach

Add the inode change time to the file identity. A content write always advances
it and user space cannot set it, so it closes the gap the modification time
leaves. The cache fails safe toward re-reading:

- `FileState` and `Scanned` carry a `ctime` (nanoseconds, `0` where the platform
  has none). `FileState.ctime` is `#[serde(default)]`, so a `state.json` written
  before the field existed still loads with `ctime == 0`.
- `change_time` reads it (`MetadataExt::ctime` on Unix, `0` otherwise), isolating
  the one platform difference, as `is_executable` already does.
- `unchanged` trusts a file only when `known.ctime != 0` and it matches. A `0`
  (an older state, or a platform without one) is never trusted, so those files
  are re-hashed once rather than ever skipped.

## Residual tradeoff (explicit)

On a platform or filesystem without a trustworthy inode change time the change
time is `0` and every file is re-read: correct, at the cost of the stat-cache
fast path. A privileged global clock rewind between two writes could still fool
the change time; that is outside the supported contract.

## Tasks

Test-first: the failing test, then the code that passes it.

1. **Regression test** — `crates/openscion/src/workcopy/test.rs`:
   `a_same_size_edit_with_a_restored_mtime_is_detected` writes a file, scans it,
   rewrites same-length content, restores the `mtime` with `File::set_modified`,
   and asserts the scan is no longer `unchanged`. Fails before the change.
2. **Identity** — `crates/openscion/src/workcopy/types.rs`: add `ctime` to
   `FileState` (serde default) and `Scanned`.
3. **Scan and predicate** — `crates/openscion/src/workcopy/mod.rs`: `change_time`
   helper, `walk` and `stat` populate the change time, `unchanged` requires it.
4. **Write sites** — `commands/{import,commit,clone}.rs` record the change time.
5. **Migration test** — `a_file_state_without_a_change_time_still_loads` pins the
   serde default for an old `state.json`.

## Verification

- Focused: `cargo test -p openscion --lib workcopy`
- Full: `cargo fmt --all -- --check`, `cargo clippy --all-targets
  --all-features -- -D warnings`, `cargo test --all-features`.

## Checklist

- [x] regression test fails before, passes after
- [x] old `state.json` still loads
- [x] docs and module comments updated
- [x] fmt, clippy, full test suite green

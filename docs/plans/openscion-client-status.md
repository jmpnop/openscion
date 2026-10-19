# Plan: working-copy status

- **Status:** Done
- **Specification:**
  [`../specs/openscion-client-status.md`](../specs/openscion-client-status.md)
- **Phase:** 5 (client and daemon), first chunk.

## Goal

Add a read-only `opnscn status` that reports a working copy's pending changes
against its base changeset — additions, modifications, deletions — from local
state alone, deciding by content rather than timestamp, and sharing one
classifier with `commit` so the two never disagree.

## Approach

The dangerous decision — "has this file changed since the base?" — lives once,
in the working-copy module, as a `classify` function. It applies the existing
`unchanged` stat fast path first and, when that cannot prove a file clean,
re-reads the file and compares blob ids. It returns enough for both callers:

- `Clean { refreshed }` — unchanged; `refreshed` carries a new `FileState` only
  when the file was re-read because its stat data moved, so `commit` can update
  its cache. `status` ignores it.
- `Changed { known_before, blob, bytes, fresh }` — new or differing.
  `known_before` separates an addition (not tracked before) from a modification.
  `commit` uploads `bytes`, records a `Change::Upsert`, and persists `fresh`;
  `status` needs only `known_before`.

`commit` is rewritten to consume `classify`, preserving its current behavior
exactly (the existing commit tests must stay green). `status` is a new,
server-free command that classifies each scanned file, adds deletions by set
difference against `state.files`, sorts by path, and formats `A`/`M`/`D` lines.

## Tasks

Test-first: the failing test, then the code that passes it.

1. **Classifier** — `crates/openscion/src/workcopy/mod.rs`: add `Classified`
   and `classify(root, file, state)`. Unit-test in `workcopy/test.rs`:
   stat-clean, touched-identical (refreshed is `Some`), added (`known_before`
   false), modified (`known_before` true), and kind-change-with-same-bytes
   (modified).
2. **Rewire commit** — `crates/openscion/src/commands/commit.rs`: replace the
   inline classify loop with `classify`. No behavior change; the existing
   `commit_records_edits_additions_and_deletions`,
   `a_clean_or_merely_touched_tree_has_nothing_to_commit`, and race tests cover
   it.
3. **Status command** — `crates/openscion/src/commands/status.rs`: `StatusCode`
   (`Added`/`Modified`/`Deleted`, each with its letter) and
   `status(root) -> Result<Vec<(String, StatusCode)>>`. Wire it from
   `commands/mod.rs`. Test in `commands/test.rs`: a clone with an add, an edit,
   and a delete reports exactly those three; a touched-identical file is clean;
   a kind change is `M`; status equals the set `commit` would record; status on
   a non-working-copy dir errors; status issues no server request (run it with
   no server and assert success); status does not change `state.json`.
4. **CLI** — `crates/openscion/src/cli/mod.rs`: `Command::Status { dir }`,
   dispatch, and formatting (`<letter> <path>` lines, or `clean`). Test in
   `cli/test.rs`: end-to-end `status` output, including the `clean` case.
5. **Docs** — module comments on the new surface; `ROADMAP.md` notes status as
   the first phase-5 deliverable; spec and plan linked from their READMEs.

## Verification

- Focused: `cargo test -p openscion status`,
  `cargo test -p openscion --lib workcopy`.
- Full contract: `cargo fmt --all -- --check`, `cargo clippy --all-targets
  --all-features -- -D warnings`, `cargo build --all-targets --all-features`,
  `cargo test --all-features`.
- Rustdoc: `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps --all-features`.
- Per-file coverage ≥90% on every touched source file.

## Checklist

- [x] one classifier shared by `status` and `commit`
- [x] status is read-only and makes no server request
- [x] content, not mtime, decides modified vs clean
- [x] existing commit tests stay green
- [x] fmt, clippy, build, full test suite, rustdoc green
- [x] per-file coverage ≥90%

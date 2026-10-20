# Plan: git ingestion (first-parent import)

- **Status:** Done
- **Specification:**
  [`../specs/openscion-git-ingestion.md`](../specs/openscion-git-ingestion.md)
- **Decision record:** [ADR 0005](../adr/0005-git-ingestion-reader.md) — read
  git with `gix`.
- **Phase:** 6 (git ingestion), first chunk.

## Goal

A `openscion-git` library that imports a git repository's first-parent spine
into OpenScion changesets on a ref, reproducing each commit's tree and
preserving the git identity and timestamps in metadata.

## Approach

`import_first_parent(repo, into, start, ref_name)` opens the git repo with
`gix`, resolves `start` to a commit, and collects its first-parent spine
(follow the first parent to the root, reverse to oldest-first). For each spine
commit it flattens the git tree into a `BTreeMap<path, (EntryKind, git oid)>` by
recursive walk — mapping git modes to `EntryKind`, skipping and counting
submodule gitlinks, erroring on a non-UTF-8 path. It diffs the flattened map
against the previous commit's: entries added or changed (kind or oid) become
`Change::Upsert` (the git blob's bytes go in via `Repository::put_blob`), entries
gone become `Change::Delete`. It creates a changeset with the previous imported
id as first parent, the git author as `Name <email>`, the message, and metadata
(`git.commit`, `git.author`, `git.committer`, `git.author_time`,
`git.commit_time`), then moves `ref_name` to the tip. Diffing flattened maps
ourselves keeps the `gix` surface to object reading.

## Tasks

Test-first: the failing test, then the code that passes it.

1. **Crate skeleton** — `crates/openscion-git/` (done: `Cargo.toml` with `gix`
   and `openscion-core`), `src/lib.rs` (crate docs + public surface),
   `src/error/mod.rs` (crate-wide `Error`/`Result`: `OpenGit`, `GitObject`,
   `NonUtf8Path`, `Core`).
2. **Entry translation** — `src/convert/mod.rs` + `test.rs`: map a `gix` tree
   entry mode to `Option<EntryKind>` (None for a submodule) and build a flat
   `BTreeMap<String, Entry>` from a git tree by recursive walk; the diff of two
   such maps into `Vec<Change>` plus the set of blobs to fetch. Unit-test the
   mode mapping and the map diff (add / modify / delete / kind-change) without a
   real repo.
3. **Import driver** — `src/import/mod.rs` + `test.rs`: `import_first_parent`
   and `Imported`, walking the spine and driving the `Repository`. Tests build a
   real git repo with the `git` CLI (fixed identity and dates for determinism)
   and assert the imported history: changeset count and first-parent order, the
   tip tree reproduced (paths, kinds, bytes including an executable and a
   symlink), a merge importing only the spine, a skipped submodule counted,
   provenance metadata present, and a non-UTF-8 path erroring.
4. **Surface** — `lib.rs` re-exports `import_first_parent`, `Imported`, `Error`,
   `Result`; crate doc with a short example.
5. **Docs** — spec/plan/ADR indexed; `ROADMAP.md`.

## Verification

- Focused: `cargo test -p openscion-git`.
- Full contract: `cargo fmt --all -- --check`, `cargo clippy --all-targets
  --all-features -- -D warnings`, `cargo build --all-targets --all-features`,
  `cargo test --all-features`, and `cargo deny check` (the new `gix` tree).
- Rustdoc: `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps --all-features`.
- Per-file coverage ≥90% on every new source file.

## Checklist

- [x] one changeset per spine commit, in first-parent order
- [x] trees reproduced: file, executable, symlink round-trip
- [x] a merge imports only the first-parent spine
- [x] submodule entries skipped and counted; non-UTF-8 path errors
- [x] git commit id, identities, and timestamps in changeset metadata
- [x] `cargo deny` clean with the `gix` dependency tree
- [x] fmt, clippy, build, full test suite, rustdoc green; per-file coverage ≥90%

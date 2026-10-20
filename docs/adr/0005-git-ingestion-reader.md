# 5. Read git with gitoxide (gix) for ingestion

- **Status:** Accepted
- **Date:** 2026-10-19

## Context

Phase 6 imports an existing git repository into OpenScion: a first-parent walk
of its history, translating each commit's tree into OpenScion changes. That
requires reading git's object database — loose and packed objects, zlib, delta
resolution, commit and tree parsing, ref resolution. OpenScion must choose how
to read git.

The workspace has firm constraints that bear on the choice: dependencies are
pure Rust and reproducible where possible (the SQLite store bundles a pinned
SQLite rather than linking a system library), `cargo deny` enforces a license
allow-list (permissive licenses plus `GPL-3.0-only`) and fails on advisories or
unmaintained crates, and the project forbids shortcuts — offloading work to an
external process the user must have installed is exactly such a shortcut.

## Decision

Read git with **`gix` (gitoxide)**, the pure-Rust git implementation, as a
dependency of a new `openscion-git` crate.

Rejected alternatives:

- **`git2` (libgit2).** Pulls `libgit2-sys`, a C library, against the
  pure-Rust/reproducible stance — and libgit2 is `GPL-2.0 WITH
  GCC-exception`, which is **not** in the `cargo deny` license allow-list, so it
  fails the supply-chain gate outright.
- **Shelling out to the `git` binary.** Requires git installed at run time, is
  fragile across git versions, and is the kind of external-process offload the
  project's no-shortcuts rule rejects.
- **Hand-rolling git's object format** (zlib, loose + packfile, deltas). That is
  reimplementing `gix` — a large, subtle surface — for no benefit; using the
  maintained pure-Rust implementation is the correct engineering choice, the
  same way the project uses the `blake3` crate rather than hand-rolling the hash.

`gix` (0.89) builds on the workspace toolchain, and its dependency tree passes
`cargo deny check` (advisories, bans, licenses, sources all clean) with no
change to the license allow-list.

## Consequences

- A new `crates/openscion-git/` library depends on `gix` and `openscion-core`.
  It reads a git repository and drives `Repository` (put blobs, create
  changesets, move a ref); it does not touch the stores directly.
- The ingester reads only git objects. It diffs flattened git trees itself into
  OpenScion `Change`s rather than relying on `gix`'s content-diff, so it needs
  only `gix`'s object reading, keeping the enabled feature surface small.
- `gix` is a large dependency tree. The crate is `publish = false` (an internal
  ingestion tool), and its features are kept to what object reading needs. The
  supply-chain gate (`cargo deny`) guards the tree on every CI run; a future
  `gix` upgrade that trips it is caught there.
- Git concepts without an OpenScion equivalent are handled explicitly by the
  ingester (see the spec): submodule gitlinks are skipped and counted,
  non-UTF-8 paths are an error, and the original git identity and timestamps are
  preserved in changeset metadata since OpenScion stamps its own creation time.

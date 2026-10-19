# Roadmap

The full sequence, with what each phase delivers, is in
[`docs/specs/openscion-architecture.md`](docs/specs/openscion-architecture.md#delivery-phases).
This page tracks where things stand.

## Done

- Workspace, lints, CI (format, clippy, build, test, per-file coverage,
  rustdoc, MSRV, supply chain), and a tag-and-release workflow.
- Draft specs: architecture, data model, and API.
- **Core data model** ([plan](docs/plans/openscion-data-model.md)): object ids, paths, trees,
  changesets, the in-memory log and compare-and-swap refs, root-tree
  derivation by path copying; the server's object, changeset, and ref routes;
  `opnscn import`, `commit`, and `clone` (including one-directory clones); and an
  OpenScion backend in `openscion-bench`.
- **Derived data and queries** ([plan](docs/plans/openscion-derived-data.md)):
  generation numbers; `Stat`, `ListTree`, `ReadFile`, `Diff`, `Log` with path
  history, `IsAncestor`, and `MergeBase` on the in-memory engine, the last two
  checked against a brute-force walk on randomized histories.
- **Server-side rebase and landing** ([plan](docs/plans/openscion-rebase-and-landing.md)):
  `Rebase`, `Revert`, and `Land` with its compare-and-swap retry loop and
  file-level conflict reporting.
- **HTTP routes and `opnscn` subcommands** over the phase-2 and phase-3 engine
  methods: the snapshot reads (`stat`, `tree`, `file`) and history questions
  (`diff`, `log`, `ancestor`, `merge-base`) on the server, the landing routes
  (`rebase`, `revert`, `land`), and the matching `opnscn stat`, `ls`, `cat`,
  `diff`, `log`, `ancestor`, `merge-base`, `rebase`, `revert`, and `land`
  commands, which name a revision by object id or ref name.
- **Persistent stores** ([research](docs/research/storage-persistence.md)): the
  `ObjectStore` and `MetadataStore` ports as traits with `Repository` holding
  them; a filesystem `ObjectStore` (git-style fanout, atomic
  temp+fsync+rename); a SQLite `MetadataStore` behind the `sqlite` feature (WAL,
  single-statement ref compare-and-swap, autoincrement LSN); crash-recovery
  tests across an abrupt kill; and `openscion-server --data-dir`, so history
  survives a restart. (MSRV raised to 1.90 for `rusqlite`.)
- **First-parent skip lists** ([spec](docs/specs/openscion-data-model.md#derived-data)):
  an eager derived kind storing each changeset's first-parent ancestor at
  distance 2^k, in both metadata stores, so `is_first_parent_ancestor` jumps the
  history in O(log n) and `is_ancestor` and `merge_base` take that fast path.
  Profiling first confirmed the need: a single `is_ancestor` down a 100k-deep
  linear spine was ~1 s on the SQLite store (one lookup per hop); the skip list
  makes the same question ~30 lookups. This completes the derived-data phase.

- **Working-copy status** ([spec](docs/specs/openscion-client-status.md),
  [plan](docs/plans/openscion-client-status.md)): `opnscn status`, a read-only
  report of a working copy's pending changes — added, modified, and deleted
  files — against its base changeset, from local state alone and with no server
  request. It decides by content, not timestamp: a file whose bytes are
  unchanged is clean even when its mtime moved. One classifier backs both
  `status` and `commit`, so they can never disagree about what changed. First
  chunk of phase 5.

## Next

1. **`opnscn` client and node daemon** (phase 5, continued): a filesystem
   watcher that makes `status` incremental rather than a full rescan;
   first-class sparse-set management (editing the checked-out paths after a
   clone); and the node daemon with its node-local store and snapshot boot.
   `status`, `commit`, and `land` already exist in the client.

## Later

The `opnscn` client and node daemon, git ingestion, network tiers (where the
distributed `MetadataStore` — Postgres/pgrust, Cassandra, or FoundationDB — and
a propagation bus live), line-level blame and merge, virtual checkouts, read
access control, and an S3 `ObjectStore`. A
DuckDB-backed analytics plane ([ADR 0002](docs/adr/0002-duckdb-analytics-plane.md))
and a Telegram cold object tier ([ADR 0003](docs/adr/0003-telegram-object-tier.md))
are optional adjacent systems, not part of the serving path.

## Out Of Scope

- Replacing git for ordinary repositories.
- Offline-first operation and branch-heavy workflows.
- Code review, code search, and CI themselves. OpenScion provides the APIs
  they need.

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

- **Watcher-backed status** ([spec](docs/specs/openscion-client-watch.md),
  [plan](docs/plans/openscion-client-watch.md)): `opnscn watch`, a long-running
  render of a working copy's pending changes that re-renders as files change,
  re-examining only the paths a filesystem event named instead of walking the
  tree. Its answer always equals a full `opnscn status`: the first render is a
  full scan, each later change is re-verified through the shared classifier, and
  any overflow or un-attributable event forces a full rescan. The event-folding
  and rendering core is pure and deterministic; a thin `notify` adapter feeds it
  native events. Second chunk of phase 5.
- **Sparse checkout management** ([spec](docs/specs/openscion-client-sparse.md),
  [plan](docs/plans/openscion-client-sparse.md)): the sparse checkout is now a
  first-class, editable set. `opnscn sparse` shows the materialized paths;
  `sparse add` widens the checkout, fetching more of the tree from the server
  (reusing the same fetch as `clone`); `sparse remove` narrows it, deleting
  paths locally and pruning emptied directories. One membership rule — the
  `Sparse` type — is shared by clone, add, and remove, the set stays a
  normalized antichain, and a working copy written with the earlier
  single-string form still loads. Third chunk of phase 5.
- **Daemon node-local store and snapshot boot**
  ([spec](docs/specs/openscion-daemon-node-store.md),
  [plan](docs/plans/openscion-daemon-node-store.md)): the new `openscion-daemon`
  crate's `NodeStore` — a bounded, read-through cache of immutable objects
  behind a single `get`, with `snapshot`/`warm` so a fresh node starts warm. It
  serves a local copy or fetches from the tier above (an `ObjectSource`),
  verifies the bytes against the id by recovering the kind, caches within a byte
  budget, and evicts least-recently-used; the index lock is never held across an
  upstream fetch or a disk write. Chose the deep read-through module via a POSD
  Design-It-Twice pass; the transport decision is recorded in
  [ADR 0004](docs/adr/0004-daemon-transport.md). Fourth chunk of phase 5.
- **Daemon framed transport** ([spec](docs/specs/openscion-daemon-transport.md),
  [plan](docs/plans/openscion-daemon-transport.md),
  [ADR 0004](docs/adr/0004-daemon-transport.md)): the object RPCs over a byte
  stream — a length-framed codec (tag, length, body; a found response carries
  raw object bytes), `serve_connection`/`serve_tcp`/`serve_unix` answering from a
  `NodeStore`, and a `SocketSource` that is itself an `ObjectSource`, so a node
  store on one tier reads through a daemon on the tier above over the same wire,
  on a Unix socket locally or TCP between machines. The codec and server are
  generic over the stream; only the Unix socket is `#[cfg(unix)]`. Fifth chunk,
  concluding phase 5 — **the client and daemon phase is complete.**

- **Git ingestion** (phase 6) ([spec](docs/specs/openscion-git-ingestion.md),
  [plan](docs/plans/openscion-git-ingestion.md),
  [ADR 0005](docs/adr/0005-git-ingestion-reader.md)): the new `openscion-git`
  crate's `import_first_parent`, which walks an existing git repository's
  first-parent spine and writes one OpenScion changeset per spine commit,
  reproducing each commit's tree (file, executable, symlink) by delta from the
  previous spine commit, skipping and counting submodule gitlinks, erroring on a
  non-UTF-8 path, and recording the git commit id, author and committer
  identities, and timestamps in each changeset's metadata. The reader is
  gitoxide (`gix`, pure Rust): chosen over `git2`/libgit2, which fails the
  workspace's cargo-deny license allow-list (GPL-2.0-with-exception) and pulls a
  C library, over shelling out to `git`, and over hand-rolling an object parser;
  the choice is recorded in [ADR 0005](docs/adr/0005-git-ingestion-reader.md).
  The mode mapping and tree-map diff are pure and unit-tested; the driver is
  tested against real git repositories built with the `git` CLI (linear history,
  a merge taking only the spine, kinds, submodules, non-UTF-8, provenance).

- **Daemon binary** ([spec](docs/specs/openscion-daemon-binary.md),
  [plan](docs/plans/openscion-daemon-binary.md)): the `openscion-daemon`
  process — `DaemonConfig` from flags and `OPENSCION_DAEMON_*` env; a
  scheme-prefixed `--listen`/`--upstream` endpoint (`tcp:…`/`unix:…`); boot that
  opens the `NodeStore` on a data directory against an optional tier above,
  warms it from a snapshot file, and binds the listener; thread-per-connection
  serving; and a `SIGINT`/`SIGTERM` shutdown that persists the warm set back to
  the snapshot file. Wiring only over the phase-5 library — no new wire or store
  behaviour — and still tokio-free (`signal-hook` for synchronous shutdown). The
  binary is proven end to end by spawning it (bind, a client request over the
  wire, a signalled shutdown that writes the snapshot). Opens phase 7.

## Next

1. **Network tiers** (phase 7): the distributed `MetadataStore`
   (Postgres/pgrust, Cassandra, or FoundationDB) and the propagation bus, with
   the regional cache tier, upstream-concurrency bounds, and `Unavailable` load
   shedding across nodes. The daemon binary, library, client, server, stores,
   and git ingestion they build on are in place.

   The backend is decided — shard Postgres
   ([ADR 0006](docs/adr/0006-distributed-metadata-store.md)) — but the Postgres
   `MetadataStore` build is **sequenced behind a dependency**: it is proven
   against a real server inside the ordinary (coverage-counting) test run, by
   dogfooding a bundled, in-process, pure-Rust Postgres from the `pgpoly`
   project. `pgpoly`/`pgrust` has no runnable wire server yet, so the required
   embeddable test server is delegated to the `pgpoly` agent (handoff
   `task_openscion_pg_testserver_20261021`). Until that lands, the immediate
   next chunk is a database-free phase-7 piece — upstream-concurrency bounds and
   `Unavailable` load shedding on the node store, or the regional cache tier.

## Later

Network tiers (where the
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

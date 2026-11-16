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

- **Upstream-concurrency bound and `Unavailable` load shedding** (phase 7)
  ([spec](docs/specs/openscion-daemon-upstream-bound.md),
  [plan](docs/plans/openscion-daemon-upstream-bound.md)): the first network-tiers
  chunk makes the daemon's tiering bounded. A `BoundedSource` decorator over the
  node store's upstream caps how many fetches to the tier above run at once with
  a lock-free slot count, and sheds the overflow immediately with a new
  `Error::Unavailable` instead of queueing. The shed crosses the framed transport
  as a distinct `TAG_UNAVAILABLE` response, so a daemon reading through a
  saturated daemon receives `Unavailable` — not a generic upstream error — and,
  because its own upstream is itself bounded, re-sheds to its caller rather than
  stampeding the tier above. The bound is `--upstream-concurrency` /
  `OPENSCION_DAEMON_UPSTREAM_CONCURRENCY` (default 32), wired in `Daemon::bind`;
  the node store, cache, and snapshot are unchanged, and no dependency was added.

- **Shared `openscion-tier` crate** (phase 7 enabler)
  ([spec](docs/specs/openscion-tier.md), [plan](docs/plans/openscion-tier.md)):
  the tier machinery every tier shares — the `ObjectSource` and new
  `ObjectBacking` ports, the read-through object `Store` (generalized over its
  byte backing), the on-disk `ObjectCache`, the `BoundedSource` upstream bound,
  and the framed transport — is extracted out of `openscion-daemon` into a new
  `openscion-tier` crate. The daemon consumes it and re-exports its prior
  surface (`NodeStore` is now `Store<S, ObjectCache>`); behaviour and public API
  are unchanged, and no third-party dependency was added. This unblocks the
  regional cache (an in-memory `ObjectBacking`) and the server's framed endpoint,
  which reuse the crate rather than duplicating it
  ([ADR 0004](docs/adr/0004-daemon-transport.md)).

- **Regional object cache** (phase 7)
  ([spec](docs/specs/openscion-cache.md), [plan](docs/plans/openscion-cache.md)):
  the new `openscion-cache` crate — an in-memory `MemoryBacking` (an
  `ObjectBacking`) and the `RegionalCache` process that fronts it over
  `openscion-tier`. It opens the shared read-through `Store` over the in-memory
  backing, bounds its upstream concurrency, and serves the same framed object
  RPCs as the daemon and the server, so a region's nodes read nearby instead of
  from the server. Unlike the daemon it holds nothing durable: no data directory,
  no snapshot — it starts empty and a restart loses only warmth, exactly as the
  architecture prescribes for an immutable-object cache. Its upstream is
  required (a cache with nothing above it has nothing to cache), and its fetches
  shed with `Unavailable` through the same `BoundedSource`, so a daemon reading
  through a saturated cache re-sheds rather than stampeding it. The shared
  endpoint type (`tcp:`/`unix:` addressing) moved into `openscion-tier` so the
  daemon and the cache parse one form; the daemon re-exports it, unchanged. No
  third-party dependency beyond those the daemon already uses.

- **The server's framed object endpoint** (phase 7)
  ([spec](docs/specs/openscion-server-object-endpoint.md),
  [plan](docs/plans/openscion-server-object-endpoint.md)): `openscion-server`
  now speaks the framed object RPCs, so the read-through chain has an origin. The
  `openscion-tier` transport was widened to serve any `ObjectService` — a trait
  the read-through `Store` implements, so the daemon and cache are unchanged —
  and the server serves a `RepositoryObjects` adapter over its authoritative
  `Repository`. An opt-in `--object-listen` (`tcp:`/`unix:`) binds a second,
  read-only listener beside the HTTP API; its sync accept loop runs on a thread
  alongside the async HTTP server. As the origin it has no tier above and sheds
  nothing. A blob uploaded over HTTP is read back over the framed wire by a tier
  client in an end-to-end test. No third-party dependency beyond `openscion-tier`.

- **`Endpoint::bind` for every tier** (phase 7 cleanup)
  ([spec](docs/specs/openscion-tier-endpoint-bind.md),
  [plan](docs/plans/openscion-tier-endpoint-bind.md)): the daemon, the cache, and
  the server each carried a private copy of the same listener plumbing — a
  `Listener` enum, a `bind_listener`, and a serve-dispatch with its own
  `#[cfg(unix)]`. That is lifted into `openscion-tier` as
  `Endpoint::bind -> BoundListener`, symmetric with the existing
  `Endpoint::dial`: `BoundListener` reports its resolved `tcp:`/`unix:` address
  and serves an `ObjectService`, hiding the transport split in one place. All
  three tiers migrate to it, binding with `endpoint.bind()` and serving with
  `bound.serve(&service)`; each still wraps a bind failure in its own error. The
  change is behaviour-preserving — every tier suite, including the binary
  integration tests, stays green — and resolves the deferred cleanup noted with
  the server's object endpoint. No new dependency.

- **Connection pooling for the framed client** (phase 7 performance)
  ([spec](docs/specs/openscion-tier-connection-pooling.md),
  [plan](docs/plans/openscion-tier-connection-pooling.md)): `SocketSource` opened
  a fresh connection on every fetch, so a clone or checkout reading thousands of
  objects through the daemon paid a TCP/Unix connect per object. It now pools and
  reuses connections across fetches behind a shared mutex, retrying once on a
  fresh connection when a pooled one was closed while idle, so a stale pool costs
  a retry rather than an error. The pool is self-bounding — a connection enters
  it only after being checked out, so it never exceeds the peak concurrent
  fetches the `BoundedSource` above already caps. The wire and every caller are
  unchanged; found/absent/unavailable/upstream replies behave exactly as before.
  No new dependency.

- **Batched object fetch over the framed wire** (phase 7 performance)
  ([spec](docs/specs/openscion-tier-batched-fetch.md),
  [plan](docs/plans/openscion-tier-batched-fetch.md)): a tier that needed many
  objects fetched them one request at a time, so even over a reused connection
  the round-trip latency serialized across every object. A new `get-many` framed
  request carries many ids and is answered with one response frame per id in
  order, reusing the existing found/absent/unavailable/error replies.
  `ObjectSource` gains `fetch_many` — the default fetches one at a time, so every
  source keeps working; `SocketSource` answers a batch in one round-trip over one
  pooled connection, and `BoundedSource` admits or sheds a batch as one upstream
  operation. `Store::warm` fetches in batches, so warming a snapshot is a handful
  of round-trips rather than one per id, with its counts, eviction order, and
  corrupt rejection unchanged. The single-object path is untouched. No new
  dependency.

- **Forwarding non-object RPCs over the framed wire** (phase 7)
  ([spec](docs/specs/openscion-tier-rpc-forwarding.md),
  [plan](docs/plans/openscion-tier-rpc-forwarding.md)): the framed wire carried
  only object reads, so a lower tier could not forward the mutable queries the
  architecture routes to the server. A forwarded-RPC envelope
  (`openscion_core::rpc` — one `Request`/`Response` pair over every non-object
  operation, an engine failure carried as `Response::Error` with the HTTP code)
  now crosses the wire as a postcard-encoded `rpc` frame. `ObjectService` gains a
  declining `rpc` default; the server (`RepositoryObjects`) overrides it to
  dispatch all fourteen operations — create/read changeset; stat, tree, file,
  diff, log, ancestor, merge-base; read/update ref; rebase, revert, land — each
  to the engine call its HTTP handler makes, and a new `Forwarding` service lets
  the daemon and the cache serve objects from their store while relaying RPCs to
  the tier above (a detached tier declines). The object path and all existing
  behaviour are unchanged. The one new dependency is `postcard` (pure-Rust binary
  serde) in the tier crate. The client still calls the server over HTTP; moving
  it onto this path is a later change.

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

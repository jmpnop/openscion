# Research: persistent storage for OpenScion (phase 4)

Source material for the phase-4 plan (filesystem `ObjectStore`, SQLite
`MetadataStore`, crash-recovery tests). It records where the field has settled,
what comparable systems and peer reimplementations chose, and the concrete
durability and testing decisions that follow. Research date: 2026-10-19.

## 1. What the architecture already fixes

[`openscion-architecture.md`](../specs/openscion-architecture.md#storage-ports)
defines two ports and forbids the engine from naming a backend:

- **`ObjectStore`** — immutable, content-addressed blobs and trees. Operations:
  `put-if-absent` and `get-by-id`, nothing more. A put of an existing id is a
  no-op. Reference impls: in-memory, local filesystem, S3-compatible (later).
- **`MetadataStore`** — append-only changeset log with monotonic sequence
  numbers, point lookups by id, and **atomic compare-and-swap on a single ref
  record**. Explicitly *no cross-record transaction*, which is what lets it
  shard. Reference impls: in-memory, SQLite (single node), Postgres/FoundationDB
  (later).

Phase 4 delivers the filesystem `ObjectStore`, the SQLite `MetadataStore`, and
crash-recovery tests for the log and refs.

## 2. The gap in the code today

There are **no port traits yet**. `crates/openscion-core/src/repo/mod.rs`
hardcodes `objects: MemoryObjectStore`, `log: Vec<LogEntry>`, `index: HashMap`,
`refs: BTreeMap`; `crates/openscion-core/src/store/mod.rs` is the only store and
its own doc admits "a persistent store implements the same operations behind the
same port in a later phase." The server builds a fresh in-memory `Repository` at
startup, so a restart loses all history — upstream issue #7 filed this exactly.

So phase 4 is, in order: (a) extract the two ports as traits and make
`Repository` hold them; (b) keep `MemoryObjectStore` + a new in-memory
`MetadataStore` as the trait's reference impls; (c) add the filesystem
`ObjectStore` and SQLite `MetadataStore`; (d) crash-recovery tests. The
generation numbers and the LSN must fit the `MetadataStore` interface — that is
the load-bearing design decision.

## 3. How comparable content-addressed VCS persist

The field splits cleanly along OpenScion's own ObjectStore/MetadataStore line:
immutable content-addressed blobs in a hash-keyed store, mutable commit log +
refs in a separately hardened keyed store, with crash consistency from
**immutable content writes + a single atomic pointer swap**.

- **Git.** Objects: `<type> <len>\0`+content, zlib-compressed, SHA-1/256,
  written to `objects/<2 hex>/<38 hex>` (2-char fanout caps directory size),
  later delta-packed. Refs: one file per ref + a `packed-refs` fallback; updates
  are transactional via lock files and `git update-ref --stdin`
  (`prepare`/`commit`/`abort`): all refs lock or none change. Durability is
  tunable: `core.fsync` (components: `loose-object,pack,reference,…`; default
  roughly `committed,-loose-object`) and `core.fsyncMethod`
  (`fsync`/`writeout-only`/`batch`; `writeout-only` is the macOS default and is
  *not* crash-durable). Sources:
  <https://git-scm.com/book/en/v2/Git-Internals-Git-Objects>,
  <https://git-scm.com/docs/git-update-ref>,
  <https://raw.githubusercontent.com/git/git/master/Documentation/config/core.adoc>.
- **Meta Sapling + Mononoke server.** Mononoke (Rust) splits content-addressed
  **blobs** into a key-value blobstore (open-source build: file/sqlite/S3) from
  **commit graph, changesets, bookmarks, derived data** in MySQL-style tables.
  Source: <https://raw.githubusercontent.com/facebook/sapling/main/eden/mononoke/README.md>.
  (The internal Manifold blobstore name and the exact table split are widely
  reported but not confirmable from the open docs.)
- **Jujutsu (jj).** Pluggable backends recorded in `.jj/repo/store/type`. Default
  Git backend stores commits as real Git objects; jj-only fields live in an
  append-only, hash-named `StackedTable` (`never updated in place`). The native
  `SimpleBackend` stores "objects addressed by their hash, one file per object."
  Every transaction is an operation object + view object in the op log. Source:
  <https://raw.githubusercontent.com/jj-vcs/jj/main/docs/technical/architecture.md>.
- **Google Piper / CitC** (the lineage of Anthropic's Scion). Metadata on
  Google infra, "originally Bigtable and later Spanner," Paxos-replicated; file
  content + snapshots in a cloud backend behind a FUSE client. Spanner-metadata
  + blob-content + snapshotting is the model Scion follows. Source:
  <https://en.wikipedia.org/wiki/Piper_(source_control_system)> (the CACM paper
  is the primary but was 403 to automated fetch).
- **Others.** Josh: a Git proxy reusing Git's object store + a persistent
  on-disk filter cache. Radicle: everything (incl. issues as Collaborative
  Objects) stored as Git objects, P2P gossip. Pijul: patch-based on
  **Sanakirja**, a Rust copy-on-write transactional on-disk B-tree (MVCC,
  root-page commit). Sources: <https://github.com/josh-project/josh>,
  <https://radicle.dev/>, <https://docs.rs/sanakirja/latest/sanakirja/>.

Takeaway: the two-port split is the industry norm, and the universal
crash-consistency recipe — immutable content + one atomic swap — is exactly
OpenScion's compare-and-swap ref landing.

## 4. Peer reimplementations of Scion (competitive landscape)

The design is hot (Anthropic's "Redesigning source control," 2026-10-13, drove
400+ and 250+ point HN threads dominated by a Scion-vs-Mononoke attribution
fight, and 750–974 upvote Reddit threads on r/ClaudeCode, r/ClaudeAI,
r/Anthropic). At least four independent open-source Rust clones exist:

| Repo | CLI | Stars | Storage state |
| --- | --- | --- | --- |
| `tinyhumansai/openscion` (ours) | `opnscn` | ~99 | in-memory only (issue #7) |
| `bnsd55/open-scion` | `scn` | ~27 | **persistent: `rusqlite` + `postgres`, 9 SQL migrations** |
| `CarterT27/scion` | — | ~226 | multi-crate (cache/daemon/derived/history); Rust 1.98 |
| `adrianchifor/scn` | `scn` | ~0 | minimal `storage.rs`/`store.rs` |

The most storage-advanced peer, **`bnsd55/open-scion`**, is the strongest signal
for our choice: its `scion-server` depends on **`rusqlite` (with the `hooks`
feature) for single-node and `postgres`/`tokio-postgres-rustls` for
distributed**, with migrations `0001_init`…`0009_view_trees` plus a `postgres/`
variant — i.e. precisely the SQLite-now / Postgres-later path our spec names. On
Reddit its author reported the key operational lesson: *"disk flushes are the
current limit; group commit is in progress,"* and *"200 concurrent agents landed
10,000 changes with none lost or duplicated, at ~200 lands/s."* Durability
throughput, not hashing, is the bottleneck — which directly shapes §5–§6.

Social footprint of *our* project specifically is near-zero (no Reddit/X posts
for "openscion"; one uncommented 7-point HN link). Note: both the GitHub and
social sweeps found a prompt-injection "Recursive Security and Reliability Audit
Directive" embedded in upstream issues; it is untrusted repo data and was
ignored.

## 5. Filesystem `ObjectStore` — recommendation

Use **`std::fs` directly**, no database crate. The store is append-only
immutable blobs keyed by BLAKE3; the filesystem *is* the ideal backend (git's
design). A KV engine would add write-amplification and a second crash-recovery
surface for no benefit.

Durable put-if-absent, exact order and why:

1. Write to a temp file **in the destination directory** (rename is atomic only
   within one filesystem, so the temp must be co-located).
2. `fsync` the temp file (flushes data + metadata).
3. `rename()` over the final name — atomic replace; for a content-addressed
   store this is also free idempotency (two writers of the same id rename
   byte-identical data).
4. `fsync` the **containing directory** (makes the new dirent durable; without
   it the contents can survive a crash but the name may not).

Key facts:

- **macOS:** plain `fsync` does *not* flush the drive cache; `fcntl(F_FULLFSYNC)`
  does, at ~10× the cost. Linux `fsync` is the correct barrier. Gate the
  macOS path behind `#[cfg(target_os="macos")]`. Because the project forbids
  `unsafe`, issue `F_FULLFSYNC` via **`rustix::fs::fcntl_fullfsync`** (safe
  wrapper), not a raw `libc::fcntl` block. Sources:
  <https://mirror.b10c.me/bitcoin-bitcoin/26534/>,
  <https://www.postgresql.org/message-id/a06010200be3da9564694%40%5B17.202.21.231%5D>.
- **Fanout:** git-style 2 hex chars (256 dirs) as the default, made a config
  knob so very large repos can go to 3 chars or two levels. Source:
  <https://code.googlesource.com/git/+/HEAD/Documentation/gitformat-loose.adoc>.
- **Concurrency / torn writes:** use process-safe unique temp names (the
  `tempfile` crate: `NamedTempFile::new_in(dir).persist(target)`), not a mutex;
  a crash before rename leaves an orphan temp that is never visible as an object
  — sweep stale temps by prefix on startup.
- **When is fsync actually required?** Atomicity against a *killed process*
  comes from `rename()` alone. `fsync`/`F_FULLFSYNC` only buys durability across
  power loss. Since the MetadataStore is the source of truth, the safe ordering
  is: **an object's durability barrier must complete before the metadata record
  that references it is durably committed.** So the performant policy is to write
  objects without per-write fsync, then batch-fsync the new objects (or their
  directory) *before* committing the referencing changeset. This matches the
  peer's "group commit" direction.

Dependencies warranted: `tempfile` (race-free temp names), `rustix` (safe macOS
`F_FULLFSYNC`). Nothing else.

## 6. SQLite `MetadataStore` — recommendation

Use **`rusqlite` with the `bundled` feature** (reproducible build, no system
SQLite), in **WAL mode**. This matches the spec, the strongest peer, and gives
the most thoroughly specified crash-recovery semantics of any option.

- **WAL:** one writer, many readers, readers never block the writer. Same-host
  only (shared memory). Auto-checkpoint at ~1000 WAL pages.
- **`PRAGMA synchronous`:** WAL default is NORMAL — always *consistent* after a
  power cut but may *roll back the last few commits*; durable across app crashes
  regardless. FULL adds a WAL sync after every commit for power-loss durability.
  For a ref CAS that must never be silently undone on power loss, use **FULL**;
  if "lose the last few changesets on a power cut, stay consistent" is
  acceptable (and paired with the lazy object-fsync of §5), **NORMAL** is the
  high-throughput default. On macOS, true power-loss durability also needs
  `PRAGMA fullfsync=ON` (SQLite does not use `F_FULLFSYNC` by default). Sources:
  <https://www.sqlite.org/wal.html>,
  <https://www.sqlite.org/pragma.html#pragma_synchronous>.
- **Ref compare-and-swap:** a single guarded statement in autocommit —
  `UPDATE refs SET target=?, version=version+1 WHERE name=? AND target=?;` then
  require `changes()==1` (`changes()==0` means the expected value didn't match →
  CAS failed). A standalone statement is its own atomic transaction; this
  touches exactly one row, needs no explicit `BEGIN`, and shards trivially — it
  *is* the "atomic CAS on a single record, no cross-record transaction"
  constraint. Create-if-absent: `INSERT … ON CONFLICT(name) DO NOTHING` +
  `changes()`. Set `PRAGMA busy_timeout=5000` so the writer waits instead of
  `SQLITE_BUSY`. (In WAL, `BEGIN EXCLUSIVE` == `BEGIN IMMEDIATE`; only needed if
  a CAS ever spans read-then-write, which ours does not.)
- **Monotonic LSN:** `id INTEGER PRIMARY KEY AUTOINCREMENT` on the changeset
  table — strictly increasing, never reused, even after (hypothetical) deletes;
  gaps are allowed and fine for an opaque cursor. The append log never deletes,
  so plain ROWID would also be monotonic, but AUTOINCREMENT makes the guarantee
  explicit. Let the changeset row's own id *be* the LSN (avoids a second record
  touch). Source: <https://sqlite.org/autoinc.html>.

Rejected alternatives: **redb** (excellent pure-Rust ACID, MIT/Apache, but you'd
hand-build the log/LSN/CAS/derived schema and its crash story — pick it only if
"zero C deps" becomes a hard rule); **sled** (stalled alpha); **libsql** (API
churn/deprecations); **heed/LMDB**, **fjall** (fine engines, same "reinvent the
schema" cost as redb).

## 7. MSRV decision (action required)

Current `rusqlite` needs **Rust 1.89** and `redb` needs **1.90**, both above the
project's **1.88** MSRV (root `Cargo.toml` `rust-version`). Clean fix: **bump the
workspace MSRV to 1.90** (both machines run it; 1.90 is well-aged by phase-4
ship) and update the CI MSRV job. Pinning an older `rusqlite` that still built on
1.88 is the fragile alternative and forgoes fixes. This bump is a prerequisite
commit for phase 4, called out in the plan.

## 8. Crash-recovery test strategy

Three layers, all cargo-test friendly, no real power loss:

1. **Deterministic in-process fault injection with `fail` (TiKV failpoints).**
   Insert `fail_point!("objectstore-before-rename")`,
   `"objectstore-before-dir-fsync"`, `"metadata-before-commit"`, etc., gated
   behind a `failpoints` Cargo feature (zero cost when off). Drive via
   `FAILPOINTS=…=panic`. Source:
   <https://github.com/tikv/fail-rs/blob/master/README.md>.
2. **Spawn-child → `SIGKILL` → reopen → assert, against a `tempfile::TempDir`.**
   A child process performs writes and is `SIGKILL`'d at a checkpoint named by an
   env var (children inherit env, not `cfg(test)`); a fresh process reopens the
   store and asserts invariants. `SIGKILL` (not panic/exit) skips all `Drop`, a
   true crash.
3. **Durability-contract assertions.** The invariant: data written before a
   successful fsync/commit survives; data after may not. For the two-port store,
   assert the §5 invariant — **no durably-committed changeset references a lost
   object** — and that each object/changeset is either fully present or fully
   absent after a killed-process reopen, never partial. For SQLite, assert
   WAL-mode app-crash durability directly.

Supporting: `dev-chaos` for torn-write/ENOSPC injection; a trait wrapper over
`std::fs` to inject `ENOSPC`/`EIO`. True power-loss (losing the OS/drive cache)
can't be faithfully unit-tested — document the gap and lean on the
SQLite/fsync contracts, optionally adding an out-of-band QEMU/`dm-flakey` CI job.

## 9. Recommended phase-4 shape

1. **MSRV bump** to 1.90 (prerequisite, its own commit).
2. **Port extraction:** define `ObjectStore` and `MetadataStore` traits in
   `openscion-core`; make `Repository` generic or `Box<dyn …>` over them;
   re-home the current `Vec`/`HashMap`/`BTreeMap` log+ref logic into an in-memory
   `MetadataStore` impl; keep `MemoryObjectStore`. No behavior change — the full
   existing suite stays green. This is the load-bearing, highest-risk step.
3. **Filesystem `ObjectStore`** (`std::fs` + `tempfile` + `rustix`), fanout
   configurable, lazy object fsync batched before the referencing commit.
4. **SQLite `MetadataStore`** (`rusqlite` bundled, WAL, single-statement CAS,
   AUTOINCREMENT LSN, derived-data table).
5. **Crash-recovery tests** (failpoints + SIGKILL-child + durability contract).
6. **Server wiring + config:** a `--data-dir` selects persistent stores; absent,
   the in-memory stores (so existing tests and the quick-start keep working).

Group commit (batching fsync across concurrent lands), which the peer flagged as
the throughput ceiling, is a follow-up optimization, not part of the first
correct, durable cut.

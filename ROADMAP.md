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

## Next

1. **First-parent skip lists**, if query profiles show the generation-pruned
   walks need them.

## Later

Persistent stores, the `opnscn` client and node daemon, git ingestion, network
tiers, line-level blame and merge, virtual checkouts, read access control. A
DuckDB-backed analytics plane ([ADR 0002](docs/adr/0002-duckdb-analytics-plane.md))
and a Telegram cold object tier ([ADR 0003](docs/adr/0003-telegram-object-tier.md))
are optional adjacent systems, not part of the serving path.

## Out Of Scope

- Replacing git for ordinary repositories.
- Offline-first operation and branch-heavy workflows.
- Code review, code search, and CI themselves. OpenScion provides the APIs
  they need.

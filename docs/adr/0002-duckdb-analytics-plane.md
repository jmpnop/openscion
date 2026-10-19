# 2. DuckDB as an optional analytics plane, not a store

- **Status:** Accepted
- **Date:** 2026-10-18

## Context

OpenScion targets agent-scale monorepos: tens of thousands of changes a day and
history in the tens of gigabytes. Beyond serving source control, there is real
demand to ask analytical questions across that history — churn and hotspots,
files that change together, ownership maps, landing and CI throughput, repo
health over time — to feed dashboards such as `apps/web`.

The question is where, if anywhere, DuckDB (an embeddable columnar OLAP engine,
Parquet-native, SQL) fits in the architecture of two storage ports: the
immutable content-addressed `ObjectStore` and the transactional `MetadataStore`
(the log, refs moved by compare-and-swap, derived indexes).

## Decision

DuckDB is **not** part of the serving path and backs **neither** port.

- `ObjectStore` is a content-addressed blob store (filesystem/S3/RocksDB);
  DuckDB is the wrong shape for it.
- `MetadataStore` is OLTP: point lookups, many small concurrent compare-and-swap
  writes (the land loop), crash recovery. SQLite is the reference; DuckDB's
  single-writer OLAP model is unsuitable.
- The serving queries (`Log`, `Diff`, `IsAncestor`, path history) are graph and
  tree walks pruned by generation numbers, not columnar scans; DuckDB would not
  accelerate them.

DuckDB may be adopted **only** as an optional, read-only **analytics plane**: a
separate crate that ingests an append-only export of changeset and change
records (naturally Parquet-friendly) and answers analytical SQL, decoupled from
serving and off the hot path. It is an adjacent system, like code search or CI,
that consumes OpenScion's APIs rather than changing its design.

## Consequences

- The core crates stay free of a heavy native dependency; the analytics plane
  lives in its own feature-gated crate and cannot affect serving latency or
  correctness.
- Analytical workloads that would be painful as engine walks (cross-history
  aggregation) get a columnar engine built for them.
- Analytics are eventually consistent with the log, by design: the plane reads
  an export, not the live authoritative state.

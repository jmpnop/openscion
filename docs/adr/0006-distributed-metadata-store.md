# 6. Shard Postgres for the distributed metadata store

- **Status:** Accepted
- **Date:** 2026-10-20

## Context

Phase 7 takes the `MetadataStore` across machines. Phase 4 defined the port and
shipped two single-node implementations (in-memory and SQLite); the distributed
tier needs a third. The architecture allows a short list of backends —
"Postgres/pgrust, Cassandra, or FoundationDB" — and this ADR settles which.

The decision turns entirely on what the port actually requires. The persistence
research fixes it:

> `MetadataStore` — append-only changeset log with monotonic sequence numbers,
> point lookups by id, and **atomic compare-and-swap on a single ref record**.
> Explicitly *no cross-record transaction*, which is what lets it shard.

So the only hot paths are (1) a single-record ref compare-and-swap — the `Land`
retry loop — (2) a monotonic-LSN log append, and (3) a point read by id. No
multi-key transaction is ever needed, and the port is deliberately shardable.

Each candidate was evaluated against those three operations (research date
2026-10-20; sources in the phase-7 spec):

- **Cassandra.** Its only compare-and-swap is a lightweight transaction
  (Paxos): roughly four coordinator↔replica round trips and about 4× the latency
  of a plain write, worse under contention and across a WAN — which Cassandra's
  own 4.1 notes call out. The ref-landing path is a contended single-key CAS,
  i.e. Cassandra's single weakest operation, and a cluster-wide monotonic
  sequence is unnatural on it. Its strength (plain-write throughput) only helps
  the log-append leg. Wrong fit for this contract.
- **FoundationDB.** The highest ceiling and the most natural LSN: versionstamps
  are a commit-time, monotonically increasing value written conflict-free in a
  single write, and its optimistic read+write-then-retry maps onto the existing
  `Land` loop. But its multi-key strict-serializable transactions — its whole
  reason for being — go unused here, and it carries real operational complexity
  (coordinators, proxies, resolvers, storage roles) and transaction limits
  (~5 s / 10 MB, Ratekeeper throttling).
- **Postgres.** A single `UPDATE … WHERE old = expected` is an atomic,
  linearizable single-row CAS with no consensus protocol and no retry storm, and
  `BIGSERIAL`/sequences give the monotonic LSN natively. A single primary tops
  out around 2,000 fully-durable write TPS, but the port forbids cross-record
  transactions *precisely so it can shard*: the log and refs key by repository,
  so many independent Postgres shards scale writes horizontally with no
  distributed transaction between them. Mature, and its Rust client
  (`tokio-postgres`) is MIT/Apache, so it passes the supply-chain gate.

"pgrust" is not a distributed Postgres — it is a single-node from-scratch Rust
rewrite, AGPL-3.0, and self-described as not yet production-ready; AGPL would
also fail the workspace license allow-list if linked. It is a possible future
drop-in for the Postgres wire, not a backend of its own.

## Decision

Implement the distributed `MetadataStore` on **Postgres, sharded by repository**
at the application layer.

Each shard is an ordinary Postgres database holding the changeset log (point
lookups by id, a `BIGSERIAL` LSN), the ref records (single-row compare-and-swap
via a conditional `UPDATE`), and the derived data. A repository lives on one
shard, so every operation the port defines is a single-shard, single-row or
single-append statement — never a cross-shard transaction, which the port
forbids anyway. Routing maps a repository to its shard; adding shards adds write
capacity linearly.

The Rust driver is `tokio-postgres` (or `sqlx` with the Postgres backend),
pinned in `[workspace.dependencies]`; both are permissively licensed and pass
`cargo deny`.

## Consequences

- A new Postgres `MetadataStore` implementation sits behind a Cargo feature, as
  the SQLite store does, implementing the same port the engine already depends
  on. The engine and every tier above are unchanged.
- Correctness tests against a real Postgres are live tests, gated behind a
  feature/env var and named `live_*` per the testing policy, so the default
  `cargo test` and CI coverage run need no database; the shard-routing and
  SQL-building logic is pure and unit-tested without one.
- Sharding is app-level routing, not a distributed transaction. The spec pins
  the shard key (repository) and the routing rule; cross-shard reads for
  fleet-wide questions are out of scope for the serving path.
- FoundationDB remains the documented scale-ceiling alternative if a single
  elastic cluster is ever preferred over many shards; the port is unchanged, so
  switching is a new implementation of the same trait, not a redesign.
- `pgrust` can later replace the Postgres server under the same wire protocol if
  it matures and its licensing is resolved, with no change above the driver.

# Specifications

| Spec | Status |
| --- | --- |
| [OpenScion architecture](openscion-architecture.md) | Draft |
| [OpenScion data model](openscion-data-model.md) | Draft |
| [OpenScion API](openscion-api.md) | Draft |
| [Working-copy status](openscion-client-status.md) | Accepted |
| [Watcher-backed status](openscion-client-watch.md) | Accepted |
| [Sparse checkout management](openscion-client-sparse.md) | Accepted |
| [Node-local object store and snapshot boot](openscion-daemon-node-store.md) | Accepted |
| [Daemon framed transport](openscion-daemon-transport.md) | Accepted |

Specifications define what the system must do before implementation details
take over. Create one for behavior that changes a public API, crosses module
boundaries, introduces a durable data format, or has meaningful operational
constraints.

Use a short kebab-case filename such as `retry-policy.md`. Each specification
should contain:

1. **Status and owner** — Draft, Accepted, Implemented, or Superseded.
2. **Problem** — the user or system need, without prescribing a solution.
3. **Goals and non-goals** — the exact boundary of the work.
4. **Proposed behavior** — public API, inputs, outputs, errors, and examples.
5. **Invariants and constraints** — properties every implementation must keep.
6. **Acceptance criteria** — externally observable pass/fail conditions.
7. **Open questions** — unresolved decisions that block acceptance.

After the specification is accepted, create a linked implementation plan in
[`../plans/`](../plans/README.md). Keep code snippets small enough to clarify
the contract; production code still belongs under `src/`.

See [`example-retry-policy.md`](example-retry-policy.md) for a complete sample.

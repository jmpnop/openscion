# Implementation plans

Plans turn an accepted specification into a reviewable sequence of small,
verifiable changes. They explain how to build the behavior; the linked
specification remains the source of truth for what the behavior must be.

Use the same kebab-case stem as the specification. A useful plan includes:

- a link to the accepted specification;
- the goal, non-goals, and assumptions relevant to implementation;
- ordered tasks with exact file paths;
- a failing test before each behavior change;
- the minimal implementation needed to pass that test;
- documentation and public-export updates;
- focused and full verification commands;
- a completion checklist updated as tasks land.

Prefer tasks that can be implemented and reviewed independently. Include short
code snippets when they remove ambiguity, but do not paste entire future files
into the plan.

See [`example-retry-policy.md`](example-retry-policy.md) for a test-first sample.

## Plans

- [`openscion-data-model.md`](openscion-data-model.md) — the core data model, server
  routes, `opnscn` import / commit / clone, and the benchmark backend.
- [`openscion-client-status.md`](openscion-client-status.md) — `opnscn status`, the
  read-only working-copy change report (phase 5).
- [`openscion-client-watch.md`](openscion-client-watch.md) — `opnscn watch`, the
  filesystem-watcher-backed live status (phase 5).
- [`openscion-client-sparse.md`](openscion-client-sparse.md) — `opnscn sparse`,
  first-class sparse-set management: show, add, remove (phase 5).
- [`openscion-daemon-node-store.md`](openscion-daemon-node-store.md) — the
  daemon's bounded, read-through node-local object store and snapshot boot
  (phase 5).
- [`openscion-daemon-transport.md`](openscion-daemon-transport.md) — the framed
  object-RPC transport over Unix and TCP, concluding phase 5.
- [`openscion-git-ingestion.md`](openscion-git-ingestion.md) — first-parent
  import of an existing git repository into OpenScion (phase 6).

# Plan: forwarding non-object RPCs over the framed wire

Implements
[`docs/specs/openscion-tier-rpc-forwarding.md`](../specs/openscion-tier-rpc-forwarding.md).
The specification is the source of truth; this plan is the test-first build
order, in three phases.

## Phase A — the envelope and the framed protocol

1. `openscion-proto`: a new `rpc` family (`Request`, `Response`) with a variant
   per non-object operation and a `Response::Error(ErrorBody)`, deriving serde;
   a round-trip test. Export the module from proto and re-export it from core.
2. `openscion-tier`: add `postcard` (workspace dep, `alloc`, no default features).
   Add `TAG_RPC`/`TAG_RPC_REPLY`; `serve_connection` decodes the request, calls
   `service.rpc`, and encodes the reply. `ObjectService::rpc(Request) ->
   Response` with a declining default. `SocketSource::rpc` sends one `rpc` frame
   and decodes the reply over a pooled connection (via `with_connection`). Tests:
   a declined RPC is an `Unsupported` error; a typed response round-trips over
   the wire.

## Phase B — the server answers from its engine

In `openscion-server`, `RepositoryObjects` overrides `rpc`: a `dispatch`/`run`
pair maps each `Request` variant to the `Repository` call its HTTP handler makes
(create/get changeset; stat, list_tree, read_file, diff, log, ancestor,
merge_base; get/update ref; rebase, revert, land), shaping the result into the
matching `Response` and rendering an engine error as `Response::Error` with
`error.code()`/`to_string()`. Test first: a missing ref is a `NotFound` error;
a created changeset reads back.

## Phase C — the daemon and the cache forward

1. `openscion-tier`: a `Forwarding<S, B>` object service wrapping an
   `Arc<Store<S, B>>` and an `Option<SocketSource>`: `get` serves from the store,
   `rpc` relays to the upstream or declines when `None`. `#[derive(Debug)]`;
   export it. Tests: a forwarding tier relays an RPC to a served origin; a
   tier with no upstream declines.
2. `openscion-daemon`, `openscion-cache`: dial the tier above for forwarding
   (the daemon's is `Option`, the cache's is required), and serve a `Forwarding`
   over the store instead of the bare store. Object serving, warm boot, snapshot,
   and shedding are unchanged.
3. `openscion-server` integration test: a tier client sends an RPC over the
   server's framed endpoint and receives the engine's answer end-to-end.

## Phase D — docs and roadmap

- `docs/specs/README.md`, `docs/plans/README.md`: add the rows.
- `ROADMAP.md`: add the Done entry.

## Verification

```sh
cargo test -p openscion-proto -p openscion-tier -p openscion-daemon -p openscion-cache -p openscion-server --all-features
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features
cargo test --all-features
env RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps --all-features
.github/scripts/check-file-coverage.sh 90 <report>
cargo deny check
```

## Completion checklist

- [ ] Phase A — envelope, framed `rpc` protocol, client, tests.
- [ ] Phase B — server dispatch of all 14 operations, tests.
- [ ] Phase C — `Forwarding`, daemon and cache wiring, end-to-end test.
- [ ] Phase D — docs and roadmap.
- [ ] Full contract green; per-file coverage ≥ 90%; `cargo deny` clean.

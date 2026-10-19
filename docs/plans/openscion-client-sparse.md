# Plan: sparse checkout management

- **Status:** Done
- **Specification:**
  [`../specs/openscion-client-sparse.md`](../specs/openscion-client-sparse.md)
- **Phase:** 5 (client and daemon), third chunk.

## Goal

Make the sparse checkout a first-class, editable part of the working copy: a
set the copy owns and can show, widened by `opnscn sparse add` (fetch and
materialize) and narrowed by `opnscn sparse remove` (delete locally), with one
membership rule shared by clone, add, and remove, and backward compatibility
with the old single-string form.

## Approach

A `Sparse` type owns the set and its one membership rule — `Full`, or
`Cone(Vec<String>)` normalized to a duplicate-free antichain of prefixes. A repo
path belongs to a cone when it equals or lies under a prefix. `Sparse` carries
its own serde that reads the legacy forms (a string → a one-entry cone, null or
absent → full) and writes the set as an array.

The fetch-and-write half of `clone` — walk the snapshot under a filter, fetch
the blobs in batches, verify, write, and build the tracked-file entries — is
factored into one `fetch_into` reused by `clone` and `sparse add`. `remove` is
local: it deletes the entry's materialized files, prunes emptied directories,
and drops them from the tracked set. Neither `commit`, `status`, nor `watch`
changes: they key off the tracked files, which add and remove keep correct.

## Tasks

Test-first: the failing test, then the code that passes it.

1. **The set type** — `crates/openscion/src/workcopy/sparse.rs`: `Sparse`
   (`Full` / `Cone`), `is_full`, `prefixes`, `covers(path)`, `insert(prefix)`
   (normalizing), `remove(prefix) -> bool`, and `covers` the shared membership
   rule. Custom serde reading string/null/array and writing null/array.
   Unit-test membership, normalization (nesting collapses), remove, and the
   legacy/round-trip serde. Wire `mod sparse;` from `workcopy/mod.rs`.
2. **State field** — `crates/openscion/src/workcopy/types.rs`: `State.sparse`
   becomes `Sparse` with `#[serde(default)]`. Update the constructors in
   `commands/clone.rs`, `commands/import.rs`, and the test states.
3. **Reusable fetch** — `crates/openscion/src/commands/clone.rs`: extract
   `fetch_into(client, root_tree, filter, dest) -> Result<BTreeMap<..>>` from
   `clone`, and have `clone` record `state.sparse` as `Full` or a one-entry
   cone.
4. **Sparse commands** — `crates/openscion/src/commands/sparse.rs`: `show`,
   `add`, and `remove`; a `remove_file` helper in `workcopy` that deletes a file
   and prunes emptied parents. New errors `Error::FullCheckout` and
   `Error::NotInSparse`. Test add (materializes and tracks), remove (deletes,
   untracks, prunes, leaves status clean), the full-checkout errors, and the
   not-an-entry error, against a real server.
5. **CLI** — `crates/openscion/src/cli/mod.rs`: `Command::Sparse { action }`
   with `Show` / `Add { paths }` / `Remove { paths }`, dispatch and formatting.
   Test the three forms end to end.
6. **Docs** — module comments; `lib.rs` command table; `ROADMAP.md`; spec and
   plan indexed.

## Verification

- Focused: `cargo test -p openscion sparse`.
- Full contract: `cargo fmt --all -- --check`, `cargo clippy --all-targets
  --all-features -- -D warnings`, `cargo build --all-targets --all-features`,
  `cargo test --all-features`.
- Rustdoc: `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps --all-features`.
- Per-file coverage ≥90% on every touched source file.

## Checklist

- [x] one membership rule shared by clone, add, and remove
- [x] add fetches and tracks; remove deletes, untracks, and prunes
- [x] the set is normalized; the legacy single-string form still loads
- [x] add/remove error on a full checkout; remove errors on a non-entry
- [x] status stays clean after add and after remove
- [x] fmt, clippy, build, full test suite, rustdoc green
- [x] per-file coverage ≥90%

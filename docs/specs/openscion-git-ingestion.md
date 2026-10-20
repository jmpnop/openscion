# Git ingestion (first-parent import)

- **Status:** Accepted
- **Owner:** OpenScion
- **Specifications it builds on:**
  [architecture](openscion-architecture.md) (phase 6), [data
  model](openscion-data-model.md) (changesets as changes against a first
  parent, trees, blobs, `EntryKind`).
- **Decision record:** [ADR 0005](../adr/0005-git-ingestion-reader.md) — git is
  read with `gix`.

## Problem

Teams arrive with history in git. To adopt OpenScion they need that history
inside it — not as an opaque blob, but as real OpenScion changesets they can
`log`, `diff`, and build on. OpenScion's model is first-parent: a changeset has
a first parent and records its changes against that parent's tree. Git's history
is a DAG, but it has a first-parent spine — the mainline a sequence of merges
produces. Importing that spine turns a git branch into an OpenScion ref whose
history mirrors git's mainline, with each commit's tree faithfully reproduced.

## Goals and non-goals

### Goals

- Import a git repository's first-parent history, from the root commit to a
  chosen tip, into OpenScion changesets on a named ref: one OpenScion changeset
  per git commit on the spine, in order, each recording the tree changes from
  the previous spine commit.
- Reproduce each commit's tree exactly: every regular file, executable, and
  symlink on the spine round-trips to the same bytes and kind.
- Preserve provenance: the original git commit id, author and committer
  identities, and author and commit timestamps travel in changeset metadata,
  since OpenScion stamps its own creation time.
- Be a library that drives a `Repository` (put blobs, create changesets, move
  the ref), usable later by a CLI or server command.

### Non-goals

- Importing the full DAG. Only the first-parent spine is imported; a merge's
  second and later parents, and the side branches they reach, are not walked.
  (The merged *content* is still captured, because the merge commit's own tree
  is imported.) Full-DAG import is a later possibility.
- Rewriting or verifying git's own hashes into OpenScion ids — the two are
  different content-addressing schemes; the git id is kept as provenance, not
  reused.
- Incremental or resumable import, tags, notes, and reflogs.
- Submodules. A gitlink (a commit entry inside a tree) has no OpenScion
  equivalent; it is skipped and counted, not imported as content.

## Proposed behaviour

### Entry point

```rust
pub fn import_first_parent(
    repo: &Path,
    into: &mut Repository,
    start: &str,
    ref_name: &str,
) -> Result<Imported>;

pub struct Imported {
    pub changesets: usize,
    pub head: ObjectId,
    pub skipped_submodules: usize,
}
```

`repo` is the path to the git repository, `start` names the git commit to import
up to (a ref name such as `HEAD` or `main`, or a commit id), `into` is the
OpenScion repository to import into, and `ref_name` is the OpenScion ref to
point at the imported tip. `Imported` reports how many changesets were created,
the id of the tip changeset, and how many submodule entries were skipped.

### What it does

1. Open the git repository and resolve `start` to a commit.
2. Collect that commit's first-parent spine — follow the first parent from the
   tip to the root — and import oldest first.
3. For each spine commit, flatten its tree to the set of tracked paths, each
   with its kind and content. The root commit's changes are every path as an
   upsert; a later commit's changes are the difference from the previous spine
   commit's tree — paths added or changed become upserts, paths removed become
   deletes.
4. Put each upserted file's bytes into the object store and create an OpenScion
   changeset whose first parent is the previous imported changeset (none for the
   root), carrying the git author rendered as `Name <email>` and the commit
   message, and metadata recording the git commit id, the author and committer
   identities, and the author and commit times.
5. After the last commit, point `ref_name` at the tip changeset.

### Translating git entries

| Git mode | OpenScion |
| --- | --- |
| regular file (`100644`) | `EntryKind::File`, blob = the file bytes |
| executable (`100755`) | `EntryKind::Executable`, blob = the file bytes |
| symlink (`120000`) | `EntryKind::Symlink`, blob = the link target bytes |
| directory (`040000`) | descended into; not itself an entry |
| gitlink / submodule (`160000`) | skipped and counted |

A path that is not valid UTF-8 is an error; the import stops and reports it
rather than lose or mangle a name. A symlink's git blob content is already its
target path, which is exactly how OpenScion stores a symlink, so it round-trips
unchanged.

### An empty commit

A spine commit whose tree equals its first parent's (a git empty commit, or a
merge that changed nothing on the mainline) still becomes a changeset — with no
changes — so the imported history has one changeset per spine commit.

## Invariants and constraints

- **One changeset per spine commit, in order.** The number of changesets equals
  the number of commits on the first-parent spine, and their first-parent chain
  is that spine's order.
- **Faithful trees.** After import, the OpenScion snapshot at each imported
  changeset has exactly the paths, kinds, and bytes of the corresponding git
  commit's tree, excluding skipped submodule entries.
- **Provenance preserved, not conflated.** The git commit id and the git
  identities and timestamps are recorded in changeset metadata; OpenScion ids
  and `created_at` are OpenScion's own. The two addressing schemes are never
  equated.
- **Content-addressed as everywhere.** Blobs are stored and addressed by their
  OpenScion BLAKE3 id through the normal object store path; nothing about import
  bypasses verification.
- **Reads git, drives the repository.** The ingester issues only `Repository`
  operations (`put_blob`, `create_changeset`, `update_ref`); it never writes the
  stores directly, so an import obeys the same invariants as any other writer.

## Acceptance criteria

- Importing a linear git history of N commits yields N changesets on `ref_name`,
  whose first-parent `log` matches git's `log --first-parent` order, with each
  commit's message and a `Name <email>` author.
- A clone of the imported ref reproduces the tip commit's tree byte for byte,
  including an executable bit and a symlink target.
- Importing a history with a merge imports only the first-parent spine; the
  merge commit's own tree is reproduced, and `skipped`/second-parent branches
  are not added as changesets.
- A submodule entry is skipped and counted in `Imported::skipped_submodules`,
  and the rest of that commit's tree imports normally.
- Each imported changeset's metadata carries the git commit id and the git
  author and commit timestamps.
- A non-UTF-8 path aborts the import with a clear error.

## Open questions

- **Full-DAG import.** Walking merge parents and side branches to reproduce the
  whole graph, mapping git's multi-parent commits to OpenScion changesets with
  multiple parents, once merge semantics are defined on the OpenScion side.
- **Tags and annotated-tag objects**, mapped to OpenScion refs.
- **Incremental re-import** that appends new git commits to an
  already-imported ref without redoing the whole spine.
- **Driving it** from `opnscn import --from-git` or a server route, once the
  library lands.

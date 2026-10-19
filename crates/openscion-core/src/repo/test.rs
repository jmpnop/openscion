//! Unit tests for the repository: validity rules, root-tree derivation, the
//! log, and compare-and-swap refs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;

use openscion_proto::{
    Change, ChangesetInfo, CreateChangeset, EntryKind, ObjectId, ObjectKind, RepoPath, UpdateRef,
};

use super::{MAX_CHANGES, MAX_TEXT, Repository};
use crate::{Error, FixedClock, encode_tree, object_id};

fn path(text: &str) -> RepoPath {
    RepoPath::new(text).unwrap()
}

fn upsert(repo: &mut Repository, at: &str, contents: &[u8]) -> Change {
    Change::Upsert {
        path: path(at),
        kind: EntryKind::File,
        blob: repo.put_blob(contents).unwrap(),
    }
}

fn delete(at: &str) -> Change {
    Change::Delete { path: path(at) }
}

fn commit(
    repo: &mut Repository,
    parents: &[ChangesetInfo],
    changes: Vec<Change>,
) -> crate::Result<ChangesetInfo> {
    repo.create_changeset(CreateChangeset::new(
        parents.iter().map(|info| info.id).collect(),
        changes,
        "test",
        "message",
    ))
}

/// Every file in a snapshot as `(path, kind, blob)`, in path order.
fn files(repo: &Repository, tree: ObjectId) -> Vec<(String, EntryKind, ObjectId)> {
    let mut out = Vec::new();
    walk(repo, tree, "", &mut out);
    out
}

fn walk(
    repo: &Repository,
    tree: ObjectId,
    prefix: &str,
    out: &mut Vec<(String, EntryKind, ObjectId)>,
) {
    for entry in repo.tree(tree).unwrap() {
        let full = format!("{prefix}{}", entry.name);
        if entry.kind == EntryKind::Tree {
            walk(repo, entry.id, &format!("{full}/"), out);
        } else {
            out.push((full, entry.kind, entry.id));
        }
    }
}

fn names(repo: &Repository, tree: ObjectId) -> Vec<String> {
    files(repo, tree)
        .into_iter()
        .map(|(name, ..)| name)
        .collect()
}

fn rule(result: crate::Result<ChangesetInfo>) -> &'static str {
    match result.unwrap_err() {
        Error::InvalidChange { rule, .. } => rule,
        other => panic!("unexpected error {other:?}"),
    }
}

fn seeded() -> (Repository, ChangesetInfo) {
    let mut repo = Repository::with_clock(FixedClock(1));
    let changes = vec![
        upsert(&mut repo, "a/x.txt", b"x"),
        upsert(&mut repo, "a/y.txt", b"y"),
        upsert(&mut repo, "b/deep/z.txt", b"z"),
        upsert(&mut repo, "top.txt", b"top"),
    ];
    let root = commit(&mut repo, &[], changes).unwrap();
    (repo, root)
}

#[test]
fn the_first_changeset_builds_a_nested_snapshot() {
    let (repo, root) = seeded();
    assert_eq!(root.lsn, 1);
    assert_eq!(
        names(&repo, root.root_tree),
        ["a/x.txt", "a/y.txt", "b/deep/z.txt", "top.txt"]
    );
    assert_eq!(repo.log_len().unwrap(), 1);
}

#[test]
fn untouched_subtrees_keep_their_ids() {
    let (mut repo, root) = seeded();
    let before = repo.tree(root.root_tree).unwrap();
    let change = upsert(&mut repo, "a/x.txt", b"x2");
    let next = commit(&mut repo, &[root], vec![change]).unwrap();
    let after = repo.tree(next.root_tree).unwrap();
    assert_eq!(next.lsn, 2);
    assert_ne!(before[0].id, after[0].id, "a/ was rewritten");
    assert_eq!(before[1].id, after[1].id, "b/ is shared");
    assert_eq!(before[2].id, after[2].id, "top.txt is shared");
}

#[test]
fn deleting_the_last_file_removes_its_directories() {
    let (mut repo, root) = seeded();
    let next = commit(&mut repo, &[root], vec![delete("b/deep/z.txt")]).unwrap();
    assert_eq!(
        names(&repo, next.root_tree),
        ["a/x.txt", "a/y.txt", "top.txt"]
    );
}

#[test]
fn deleting_everything_leaves_the_empty_tree() {
    let mut repo = Repository::new();
    let change = upsert(&mut repo, "only.txt", b"1");
    let root = commit(&mut repo, &[], vec![change]).unwrap();
    let next = commit(&mut repo, &[root], vec![delete("only.txt")]).unwrap();
    assert_eq!(next.root_tree, object_id(ObjectKind::Tree, b""));
    assert!(repo.tree(next.root_tree).unwrap().is_empty());
}

#[test]
fn a_file_can_become_a_directory_in_one_changeset() {
    let (mut repo, root) = seeded();
    let change = upsert(&mut repo, "top.txt/inner", b"i");
    let next = commit(&mut repo, &[root], vec![delete("top.txt"), change]).unwrap();
    assert!(names(&repo, next.root_tree).contains(&"top.txt/inner".to_owned()));
}

#[test]
fn a_directory_can_become_a_file_in_one_changeset() {
    let (mut repo, root) = seeded();
    let change = upsert(&mut repo, "b", b"now a file");
    let next = commit(&mut repo, &[root], vec![change, delete("b/deep/z.txt")]).unwrap();
    assert_eq!(
        names(&repo, next.root_tree),
        ["a/x.txt", "a/y.txt", "b", "top.txt"]
    );
}

#[test]
fn kinds_are_recorded_and_can_change() {
    let (mut repo, root) = seeded();
    let blob = repo.put_blob(b"top").unwrap();
    let change = Change::Upsert {
        path: path("top.txt"),
        kind: EntryKind::Executable,
        blob,
    };
    let next = commit(&mut repo, &[root], vec![change]).unwrap();
    let top = files(&repo, next.root_tree).pop().unwrap();
    assert_eq!(top, ("top.txt".to_owned(), EntryKind::Executable, blob));
}

#[test]
fn rejects_a_second_parentless_changeset() {
    let (mut repo, _) = seeded();
    let change = upsert(&mut repo, "n", b"n");
    assert_eq!(
        rule(commit(&mut repo, &[], vec![change])),
        "only the first changeset may have no parents"
    );
}

#[test]
fn rejects_a_missing_parent() {
    let mut repo = Repository::new();
    let change = upsert(&mut repo, "n", b"n");
    let ghost = ObjectId::from_bytes([7; 32]);
    let result = repo.create_changeset(CreateChangeset::new(vec![ghost], vec![change], "t", "m"));
    assert_eq!(result.unwrap_err(), Error::NotFound(ghost));
}

#[test]
fn rejects_an_empty_non_merge() {
    let (mut repo, root) = seeded();
    assert_eq!(
        rule(commit(&mut repo, &[root], vec![])),
        "only a merge may have no changes"
    );
}

#[test]
fn accepts_an_empty_merge() {
    let (mut repo, root) = seeded();
    let change = upsert(&mut repo, "side", b"s");
    let side = commit(&mut repo, &[root], vec![change]).unwrap();
    let merge = commit(&mut repo, &[root, side], vec![]).unwrap();
    assert_eq!(merge.root_tree, root.root_tree);
}

#[test]
fn rejects_too_many_changes() {
    let (mut repo, root) = seeded();
    let changes = (0..=MAX_CHANGES)
        .map(|i| delete(&format!("f{i}")))
        .collect();
    assert_eq!(
        rule(commit(&mut repo, &[root], changes)),
        "too many changes"
    );
}

#[test]
fn rejects_oversized_text() {
    let (mut repo, root) = seeded();
    let change = upsert(&mut repo, "top.txt", b"new");
    let mut request = CreateChangeset::new(vec![root.id], vec![change], "t", "m");
    request.message = "m".repeat(MAX_TEXT + 1);
    assert_eq!(
        rule(repo.create_changeset(request)),
        "text or metadata too large"
    );
}

#[test]
fn rejects_a_path_changed_twice() {
    let (mut repo, root) = seeded();
    let change = upsert(&mut repo, "top.txt", b"new");
    assert_eq!(
        rule(commit(&mut repo, &[root], vec![change, delete("top.txt")])),
        "path changed twice"
    );
}

#[test]
fn rejects_an_upsert_of_a_tree_kind() {
    let (mut repo, root) = seeded();
    let blob = repo.put_blob(b"t").unwrap();
    let change = Change::Upsert {
        path: path("t"),
        kind: EntryKind::Tree,
        blob,
    };
    assert_eq!(
        rule(commit(&mut repo, &[root], vec![change])),
        "upsert of a tree"
    );
}

#[test]
fn rejects_an_upsert_of_a_missing_blob() {
    let (mut repo, root) = seeded();
    let ghost = ObjectId::from_bytes([8; 32]);
    let change = Change::Upsert {
        path: path("t"),
        kind: EntryKind::File,
        blob: ghost,
    };
    assert_eq!(
        commit(&mut repo, &[root], vec![change]).unwrap_err(),
        Error::NotFound(ghost)
    );
}

#[test]
fn rejects_deleting_a_missing_file() {
    let (mut repo, root) = seeded();
    assert_eq!(
        rule(commit(&mut repo, &[root], vec![delete("nope")])),
        "deletes a missing file"
    );
    assert_eq!(
        rule(commit(&mut repo, &[root], vec![delete("nope/deeper")])),
        "deletes a missing file"
    );
    assert_eq!(
        rule(commit(&mut repo, &[root], vec![delete("top.txt/under")])),
        "deletes a missing file"
    );
}

#[test]
fn rejects_deleting_a_directory() {
    let (mut repo, root) = seeded();
    assert_eq!(
        rule(commit(&mut repo, &[root], vec![delete("a")])),
        "deletes a directory"
    );
}

#[test]
fn rejects_a_no_op_upsert() {
    let (mut repo, root) = seeded();
    let change = upsert(&mut repo, "top.txt", b"top");
    assert_eq!(
        rule(commit(&mut repo, &[root], vec![change])),
        "upsert does not change the file"
    );
}

#[test]
fn rejects_a_file_under_a_file() {
    let (mut repo, root) = seeded();
    let change = upsert(&mut repo, "top.txt/inner", b"i");
    assert_eq!(
        rule(commit(&mut repo, &[root], vec![change])),
        "a parent is a file"
    );
}

#[test]
fn rejects_a_file_over_a_directory() {
    let (mut repo, root) = seeded();
    let change = upsert(&mut repo, "a", b"file");
    assert_eq!(
        rule(commit(&mut repo, &[root], vec![change])),
        "path is a directory"
    );
}

#[test]
fn a_corrupt_base_tree_is_reported() {
    // The API never writes a tree that points at a blob as a directory, so
    // build one by hand and derive from it directly.
    let mut repo = Repository::new();
    let blob = repo.put_blob(b"not a tree").unwrap();
    assert!(matches!(repo.tree(blob), Err(Error::Malformed { .. })));
    let entry = openscion_proto::TreeEntry::new("d", EntryKind::Tree, blob);
    let bogus = repo
        .objects
        .put(ObjectKind::Tree, &encode_tree(&[entry]))
        .unwrap();
    let error = super::derive::derive_root_tree(repo.objects.as_mut(), bogus, &[delete("d/f")])
        .unwrap_err();
    assert!(matches!(error, Error::Malformed { .. }));
}

#[test]
fn an_identical_request_returns_the_existing_changeset() {
    let (mut repo, root) = seeded();
    let change = upsert(&mut repo, "top.txt", b"v2");
    let first = commit(&mut repo, &[root], vec![change.clone()]).unwrap();
    let again = commit(&mut repo, &[root], vec![change]).unwrap();
    assert_eq!(first, again);
    assert_eq!(repo.log_len().unwrap(), 2);
}

#[test]
fn changesets_read_back_with_sorted_changes() {
    let (mut repo, root) = seeded();
    let change = upsert(&mut repo, "a/new", b"n");
    let next = commit(&mut repo, &[root], vec![delete("top.txt"), change]).unwrap();
    let record = repo.changeset(next.id).unwrap();
    assert_eq!(record.info, next);
    assert_eq!(record.changeset.created_at, 1);
    let paths: Vec<_> = record
        .changeset
        .changes
        .iter()
        .map(|c| c.path().as_str())
        .collect();
    assert_eq!(paths, ["a/new", "top.txt"]);
    let ghost = ObjectId::from_bytes([9; 32]);
    assert_eq!(repo.changeset(ghost).unwrap_err(), Error::NotFound(ghost));
}

#[test]
fn objects_are_readable() {
    let (repo, root) = seeded();
    assert!(repo.object(root.root_tree).is_ok());
    // The tree names entries whose objects are retrievable.
    let top = repo.tree(root.root_tree).unwrap();
    assert!(!top.is_empty());
    assert!(repo.object(top[0].id).is_ok());
    // An object that was never stored is not found.
    assert!(matches!(
        Repository::default().object(ObjectId::from_bytes([0; 32])),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn blobs_uploaded_with_an_id_are_verified() {
    let mut repo = Repository::new();
    let id = object_id(ObjectKind::Blob, b"data");
    repo.put_blob_verified(id, b"data").unwrap();
    assert!(matches!(
        repo.put_blob_verified(id, b"other"),
        Err(Error::CorruptObject { .. })
    ));
}

#[test]
fn refs_move_only_by_compare_and_swap() {
    let (mut repo, root) = seeded();
    let change = upsert(&mut repo, "top.txt", b"v2");
    let next = commit(&mut repo, &[root], vec![change]).unwrap();

    assert_eq!(
        repo.get_ref("main").unwrap_err(),
        Error::NoSuchRef("main".to_owned())
    );
    let created = repo
        .update_ref("main", UpdateRef::new(None, root.id))
        .unwrap();
    assert_eq!((created.target, created.version), (root.id, 1));

    let stale = repo
        .update_ref("main", UpdateRef::new(None, next.id))
        .unwrap_err();
    assert_eq!(
        stale,
        Error::RefConflict {
            name: "main".to_owned(),
            current: Some(root.id)
        }
    );
    let moved = repo
        .update_ref("main", UpdateRef::new(Some(root.id), next.id))
        .unwrap();
    assert_eq!((moved.target, moved.version), (next.id, 2));
    assert_eq!(repo.get_ref("main").unwrap(), moved);
}

#[test]
fn refs_reject_bad_names_and_unknown_targets() {
    let (mut repo, root) = seeded();
    assert!(matches!(
        repo.get_ref("../main"),
        Err(Error::InvalidPath(_))
    ));
    assert!(matches!(
        repo.update_ref("", UpdateRef::new(None, root.id)),
        Err(Error::InvalidPath(_))
    ));
    let ghost = ObjectId::from_bytes([6; 32]);
    assert_eq!(
        repo.update_ref("main", UpdateRef::new(None, ghost))
            .unwrap_err(),
        Error::NotFound(ghost)
    );
    let missing = repo
        .update_ref("main", UpdateRef::new(Some(root.id), root.id))
        .unwrap_err();
    assert_eq!(
        missing,
        Error::RefConflict {
            name: "main".to_owned(),
            current: None
        }
    );
}

// ---------------------------------------------------------------------------
// Phase 2: derived data and queries.
// ---------------------------------------------------------------------------

fn create(
    repo: &mut Repository,
    parents: &[ObjectId],
    changes: Vec<Change>,
) -> crate::Result<ChangesetInfo> {
    repo.create_changeset(CreateChangeset::new(
        parents.to_vec(),
        changes,
        "test",
        "message",
    ))
}

/// One changeset that upserts a single file (avoids borrowing `repo` twice in
/// one expression).
fn commit1(repo: &mut Repository, parents: &[ObjectId], at: &str, bytes: &[u8]) -> ChangesetInfo {
    let change = upsert(repo, at, bytes);
    create(repo, parents, vec![change]).unwrap()
}

/// A tiny deterministic generator, so randomized-history tests never flake.
struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as usize) % bound
    }
}

/// Every ancestor of `id`, including itself, by walking parents through the
/// public API — the brute-force oracle the pruned walks must agree with.
fn ancestors(repo: &Repository, id: ObjectId) -> HashSet<ObjectId> {
    let mut seen = HashSet::new();
    let mut stack = vec![id];
    seen.insert(id);
    while let Some(current) = stack.pop() {
        for parent in repo.changeset(current).unwrap().changeset.parents {
            if seen.insert(parent) {
                stack.push(parent);
            }
        }
    }
    seen
}

/// Builds a random single-root DAG of `n` changesets and returns their ids in
/// creation order. Every changeset carries one unique change, so ids differ.
fn random_history(n: usize, seed: u64) -> (Repository, Vec<ObjectId>) {
    let mut repo = Repository::with_clock(FixedClock(0));
    let mut rng = Rng(seed);
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let change = upsert(&mut repo, &format!("f{i}.txt"), format!("v{i}").as_bytes());
        let parents = if i == 0 {
            Vec::new()
        } else {
            let first = ids[rng.below(ids.len())];
            if rng.below(2) == 0 {
                vec![first]
            } else {
                let second = ids[rng.below(ids.len())];
                if second == first {
                    vec![first]
                } else {
                    vec![first, second]
                }
            }
        };
        ids.push(create(&mut repo, &parents, vec![change]).unwrap().id);
    }
    (repo, ids)
}

#[test]
fn generation_is_one_plus_the_max_parent() {
    let mut repo = Repository::with_clock(FixedClock(0));
    let a = commit1(&mut repo, &[], "a", b"a");
    let b = commit1(&mut repo, &[a.id], "b", b"b");
    let c = commit1(&mut repo, &[a.id], "c", b"c");
    // A merge of b (generation 2) and c (generation 2) is generation 3.
    let m = commit1(&mut repo, &[b.id, c.id], "d", b"d");
    assert_eq!(repo.generation(a.id).unwrap(), 1);
    assert_eq!(repo.generation(b.id).unwrap(), 2);
    assert_eq!(repo.generation(c.id).unwrap(), 2);
    assert_eq!(repo.generation(m.id).unwrap(), 3);
    assert_eq!(
        repo.generation(ObjectId::from_bytes([9; 32])).unwrap_err(),
        Error::NotFound(ObjectId::from_bytes([9; 32]))
    );
}

#[test]
fn is_ancestor_agrees_with_a_brute_force_walk() {
    let (repo, ids) = random_history(60, 0x00C0_FFEE);
    for &a in &ids {
        for &d in &ids {
            assert_eq!(
                repo.is_ancestor(a, d).unwrap(),
                ancestors(&repo, d).contains(&a),
                "is_ancestor disagreed for {a} -> {d}"
            );
        }
    }
}

#[test]
fn merge_base_agrees_with_brute_force() {
    let (repo, ids) = random_history(50, 0x1234_5678);
    for &a in &ids {
        for &b in &ids {
            let common: HashSet<_> = ancestors(&repo, a)
                .intersection(&ancestors(&repo, b))
                .copied()
                .collect();
            let base = repo.merge_base(a, b).unwrap();
            // Single-root histories always share the root, so there is always
            // a base, and it is a common ancestor of greatest generation.
            let best = common
                .iter()
                .map(|id| repo.generation(*id).unwrap())
                .max()
                .unwrap();
            let base = base.expect("a single-root history always has a merge base");
            assert!(common.contains(&base));
            assert_eq!(repo.generation(base).unwrap(), best);
        }
    }
}

#[test]
fn snapshot_stat_list_tree_and_read_file() {
    let mut repo = Repository::with_clock(FixedClock(0));
    let changes = vec![
        upsert(&mut repo, "src/main.rs", b"fn main() {}"),
        upsert(&mut repo, "src/lib.rs", b"//! lib"),
        upsert(&mut repo, "README.md", b"# readme"),
    ];
    let cs = create(&mut repo, &[], changes).unwrap();

    let (kind, id) = repo.stat(cs.id, &path("src/main.rs")).unwrap().unwrap();
    assert_eq!(kind, EntryKind::File);
    assert_eq!(id, repo.put_blob(b"fn main() {}").unwrap());
    assert!(repo.stat(cs.id, &path("src/missing.rs")).unwrap().is_none());
    assert!(repo.stat(cs.id, &path("README.md/x")).unwrap().is_none());

    let (_, bytes) = repo.read_file(cs.id, &path("README.md")).unwrap().unwrap();
    assert_eq!(&*bytes, b"# readme");
    // A directory is not a file.
    assert!(repo.read_file(cs.id, &path("src")).unwrap().is_none());
    assert!(repo.read_file(cs.id, &path("nope")).unwrap().is_none());

    let top: Vec<String> = repo
        .list_tree(cs.id, None, false)
        .unwrap()
        .into_iter()
        .map(|(path, ..)| path)
        .collect();
    assert_eq!(top, ["README.md", "src"]);
    let under_src: Vec<String> = repo
        .list_tree(cs.id, Some(&path("src")), false)
        .unwrap()
        .into_iter()
        .map(|(path, ..)| path)
        .collect();
    assert_eq!(under_src, ["src/lib.rs", "src/main.rs"]);
    let all: Vec<String> = repo
        .list_tree(cs.id, None, true)
        .unwrap()
        .into_iter()
        .map(|(path, ..)| path)
        .collect();
    assert_eq!(all, ["README.md", "src", "src/lib.rs", "src/main.rs"]);
    // Listing a file yields just that file; a missing path yields nothing.
    assert_eq!(
        repo.list_tree(cs.id, Some(&path("README.md")), true)
            .unwrap()
            .len(),
        1
    );
    assert!(
        repo.list_tree(cs.id, Some(&path("gone")), true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn diff_turns_one_snapshot_into_the_other() {
    let mut repo = Repository::with_clock(FixedClock(0));
    let first = vec![
        upsert(&mut repo, "keep.txt", b"keep"),
        upsert(&mut repo, "src/old.rs", b"old"),
        upsert(&mut repo, "drop.txt", b"drop"),
    ];
    let a = create(&mut repo, &[], first).unwrap();
    let second = vec![
        upsert(&mut repo, "src/old.rs", b"new"),
        upsert(&mut repo, "src/add.rs", b"add"),
        delete("drop.txt"),
    ];
    let b = create(&mut repo, &[a.id], second).unwrap();

    // The diff equals the second changeset's own changes (sorted by path).
    let changes = repo.diff(a.id, b.id, &[]).unwrap();
    let applied = repo
        .create_changeset(CreateChangeset::new(vec![a.id], changes, "t", "m"))
        .unwrap();
    assert_eq!(applied.root_tree, b.root_tree);
    // An unchanged path is absent; a prefix filter narrows the result.
    let only_src = repo.diff(a.id, b.id, &["src"]).unwrap();
    assert!(
        only_src
            .iter()
            .all(|c| c.path().as_str().starts_with("src/"))
    );
    assert_eq!(only_src.len(), 2);
    // A diff of a snapshot against itself is empty.
    assert!(repo.diff(b.id, b.id, &[]).unwrap().is_empty());
}

#[test]
fn log_walks_first_parent_and_filters_by_path() {
    let mut repo = Repository::with_clock(FixedClock(0));
    let c1 = commit1(&mut repo, &[], "a.txt", b"1");
    let c2 = commit1(&mut repo, &[c1.id], "b.txt", b"1");
    let c3 = commit1(&mut repo, &[c2.id], "a.txt", b"2");

    let all: Vec<ObjectId> = repo
        .log(c3.id, None, None, 10)
        .unwrap()
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    assert_eq!(all, [c3.id, c2.id, c1.id]);
    // `until` is exclusive; `limit` caps the walk.
    assert_eq!(repo.log(c3.id, Some(c1.id), None, 10).unwrap().len(), 2);
    assert_eq!(repo.log(c3.id, None, None, 1).unwrap().len(), 1);
    // Path history: only the changesets that touched a.txt.
    let touched: Vec<ObjectId> = repo
        .log(c3.id, None, Some(&path("a.txt")), 10)
        .unwrap()
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    assert_eq!(touched, [c3.id, c1.id]);
}

// ---------------------------------------------------------------------------
// Phase 3: rebase, revert, and landing.
// ---------------------------------------------------------------------------

#[test]
fn rebase_applies_drops_identical_and_conflicts() {
    let mut repo = Repository::with_clock(FixedClock(0));
    let base = commit1(&mut repo, &[], "shared.txt", b"0");
    // A feature changeset off `base` that edits its own file.
    let feature = commit1(&mut repo, &[base.id], "feature.txt", b"f");

    // `onto` advances `base` on a different path: the feature applies cleanly.
    let onto = commit1(&mut repo, &[base.id], "other.txt", b"o");
    let rebased = repo.rebase(feature.id, onto.id).unwrap();
    let record = repo.changeset(rebased).unwrap();
    assert_eq!(record.changeset.parents, [onto.id]);
    assert_eq!(
        record.changeset.metadata.get("rebased-from"),
        Some(&feature.id.to_string())
    );
    assert!(repo.stat(rebased, &path("other.txt")).unwrap().is_some());
    assert!(repo.stat(rebased, &path("feature.txt")).unwrap().is_some());

    // Identical change on both sides: the rebase keeps nothing and returns onto.
    let same = commit1(&mut repo, &[base.id], "feature.txt", b"f");
    assert_eq!(repo.rebase(feature.id, same.id).unwrap(), same.id);

    // Conflicting change on the same path: no changeset is created.
    let before = repo.log_len().unwrap();
    let conflict = commit1(&mut repo, &[base.id], "feature.txt", b"OTHER");
    let err = repo.rebase(feature.id, conflict.id).unwrap_err();
    assert_eq!(
        err,
        Error::MergeConflict {
            paths: vec!["feature.txt".to_owned()]
        }
    );
    assert_eq!(
        repo.log_len().unwrap(),
        before + 1,
        "conflict created nothing beyond `conflict`"
    );
}

#[test]
fn revert_restores_the_first_parent_content() {
    let mut repo = Repository::with_clock(FixedClock(0));
    let base = commit1(&mut repo, &[], "a.txt", b"original");
    let edit = commit1(&mut repo, &[base.id], "a.txt", b"changed");

    let reverted = repo.revert(edit.id, edit.id, "t", "revert").unwrap();
    let (_, bytes) = repo.read_file(reverted, &path("a.txt")).unwrap().unwrap();
    assert_eq!(&*bytes, b"original");
    assert_eq!(
        repo.changeset(reverted)
            .unwrap()
            .changeset
            .metadata
            .get("reverts"),
        Some(&edit.id.to_string())
    );

    // Reverting a creation deletes the file.
    let add = commit1(&mut repo, &[base.id], "new.txt", b"x");
    let undo = repo.revert(add.id, add.id, "t", "revert new").unwrap();
    assert!(repo.stat(undo, &path("new.txt")).unwrap().is_none());
}

#[test]
fn land_disjoint_files_both_succeed_and_same_file_conflicts() {
    let mut repo = Repository::with_clock(FixedClock(0));
    let base = commit1(&mut repo, &[], "base.txt", b"0");
    repo.update_ref("main", UpdateRef::new(None, base.id))
        .unwrap();

    // Two writers branch off `base` touching different files.
    let a = commit1(&mut repo, &[base.id], "a.txt", b"a");
    let b = commit1(&mut repo, &[base.id], "b.txt", b"b");
    let (landed_a, version_a) = repo.land(a.id, "main", 5).unwrap();
    assert_eq!(landed_a, a.id, "first lands as-is, its parent is the tip");
    assert_eq!(version_a, 2);
    // The second lost the race on the tip and rebased onto `a`.
    let (landed_b, _) = repo.land(b.id, "main", 5).unwrap();
    assert_ne!(landed_b, b.id);
    assert_eq!(repo.get_ref("main").unwrap().target, landed_b);
    // The landed tip contains both files.
    assert!(repo.stat(landed_b, &path("a.txt")).unwrap().is_some());
    assert!(repo.stat(landed_b, &path("b.txt")).unwrap().is_some());

    // A changeset that edits an already-landed file conflicts.
    let clash = commit1(&mut repo, &[base.id], "a.txt", b"CLASH");
    assert!(matches!(
        repo.land(clash.id, "main", 5),
        Err(Error::MergeConflict { .. })
    ));
}

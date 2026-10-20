//! Unit tests for the pure conversion logic.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use gix::ObjectId;
use gix::hash::Kind;
use gix::objs::tree::EntryKind as GitEntryKind;
use openscion_core::EntryKind;

use super::{Entry, Mapped, classify, diff};

/// Two distinct git object ids for fixtures.
fn oids() -> (ObjectId, ObjectId) {
    (
        ObjectId::empty_blob(Kind::Sha1),
        ObjectId::empty_tree(Kind::Sha1),
    )
}

fn entry(kind: EntryKind, oid: ObjectId) -> Entry {
    Entry { kind, oid }
}

#[test]
fn classify_maps_each_git_mode() {
    assert_eq!(classify(GitEntryKind::Blob), Mapped::File(EntryKind::File));
    assert_eq!(
        classify(GitEntryKind::BlobExecutable),
        Mapped::File(EntryKind::Executable)
    );
    assert_eq!(
        classify(GitEntryKind::Link),
        Mapped::File(EntryKind::Symlink)
    );
    assert_eq!(classify(GitEntryKind::Tree), Mapped::Tree);
    assert_eq!(classify(GitEntryKind::Commit), Mapped::Submodule);
}

#[test]
fn diff_reports_additions_modifications_and_deletions() {
    let (a, b) = oids();
    let previous = BTreeMap::from([
        ("keep.txt".to_owned(), entry(EntryKind::File, a)),
        ("edit.txt".to_owned(), entry(EntryKind::File, a)),
        ("gone.txt".to_owned(), entry(EntryKind::File, a)),
    ]);
    let current = BTreeMap::from([
        ("keep.txt".to_owned(), entry(EntryKind::File, a)),
        ("edit.txt".to_owned(), entry(EntryKind::File, b)), // content changed
        ("add.txt".to_owned(), entry(EntryKind::File, a)),  // new
    ]);

    let result = diff(&previous, &current).unwrap();
    let upserts: Vec<_> = result
        .upserts
        .iter()
        .map(|u| u.path.as_str().to_owned())
        .collect();
    assert_eq!(upserts, ["add.txt", "edit.txt"]);
    let deletes: Vec<_> = result
        .deletes
        .iter()
        .map(|p| p.as_str().to_owned())
        .collect();
    assert_eq!(deletes, ["gone.txt"]);
}

#[test]
fn diff_treats_a_kind_change_as_an_upsert() {
    let (a, _) = oids();
    let previous = BTreeMap::from([("f".to_owned(), entry(EntryKind::File, a))]);
    // Same bytes, different kind (a file became executable).
    let current = BTreeMap::from([("f".to_owned(), entry(EntryKind::Executable, a))]);
    let result = diff(&previous, &current).unwrap();
    assert_eq!(result.upserts.len(), 1);
    assert_eq!(result.upserts[0].kind, EntryKind::Executable);
    assert!(result.deletes.is_empty());
}

#[test]
fn diff_of_identical_trees_is_empty() {
    let (a, _) = oids();
    let tree = BTreeMap::from([("f".to_owned(), entry(EntryKind::File, a))]);
    let result = diff(&tree, &tree).unwrap();
    assert!(result.upserts.is_empty());
    assert!(result.deletes.is_empty());
}

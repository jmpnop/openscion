//! Integration tests for the public crate surface.
//!
//! These tests link against the crate as a downstream consumer would: they can
//! only use what `src/lib.rs` re-exports. Treat them as the regression suite
//! for the crate's public contract — if a change breaks a test here, it is a
//! breaking change for users.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use openscion_core::{
    Change, CreateChangeset, EntryKind, Error, FixedClock, FrameWriter, ObjectKind, RepoPath,
    Repository, UpdateRef, decode_frames, object_id, openscion_proto,
};

#[test]
fn the_protocol_types_are_the_same_types() {
    let id: openscion_proto::ObjectId = object_id(ObjectKind::Blob, b"x");
    let _: openscion_core::ObjectId = id;
}

#[test]
fn a_consumer_can_commit_and_move_a_ref() {
    let mut repo = Repository::with_clock(FixedClock(5));
    let blob = repo.put_blob(b"hello").unwrap();
    let info = repo
        .create_changeset(CreateChangeset::new(
            vec![],
            vec![Change::Upsert {
                path: RepoPath::new("hello.txt").unwrap(),
                kind: EntryKind::File,
                blob,
            }],
            "test",
            "init",
        ))
        .unwrap();
    let main = repo
        .update_ref("main", UpdateRef::new(None, info.id))
        .unwrap();
    assert_eq!(main.target, info.id);
    assert_eq!(repo.tree(info.root_tree).unwrap()[0].id, blob);
}

#[test]
fn ids_are_deterministic_under_a_fixed_clock() {
    let build = || {
        let mut repo = Repository::with_clock(FixedClock(5));
        let blob = repo.put_blob(b"same").unwrap();
        repo.create_changeset(CreateChangeset::new(
            vec![],
            vec![Change::Upsert {
                path: RepoPath::new("f").unwrap(),
                kind: EntryKind::File,
                blob,
            }],
            "test",
            "init",
        ))
        .unwrap()
    };
    assert_eq!(build(), build());
}

#[test]
fn errors_are_available_to_consumers() {
    let repo = Repository::new();
    assert_eq!(
        repo.get_ref("main").unwrap_err(),
        Error::NoSuchRef("main".to_owned())
    );
    let mut frames = FrameWriter::new();
    frames.push(object_id(ObjectKind::Blob, b"x"), b"x");
    assert_eq!(decode_frames(&frames.finish()).unwrap().len(), 1);
}

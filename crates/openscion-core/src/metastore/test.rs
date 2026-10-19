//! Unit tests for the in-memory metadata store.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use openscion_proto::{Changeset, CreateChangeset, ObjectId};

use super::{MemoryMetadataStore, MetadataStore, NewChangeset};

fn id(byte: u8) -> ObjectId {
    ObjectId::from_bytes([byte; 32])
}

fn changeset(parents: Vec<ObjectId>) -> Changeset {
    Changeset::from_request(CreateChangeset::new(parents, vec![], "agent", "m"), 7)
}

fn append(store: &mut MemoryMetadataStore, byte: u8, parents: Vec<ObjectId>, generation: u64) {
    store
        .append(NewChangeset {
            id: id(byte),
            root_tree: id(100 + byte),
            changeset: changeset(parents),
            generation,
        })
        .unwrap();
}

#[test]
fn append_assigns_sequential_sequence_numbers() {
    let mut store = MemoryMetadataStore::new();
    assert_eq!(store.log_len().unwrap(), 0);
    append(&mut store, 1, vec![], 1);
    append(&mut store, 2, vec![id(1)], 2);
    assert_eq!(store.log_len().unwrap(), 2);
    assert_eq!(store.info(id(1)).unwrap().unwrap().lsn, 1);
    assert_eq!(store.info(id(2)).unwrap().unwrap().lsn, 2);
    assert_eq!(store.info(id(2)).unwrap().unwrap().root_tree, id(102));
}

#[test]
fn the_three_read_widths_agree() {
    let mut store = MemoryMetadataStore::new();
    append(&mut store, 1, vec![], 1);
    append(&mut store, 2, vec![id(1)], 2);

    let info = store.info(id(2)).unwrap().unwrap();
    let ancestry = store.ancestry(id(2)).unwrap().unwrap();
    let record = store.record(id(2)).unwrap().unwrap();
    assert_eq!(record.info, info);
    assert_eq!(record.generation, ancestry.generation);
    assert_eq!(ancestry.generation, 2);
    assert_eq!(ancestry.parents, vec![id(1)]);
    assert_eq!(record.changeset.parents, vec![id(1)]);
}

#[test]
fn an_unknown_id_reads_as_none() {
    let store = MemoryMetadataStore::new();
    assert!(store.info(id(9)).unwrap().is_none());
    assert!(store.ancestry(id(9)).unwrap().is_none());
    assert!(store.record(id(9)).unwrap().is_none());
}

#[test]
fn a_ref_is_created_then_moved_by_compare_and_swap() {
    let mut store = MemoryMetadataStore::new();
    assert!(store.get_ref("main").unwrap().is_none());

    let created = store.update_ref("main", None, id(1)).unwrap();
    assert_eq!((created.target, created.version), (id(1), 1));
    assert_eq!(store.get_ref("main").unwrap().unwrap(), created);

    let moved = store.update_ref("main", Some(id(1)), id(2)).unwrap();
    assert_eq!((moved.target, moved.version), (id(2), 2));
}

#[test]
fn a_swap_against_the_wrong_expected_value_conflicts() {
    let mut store = MemoryMetadataStore::new();
    store.update_ref("main", None, id(1)).unwrap();

    // Creating a ref that already exists conflicts.
    let error = store.update_ref("main", None, id(2)).unwrap_err();
    assert!(matches!(
        &error,
        crate::Error::RefConflict { name, current } if name == "main" && *current == Some(id(1))
    ));
    // The ref is unchanged.
    assert_eq!(store.get_ref("main").unwrap().unwrap().target, id(1));

    // Moving from the wrong current value conflicts too.
    assert!(matches!(
        store.update_ref("main", Some(id(9)), id(2)),
        Err(crate::Error::RefConflict { .. })
    ));
}

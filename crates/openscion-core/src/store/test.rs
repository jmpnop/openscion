//! Unit tests for the in-memory object store.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use openscion_proto::{ObjectId, ObjectKind};

use super::{MemoryObjectStore, ObjectStore};
use crate::{Error, object_id};

#[test]
fn stores_and_returns_objects_by_id() {
    let mut store = MemoryObjectStore::new();
    assert!(store.is_empty());
    let id = store.put(ObjectKind::Blob, b"hello").unwrap();
    assert_eq!(id, object_id(ObjectKind::Blob, b"hello"));
    assert_eq!(&*store.get(id).unwrap(), b"hello");
    assert!(store.contains(id).unwrap());
    assert_eq!(store.len(), 1);
    assert_eq!(store.total_bytes(), 5);
}

#[test]
fn storing_the_same_object_twice_is_a_no_op() {
    let mut store = MemoryObjectStore::new();
    store.put(ObjectKind::Blob, b"x").unwrap();
    store.put(ObjectKind::Blob, b"x").unwrap();
    assert_eq!(store.len(), 1);
    assert_eq!(store.total_bytes(), 1);
}

#[test]
fn verified_puts_reject_a_mismatched_id() {
    let mut store = MemoryObjectStore::new();
    let wrong = ObjectId::from_bytes([0; 32]);
    let error = store
        .put_verified(ObjectKind::Blob, wrong, b"hello")
        .unwrap_err();
    assert_eq!(
        error,
        Error::CorruptObject {
            expected: wrong,
            actual: object_id(ObjectKind::Blob, b"hello"),
        }
    );
    assert!(store.is_empty());
    let right = object_id(ObjectKind::Blob, b"hello");
    store
        .put_verified(ObjectKind::Blob, right, b"hello")
        .unwrap();
    assert!(store.contains(right).unwrap());
}

#[test]
fn missing_objects_are_not_found() {
    let store = MemoryObjectStore::new();
    let id = ObjectId::from_bytes([1; 32]);
    assert_eq!(store.get(id).unwrap_err(), Error::NotFound(id));
    assert!(!store.contains(id).unwrap());
}

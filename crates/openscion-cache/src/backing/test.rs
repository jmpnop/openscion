//! Unit tests for the in-memory backing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use openscion_tier::ObjectBacking;

use super::MemoryBacking;
use crate::test_support::blob_id;

#[test]
fn a_written_object_round_trips_by_id() {
    let backing = MemoryBacking::new();
    let id = blob_id(b"an object");
    backing.write(id, b"an object").unwrap();

    assert!(backing.contains(id).unwrap());
    assert_eq!(
        backing.read(id).unwrap().as_deref(),
        Some(&b"an object"[..])
    );
}

#[test]
fn an_absent_id_is_none_and_not_contained() {
    let backing = MemoryBacking::new();
    let id = blob_id(b"never written");

    assert!(!backing.contains(id).unwrap());
    assert!(backing.read(id).unwrap().is_none());
}

#[test]
fn a_repeated_write_is_idempotent_and_keeps_the_first_bytes() {
    let backing = MemoryBacking::new();
    let id = blob_id(b"first");
    backing.write(id, b"first").unwrap();
    // A second write under the same id is a no-op; the first bytes stand. (A
    // real store only ever writes verified, id-matching bytes, so the two are
    // identical anyway; this proves the backing does not churn.)
    backing.write(id, b"first").unwrap();

    assert_eq!(backing.read(id).unwrap().as_deref(), Some(&b"first"[..]));
}

#[test]
fn remove_frees_the_bytes_and_tolerates_an_absent_id() {
    let backing = MemoryBacking::new();
    let id = blob_id(b"transient");
    backing.write(id, b"transient").unwrap();
    backing.remove(id).unwrap();

    assert!(!backing.contains(id).unwrap());
    // Removing an id the backing does not hold is a success, so a racing
    // double-eviction is harmless.
    backing.remove(id).unwrap();
    backing.remove(blob_id(b"never there")).unwrap();
}

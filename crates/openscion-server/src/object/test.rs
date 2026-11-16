//! Unit tests for the framed object endpoint's repository adapter.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, RwLock};

use openscion_core::{ObjectKind, Repository, object_id};
use openscion_tier::{Error as TierError, ObjectService};

use super::{RepositoryObjects, map_object};

fn service_over(repo: Repository) -> RepositoryObjects {
    RepositoryObjects::new(Arc::new(RwLock::new(repo)))
}

#[test]
fn get_returns_the_bytes_of_a_stored_object() {
    let mut repo = Repository::new();
    let id = repo.put_blob(b"hello object").unwrap();
    let service = service_over(repo);
    assert_eq!(&*service.get(id).unwrap(), b"hello object");
}

#[test]
fn get_is_not_found_for_an_absent_object() {
    let service = service_over(Repository::new());
    let absent = object_id(ObjectKind::Blob, b"never stored");
    assert!(matches!(service.get(absent), Err(TierError::NotFound(_))));
}

#[test]
fn map_object_relays_a_found_object() {
    let id = object_id(ObjectKind::Blob, b"bytes");
    let bytes: Arc<[u8]> = Arc::from(&b"bytes"[..]);
    let mapped = map_object(Ok(Arc::clone(&bytes)), id).unwrap();
    assert_eq!(&*mapped, b"bytes");
}

#[test]
fn map_object_turns_the_engines_absence_into_tier_not_found() {
    let id = object_id(ObjectKind::Blob, b"x");
    let absent = Err(openscion_core::Error::NotFound(id));
    assert!(matches!(
        map_object(absent, id),
        Err(TierError::NotFound(_))
    ));
}

#[test]
fn map_object_relays_any_other_engine_failure_as_upstream() {
    // A storage failure is not an absence: it must reach the caller as an error,
    // not be mistaken for a missing object.
    let id = object_id(ObjectKind::Blob, b"x");
    let failure = Err(openscion_core::Error::Database {
        message: "disk full".to_owned(),
    });
    match map_object(failure, id) {
        Err(TierError::Upstream(message)) => assert!(message.contains("disk full")),
        other => panic!("expected an upstream error, got {other:?}"),
    }
}

//! Unit tests for the framed object endpoint.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::TcpListener;
use std::sync::{Arc, RwLock};

use openscion_core::{ObjectKind, Repository, object_id};
use openscion_tier::{Endpoint, Error as TierError, ObjectService};

use super::{RepositoryObjects, bind_object_listener, map_object};
use crate::Error;

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

#[test]
fn binding_a_taken_object_endpoint_fails_with_object_listen() {
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = Endpoint::Tcp(taken.local_addr().unwrap());
    let error = bind_object_listener(&endpoint).unwrap_err();
    assert!(matches!(error, Error::ObjectListen { .. }));
}

#[test]
fn binding_reports_the_resolved_address() {
    let endpoint = Endpoint::Tcp("127.0.0.1:0".parse().unwrap());
    let (_, listening) = bind_object_listener(&endpoint).unwrap();
    assert!(listening.starts_with("tcp:127.0.0.1:"));
}

#[cfg(unix)]
#[test]
fn serves_objects_over_a_unix_socket() {
    use openscion_tier::{ObjectSource, SocketSource};

    use super::serve_objects;

    let mut repo = Repository::new();
    let id = repo.put_blob(b"unix object").unwrap();
    let service = Arc::new(RepositoryObjects::new(Arc::new(RwLock::new(repo))));

    let dir = std::env::temp_dir().join(format!("openscion-objsock-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("objects.sock");
    let (listener, listening) = bind_object_listener(&Endpoint::Unix(path.clone())).unwrap();
    assert_eq!(listening, format!("unix:{}", path.display()));
    serve_objects(listener, service);

    let source = SocketSource::unix(&path);
    assert_eq!(&*source.fetch(id).unwrap().unwrap(), b"unix object");
    assert_eq!(
        source
            .fetch(object_id(ObjectKind::Blob, b"absent"))
            .unwrap(),
        None
    );

    let _ = std::fs::remove_dir_all(&dir);
}

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
fn rpc_get_ref_on_a_missing_ref_is_a_not_found_error() {
    use openscion_core::rpc::{Request, Response};

    let service = service_over(Repository::new());
    match service
        .rpc(Request::GetRef("refs/heads/nope".to_owned()))
        .unwrap()
    {
        Response::Error(body) => assert_eq!(body.code, "NotFound"),
        other => panic!("expected an error response, got {other:?}"),
    }
}

#[test]
fn rpc_dispatches_the_read_and_query_operations() {
    use openscion_core::rpc::{Request, Response};
    use openscion_core::{
        AncestorQuery, Change, CreateChangeset, DiffRequest, EntryKind, LogRequest, MergeBaseQuery,
        RepoPath,
    };

    let (service, cs, blob) = seeded();

    let another = Change::Upsert {
        path: RepoPath::new("file2.txt").unwrap(),
        kind: EntryKind::File,
        blob,
    };
    let create = CreateChangeset::new(vec![cs], vec![another], "author", "another");
    assert!(matches!(
        service.rpc(Request::CreateChangeset(create)).unwrap(),
        Response::Changeset(_)
    ));
    assert!(matches!(
        service.rpc(Request::GetChangeset(cs)).unwrap(),
        Response::ChangesetRecord(_)
    ));
    let stat = Request::Stat {
        changeset: cs,
        path: "file.txt".to_owned(),
    };
    assert!(matches!(service.rpc(stat).unwrap(), Response::Stat(_)));
    let tree = Request::ListTree {
        changeset: cs,
        path: None,
        recursive: true,
    };
    assert!(matches!(service.rpc(tree).unwrap(), Response::Tree(_)));
    let read = Request::ReadFile {
        changeset: cs,
        path: "file.txt".to_owned(),
    };
    assert!(matches!(
        service.rpc(read).unwrap(),
        Response::File(Some(_))
    ));
    let diff = Request::Diff(DiffRequest::new(cs, cs, Vec::new()));
    assert!(matches!(service.rpc(diff).unwrap(), Response::Diff(_)));
    let mut log = LogRequest::new(cs, 10);
    log.until = None;
    log.path = None;
    assert!(matches!(
        service.rpc(Request::Log(log)).unwrap(),
        Response::Log(_)
    ));
    let ancestor = Request::Ancestor(AncestorQuery::new(cs, cs));
    assert!(matches!(
        service.rpc(ancestor).unwrap(),
        Response::Ancestor(_)
    ));
    let merge_base = Request::MergeBase(MergeBaseQuery::new(cs, cs));
    assert!(matches!(
        service.rpc(merge_base).unwrap(),
        Response::MergeBase(_)
    ));
    let get_ref = Request::GetRef("refs/heads/main".to_owned());
    assert!(matches!(service.rpc(get_ref).unwrap(), Response::Ref(_)));
}

#[test]
fn rpc_dispatches_the_ref_and_landing_mutations() {
    use openscion_core::rpc::{Request, Response};
    use openscion_core::{LandRequest, RebaseRequest, RevertRequest, UpdateRef};

    let (service, cs, _blob) = seeded();

    let update = Request::UpdateRef {
        name: "refs/heads/other".to_owned(),
        update: UpdateRef::new(None, cs),
    };
    assert!(matches!(service.rpc(update).unwrap(), Response::Ref(_)));

    // Rebase, revert, and land reach their engine calls; onto/into itself may
    // succeed or conflict — either is a well-formed response, and the arm ran.
    let rebase = Request::Rebase(RebaseRequest::new(cs, cs));
    assert!(matches!(
        service.rpc(rebase).unwrap(),
        Response::Rebase(_) | Response::Error(_)
    ));
    let revert = Request::Revert(RevertRequest::new(cs, cs, "author", "revert"));
    assert!(matches!(
        service.rpc(revert).unwrap(),
        Response::Rebase(_) | Response::Error(_)
    ));
    let land = Request::Land(LandRequest::new(cs, "refs/heads/main", 3));
    assert!(matches!(
        service.rpc(land).unwrap(),
        Response::Land(_) | Response::Error(_)
    ));
}

/// A repository with a blob, a changeset holding `file.txt`, and `refs/heads/main`
/// pointing at it — the service, that changeset's id, and the blob's id.
fn seeded() -> (
    RepositoryObjects,
    openscion_core::ObjectId,
    openscion_core::ObjectId,
) {
    use openscion_core::{Change, CreateChangeset, EntryKind, RepoPath, UpdateRef};

    let mut repo = Repository::new();
    let blob = repo.put_blob(b"hello").unwrap();
    let change = Change::Upsert {
        path: RepoPath::new("file.txt").unwrap(),
        kind: EntryKind::File,
        blob,
    };
    let cs = repo
        .create_changeset(CreateChangeset::new(
            vec![],
            vec![change],
            "author",
            "message",
        ))
        .unwrap()
        .id;
    repo.update_ref("refs/heads/main", UpdateRef::new(None, cs))
        .unwrap();
    (
        RepositoryObjects::new(Arc::new(RwLock::new(repo))),
        cs,
        blob,
    )
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

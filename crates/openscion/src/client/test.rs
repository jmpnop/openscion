//! Unit tests for the server client, against real and canned servers.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use openscion_core::{
    Change, CreateChangeset, EntryKind, FrameWriter, ObjectId, ObjectKind, PROTOCOL_VERSION,
    RepoPath, UpdateRef, object_id,
};

use super::ServerClient;
use crate::Error;
use crate::test_support::{canned_server, real_server, unreachable_server};

/// Uploads one blob and returns its id.
fn put(client: &ServerClient, bytes: &[u8]) -> ObjectId {
    let id = object_id(ObjectKind::Blob, bytes);
    let mut frames = FrameWriter::new();
    frames.push(id, bytes);
    client.upload_blobs(&frames.finish()).unwrap();
    id
}

/// An upsert of a regular file.
fn upsert(path: &str, blob: ObjectId) -> Change {
    Change::Upsert {
        path: RepoPath::new(path).unwrap(),
        kind: EntryKind::File,
        blob,
    }
}

#[test]
fn trailing_slashes_are_dropped_from_the_base_url() {
    assert_eq!(ServerClient::new("http://h:1///").base(), "http://h:1");
}

#[test]
fn info_reads_a_real_server() {
    let info = ServerClient::new(real_server()).info().unwrap();
    assert_eq!(info.protocol_version, PROTOCOL_VERSION);
    assert_eq!(info.server_version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn an_unreachable_server_is_a_request_error() {
    let base = unreachable_server();
    let error = ServerClient::new(&base).info().unwrap_err();
    assert!(matches!(error, Error::Request { url, .. } if url == format!("{base}/v1/version")));
}

#[test]
fn an_error_status_is_a_request_error() {
    let error = ServerClient::new(canned_server(500, "{}"))
        .info()
        .unwrap_err();
    assert!(matches!(error, Error::Request { .. }));
}

#[test]
fn a_body_that_is_not_server_info_is_a_decode_error() {
    let error = ServerClient::new(canned_server(200, r#"{"hello":1}"#))
        .info()
        .unwrap_err();
    assert!(matches!(error, Error::Decode { .. }));
}

#[test]
fn a_server_on_another_major_protocol_is_rejected() {
    let base = canned_server(
        200,
        r#"{"server_version":"9.0.0","protocol_version":[99,0]}"#,
    );
    let error = ServerClient::new(base).info().unwrap_err();
    assert!(matches!(
        error,
        Error::IncompatibleProtocol { server: (99, 0) }
    ));
    assert_eq!(
        error.to_string(),
        format!(
            "server speaks protocol 99.0, which this client ({}.{}) cannot use",
            PROTOCOL_VERSION.0, PROTOCOL_VERSION.1
        )
    );
}

#[test]
fn an_error_body_becomes_a_server_error() {
    let base = canned_server(409, r#"{"code":"RefConflict","message":"lost"}"#);
    let error = ServerClient::new(&base).get_ref("main").unwrap_err();
    assert!(matches!(
        &error,
        Error::Server { status: 409, code, message, .. } if code == "RefConflict" && message == "lost"
    ));
    assert_eq!(
        error.to_string(),
        format!("{base}/v1/refs/main answered 409 RefConflict: lost")
    );
}

#[test]
fn a_fetch_from_an_unreachable_server_is_a_request_error() {
    let client = ServerClient::new(unreachable_server());
    assert!(matches!(
        client.fetch_objects(vec![]),
        Err(Error::Request { .. })
    ));
    assert!(matches!(
        client.upload_blobs(&[]),
        Err(Error::Request { .. })
    ));
}

#[test]
fn the_client_queries_a_snapshot_and_history() {
    let client = ServerClient::new(real_server());
    let one = put(&client, b"one");
    let two = put(&client, b"two");
    let b = put(&client, b"b");
    let c1 = client
        .create_changeset(&CreateChangeset::new(
            vec![],
            vec![upsert("a.txt", one)],
            "t",
            "root",
        ))
        .unwrap();
    let c2 = client
        .create_changeset(&CreateChangeset::new(
            vec![c1.id],
            vec![upsert("a.txt", two), upsert("b.txt", b)],
            "t",
            "edit",
        ))
        .unwrap();

    let stat = client.stat(c1.id, "a.txt").unwrap();
    assert_eq!(stat.entry.unwrap().id, one);
    assert_eq!(client.stat(c1.id, "b.txt").unwrap().entry, None);

    let tree = client.list_tree(c2.id, None, false).unwrap();
    assert_eq!(tree.entries.len(), 2);

    assert_eq!(client.read_file(c1.id, "a.txt").unwrap(), b"one");

    let diff = client.diff(c1.id, c2.id, &[]).unwrap();
    assert_eq!(diff.changes.len(), 2);
    assert_eq!(
        client.diff(c1.id, c2.id, &["b.txt"]).unwrap().changes.len(),
        1
    );

    let log = client.log(c2.id, None, None, 10).unwrap();
    assert_eq!(
        log.changesets.iter().map(|s| s.id).collect::<Vec<_>>(),
        [c2.id, c1.id]
    );

    assert!(client.is_ancestor(c1.id, c2.id).unwrap().is_ancestor);
    assert!(!client.is_ancestor(c2.id, c1.id).unwrap().is_ancestor);
    assert_eq!(client.merge_base(c1.id, c2.id).unwrap().base, Some(c1.id));
}

#[test]
fn the_client_rebases_reverts_and_lands() {
    let client = ServerClient::new(real_server());
    let one = put(&client, b"one");
    let two = put(&client, b"two");
    let c = put(&client, b"c");
    let c1 = client
        .create_changeset(&CreateChangeset::new(
            vec![],
            vec![upsert("a.txt", one)],
            "t",
            "root",
        ))
        .unwrap();
    let c2 = client
        .create_changeset(&CreateChangeset::new(
            vec![c1.id],
            vec![upsert("a.txt", two)],
            "t",
            "edit",
        ))
        .unwrap();
    let side = client
        .create_changeset(&CreateChangeset::new(
            vec![c1.id],
            vec![upsert("c.txt", c)],
            "t",
            "side",
        ))
        .unwrap();

    let rebased = client.rebase(side.id, c2.id).unwrap();
    assert_ne!(rebased.changeset, side.id);
    assert_eq!(
        client
            .list_tree(rebased.changeset, None, false)
            .unwrap()
            .entries
            .len(),
        2
    );

    let reverted = client.revert(c2.id, c2.id, "t", "undo").unwrap();
    assert_eq!(
        client.read_file(reverted.changeset, "a.txt").unwrap(),
        b"one"
    );

    client
        .update_ref("main", UpdateRef::new(None, c2.id))
        .unwrap();
    let landed = client.land(side.id, "main", 10).unwrap();
    // side had to rebase onto the tip c2, so it is a fresh changeset.
    assert_ne!(landed.changeset, side.id);
    assert_eq!(landed.version, 2);
    assert_eq!(client.get_ref("main").unwrap().target, landed.changeset);
}

#[test]
fn a_rebase_conflict_and_a_missing_file_are_server_errors() {
    let client = ServerClient::new(real_server());
    let one = put(&client, b"one");
    let two = put(&client, b"two");
    let rival = put(&client, b"rival");
    let c1 = client
        .create_changeset(&CreateChangeset::new(
            vec![],
            vec![upsert("a.txt", one)],
            "t",
            "root",
        ))
        .unwrap();
    let c2 = client
        .create_changeset(&CreateChangeset::new(
            vec![c1.id],
            vec![upsert("a.txt", two)],
            "t",
            "edit",
        ))
        .unwrap();
    let other = client
        .create_changeset(&CreateChangeset::new(
            vec![c1.id],
            vec![upsert("a.txt", rival)],
            "t",
            "rival",
        ))
        .unwrap();

    let conflict = client.rebase(other.id, c2.id).unwrap_err();
    assert!(matches!(
        &conflict,
        Error::Server { status: 409, code, .. } if code == "MergeConflict"
    ));

    let missing = client.read_file(c1.id, "nope").unwrap_err();
    assert!(matches!(
        &missing,
        Error::Server { status: 404, code, .. } if code == "NotFound"
    ));
}

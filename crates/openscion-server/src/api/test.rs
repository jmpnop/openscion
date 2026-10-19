//! Tests for the API routes, over a real socket.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use axum::http::StatusCode;
use openscion_core::{
    AncestorQuery, AncestorResult, Change, ChangesetInfo, ChangesetRecord, CreateChangeset,
    DiffRequest, DiffResult, Entry, EntryKind, ErrorBody, FetchObjects, FrameWriter, LandRequest,
    LandResult, LogPage, LogRequest, MergeBaseQuery, MergeBaseResult, ObjectId, ObjectKind,
    RebaseRequest, RebaseResult, RefValue, RepoPath, RevertRequest, StatResult, StoredObjects,
    TreeListing, UpdateRef, decode_frames, decode_tree, object_id,
};
use serde_json::Value;

use super::ApiError;
use crate::{Server, ServerConfig};

/// Starts a server on its own runtime thread and returns its base URL.
fn start() -> String {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let server = Server::bind(&ServerConfig::new("127.0.0.1:0".parse().unwrap()))
                .await
                .unwrap();
            sender.send(server.local_addr()).unwrap();
            server.serve(std::future::pending()).await.unwrap();
        });
    });
    format!("http://{}", receiver.recv().unwrap())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

fn post_json(url: &str, body: &impl serde::Serialize) -> (u16, Value) {
    let mut response = agent().post(url).send_json(body).unwrap();
    (
        response.status().as_u16(),
        response.body_mut().read_json().unwrap(),
    )
}

fn get_json(url: &str) -> (u16, Value) {
    let mut response = agent().get(url).call().unwrap();
    (
        response.status().as_u16(),
        response.body_mut().read_json().unwrap(),
    )
}

fn upload(base: &str, blobs: &[&[u8]]) -> Vec<ObjectId> {
    let mut frames = FrameWriter::new();
    for blob in blobs {
        frames.push(object_id(ObjectKind::Blob, blob), blob);
    }
    let mut response = agent()
        .post(format!("{base}/v1/blobs"))
        .send(&frames.finish()[..])
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    response
        .body_mut()
        .read_json::<StoredObjects>()
        .unwrap()
        .ids
}

fn create(base: &str, request: &CreateChangeset) -> ChangesetInfo {
    let (status, body) = post_json(&format!("{base}/v1/changesets"), request);
    assert_eq!(status, 200, "{body}");
    serde_json::from_value(body).unwrap()
}

fn file(path: &str, blob: ObjectId) -> Change {
    Change::Upsert {
        path: RepoPath::new(path).unwrap(),
        kind: EntryKind::File,
        blob,
    }
}

#[test]
fn a_client_can_upload_commit_move_a_ref_and_fetch_the_snapshot() {
    let base = start();
    let ids = upload(&base, &[b"one", b"two"]);
    assert_eq!(ids[0], object_id(ObjectKind::Blob, b"one"));

    let request = CreateChangeset::new(
        vec![],
        vec![file("a/one.txt", ids[0]), file("two.txt", ids[1])],
        "test",
        "init",
    );
    let info = create(&base, &request);

    let (status, body) = post_json(
        &format!("{base}/v1/refs/main"),
        &UpdateRef::new(None, info.id),
    );
    assert_eq!(status, 200);
    let main: RefValue = serde_json::from_value(body).unwrap();
    assert_eq!((main.target, main.version), (info.id, 1));

    let (status, body) = get_json(&format!("{base}/v1/refs/main"));
    assert_eq!(status, 200);
    assert_eq!(serde_json::from_value::<RefValue>(body).unwrap(), main);

    let (status, body) = get_json(&format!("{base}/v1/changesets/{}", info.id));
    assert_eq!(status, 200);
    let record: ChangesetRecord = serde_json::from_value(body).unwrap();
    assert_eq!(record.info, info);

    let mut response = agent()
        .post(format!("{base}/v1/objects"))
        .send_json(FetchObjects::new(vec![info.root_tree, ids[1]]))
        .unwrap();
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    let bytes = response.body_mut().read_to_vec().unwrap();
    let frames = decode_frames(&bytes).unwrap();
    assert_eq!(frames.len(), 2);
    let root = decode_tree(frames[0].1).unwrap();
    assert_eq!(root[0].name, "a");
    assert_eq!(frames[1], (ids[1], &b"two"[..]));
}

#[test]
fn refs_with_slashes_in_their_names_are_routed() {
    let base = start();
    let ids = upload(&base, &[b"x"]);
    let info = create(
        &base,
        &CreateChangeset::new(vec![], vec![file("x", ids[0])], "t", "m"),
    );
    let (status, body) = post_json(
        &format!("{base}/v1/refs/release/1.0"),
        &UpdateRef::new(None, info.id),
    );
    assert_eq!(status, 200);
    assert_eq!(body["name"], "release/1.0");
}

fn error(status_and_body: (u16, Value)) -> (u16, String) {
    let (status, body) = status_and_body;
    (
        status,
        serde_json::from_value::<ErrorBody>(body).unwrap().code,
    )
}

#[test]
fn failures_map_to_status_codes_and_error_bodies() {
    let base = start();
    let ghost = ObjectId::from_bytes([7; 32]);

    assert_eq!(
        error(get_json(&format!("{base}/v1/refs/main"))),
        (404, "NotFound".to_owned())
    );
    assert_eq!(
        error(get_json(&format!("{base}/v1/changesets/{ghost}"))),
        (404, "NotFound".to_owned())
    );
    assert_eq!(
        error(get_json(&format!("{base}/v1/changesets/not-an-id"))),
        (400, "InvalidArgument".to_owned())
    );
    assert_eq!(
        error(post_json(
            &format!("{base}/v1/objects"),
            &FetchObjects::new(vec![ghost])
        )),
        (404, "NotFound".to_owned())
    );
    let request = CreateChangeset::new(vec![], vec![file("x", ghost)], "t", "m");
    assert_eq!(
        error(post_json(&format!("{base}/v1/changesets"), &request)),
        (404, "NotFound".to_owned())
    );

    let ids = upload(&base, &[b"x"]);
    let info = create(
        &base,
        &CreateChangeset::new(vec![], vec![file("x", ids[0])], "t", "m"),
    );
    let swap = UpdateRef::new(Some(info.id), info.id);
    assert_eq!(
        error(post_json(&format!("{base}/v1/refs/main"), &swap)),
        (409, "RefConflict".to_owned())
    );
}

#[test]
fn uploads_with_a_wrong_id_or_a_truncated_frame_are_rejected() {
    let base = start();
    let mut frames = FrameWriter::new();
    frames.push(ObjectId::from_bytes([1; 32]), b"lies");
    let mut response = agent()
        .post(format!("{base}/v1/blobs"))
        .send(&frames.finish()[..])
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
    let body: ErrorBody = response.body_mut().read_json().unwrap();
    assert_eq!(body.code, "CorruptObject");

    let mut response = agent()
        .post(format!("{base}/v1/blobs"))
        .send(&[1u8, 2, 3][..])
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
    let body: ErrorBody = response.body_mut().read_json().unwrap();
    assert_eq!(body.code, "Malformed");
}

#[test]
fn engine_errors_other_than_lookups_and_conflicts_are_bad_requests() {
    let error = ApiError::from(openscion_core::Error::InvalidChange {
        path: "a".to_owned(),
        rule: "path changed twice",
    });
    let (status, body) = error.parts();
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body.code, "InvalidChange");
}

#[test]
fn a_merge_conflict_is_a_409() {
    let error = ApiError::from(openscion_core::Error::MergeConflict {
        paths: vec!["a".to_owned()],
    });
    let (status, body) = error.parts();
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body.code, "MergeConflict");
}

/// Builds a child changeset of `parent` with `changes` and returns its info.
fn child(base: &str, parent: ObjectId, changes: Vec<Change>) -> ChangesetInfo {
    create(
        base,
        &CreateChangeset::new(vec![parent], changes, "test", "m"),
    )
}

fn get_bytes(url: &str) -> (u16, Vec<u8>) {
    let mut response = agent().get(url).call().unwrap();
    (
        response.status().as_u16(),
        response.body_mut().read_to_vec().unwrap(),
    )
}

/// A two-changeset history: a root `{a.txt: one}` and a child that rewrites
/// `a.txt` and adds `b.txt`. Returns the base URL and the two changesets.
fn history() -> (String, ChangesetInfo, ChangesetInfo) {
    let base = start();
    let ids = upload(&base, &[b"one", b"two", b"b"]);
    let c1 = create(
        &base,
        &CreateChangeset::new(vec![], vec![file("a.txt", ids[0])], "test", "root"),
    );
    let c2 = child(
        &base,
        c1.id,
        vec![file("a.txt", ids[1]), file("b.txt", ids[2])],
    );
    (base, c1, c2)
}

#[test]
fn snapshot_queries_resolve_list_and_read_a_path() {
    let (base, c1, c2) = history();
    let one = object_id(ObjectKind::Blob, b"one");

    let (status, body) = get_json(&format!("{base}/v1/changesets/{}/stat?path=a.txt", c1.id));
    assert_eq!(status, 200);
    let stat: StatResult = serde_json::from_value(body).unwrap();
    assert_eq!(stat.entry, Some(Entry::new("a.txt", EntryKind::File, one)));

    // An absent path is a normal answer, not a 404.
    let (status, body) = get_json(&format!("{base}/v1/changesets/{}/stat?path=b.txt", c1.id));
    assert_eq!(status, 200);
    assert_eq!(
        serde_json::from_value::<StatResult>(body).unwrap().entry,
        None
    );

    let (status, body) = get_json(&format!("{base}/v1/changesets/{}/tree", c2.id));
    assert_eq!(status, 200);
    let tree: TreeListing = serde_json::from_value(body).unwrap();
    let paths: Vec<&str> = tree.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, ["a.txt", "b.txt"]);

    let (status, bytes) = get_bytes(&format!("{base}/v1/changesets/{}/file?path=a.txt", c1.id));
    assert_eq!(status, 200);
    assert_eq!(bytes, b"one");

    // A directory and an absent path both read as 404 NotFound.
    assert_eq!(
        error(get_json(&format!(
            "{base}/v1/changesets/{}/file?path=missing",
            c1.id
        ))),
        (404, "NotFound".to_owned())
    );
    // A malformed path is a 400.
    assert_eq!(
        error(get_json(&format!(
            "{base}/v1/changesets/{}/stat?path=a//b",
            c1.id
        ))),
        (400, "InvalidPath".to_owned())
    );
}

#[test]
fn history_queries_diff_log_and_relate_changesets() {
    let (base, c1, c2) = history();

    let (status, body) = post_json(
        &format!("{base}/v1/diff"),
        &DiffRequest::new(c1.id, c2.id, vec![]),
    );
    assert_eq!(status, 200);
    let diff: DiffResult = serde_json::from_value(body).unwrap();
    let paths: Vec<&str> = diff.changes.iter().map(|c| c.path().as_str()).collect();
    assert_eq!(paths, ["a.txt", "b.txt"]);

    // A prefix filter narrows the diff.
    let (_, body) = post_json(
        &format!("{base}/v1/diff"),
        &DiffRequest::new(c1.id, c2.id, vec!["b.txt".to_owned()]),
    );
    let diff: DiffResult = serde_json::from_value(body).unwrap();
    assert_eq!(diff.changes.len(), 1);

    let (status, body) = post_json(&format!("{base}/v1/log"), &LogRequest::new(c2.id, 10));
    assert_eq!(status, 200);
    let log: LogPage = serde_json::from_value(body).unwrap();
    let ids: Vec<ObjectId> = log.changesets.iter().map(|s| s.id).collect();
    assert_eq!(ids, [c2.id, c1.id]);

    let (status, body) = post_json(
        &format!("{base}/v1/ancestor"),
        &AncestorQuery::new(c1.id, c2.id),
    );
    assert_eq!(status, 200);
    assert!(
        serde_json::from_value::<AncestorResult>(body)
            .unwrap()
            .is_ancestor
    );

    let (_, body) = post_json(
        &format!("{base}/v1/ancestor"),
        &AncestorQuery::new(c2.id, c1.id),
    );
    assert!(
        !serde_json::from_value::<AncestorResult>(body)
            .unwrap()
            .is_ancestor
    );

    let (status, body) = post_json(
        &format!("{base}/v1/merge-base"),
        &MergeBaseQuery::new(c1.id, c2.id),
    );
    assert_eq!(status, 200);
    assert_eq!(
        serde_json::from_value::<MergeBaseResult>(body)
            .unwrap()
            .base,
        Some(c1.id)
    );
}

#[test]
fn landing_routes_rebase_revert_and_land() {
    let (base, c1, c2) = history();
    let ids = upload(&base, &[b"c"]);

    // A disjoint side branch rebases onto c2 cleanly.
    let side = child(&base, c1.id, vec![file("c.txt", ids[0])]);
    let (status, body) = post_json(
        &format!("{base}/v1/rebase"),
        &RebaseRequest::new(side.id, c2.id),
    );
    assert_eq!(status, 200);
    let rebased: RebaseResult = serde_json::from_value(body).unwrap();
    assert_ne!(rebased.changeset, side.id);

    // The rebased changeset has all three files.
    let (_, body) = get_json(&format!("{base}/v1/changesets/{}/tree", rebased.changeset));
    let tree: TreeListing = serde_json::from_value(body).unwrap();
    assert_eq!(tree.entries.len(), 3);

    // Reverting c2 onto c2 produces a new changeset that drops b.txt.
    let (status, body) = post_json(
        &format!("{base}/v1/revert"),
        &RevertRequest::new(c2.id, c2.id, "test", "undo"),
    );
    assert_eq!(status, 200);
    let reverted: RebaseResult = serde_json::from_value(body).unwrap();
    let (_, body) = get_json(&format!(
        "{base}/v1/changesets/{}/stat?path=b.txt",
        reverted.changeset
    ));
    assert_eq!(
        serde_json::from_value::<StatResult>(body).unwrap().entry,
        None
    );

    // Land a child of c2 onto main, which starts at c2.
    let (status, _) = post_json(
        &format!("{base}/v1/refs/main"),
        &UpdateRef::new(None, c2.id),
    );
    assert_eq!(status, 200);
    let tip = child(&base, c2.id, vec![file("d.txt", ids[0])]);
    let (status, body) = post_json(
        &format!("{base}/v1/land"),
        &LandRequest::new(tip.id, "main", 10),
    );
    assert_eq!(status, 200);
    let landed: LandResult = serde_json::from_value(body).unwrap();
    assert_eq!(landed.changeset, tip.id);
    assert_eq!(landed.version, 2);
    let (_, body) = get_json(&format!("{base}/v1/refs/main"));
    assert_eq!(
        serde_json::from_value::<RefValue>(body).unwrap().target,
        tip.id
    );
}

#[test]
fn a_conflicting_rebase_is_a_409() {
    let (base, c1, c2) = history();
    let ids = upload(&base, &[b"rival"]);
    // A branch that also rewrites a.txt, differently from c2.
    let rival = child(&base, c1.id, vec![file("a.txt", ids[0])]);
    let (status, body) = post_json(
        &format!("{base}/v1/rebase"),
        &RebaseRequest::new(rival.id, c2.id),
    );
    assert_eq!(status, 409);
    assert_eq!(
        serde_json::from_value::<ErrorBody>(body).unwrap().code,
        "MergeConflict"
    );
}

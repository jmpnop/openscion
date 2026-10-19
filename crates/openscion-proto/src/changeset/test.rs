//! Unit tests pinning the changeset payloads' wire forms.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{Change, Changeset, ChangesetInfo, ChangesetRecord, ChangesetSummary, CreateChangeset};
use crate::{EntryKind, ObjectId, RepoPath};

fn id(byte: u8) -> ObjectId {
    ObjectId::from_bytes([byte; 32])
}

fn path(text: &str) -> RepoPath {
    RepoPath::new(text).unwrap()
}

#[test]
fn changeset_summary_pins_its_wire_form() {
    let summary = ChangesetSummary::new(id(7), vec![id(1), id(2)], "agent", "land it", 1_700);
    let json = serde_json::json!({
        "id": id(7).to_string(),
        "parents": [id(1).to_string(), id(2).to_string()],
        "author": "agent",
        "message": "land it",
        "created_at": 1_700,
    });
    assert_eq!(serde_json::to_value(&summary).unwrap(), json);
    let decoded: ChangesetSummary = serde_json::from_value(json).unwrap();
    assert_eq!(decoded, summary);
}

#[test]
fn changes_serialize_with_an_op_tag() {
    let upsert = Change::Upsert {
        path: path("a/b.txt"),
        kind: EntryKind::File,
        blob: id(1),
    };
    assert_eq!(
        serde_json::to_value(&upsert).unwrap(),
        serde_json::json!({
            "op": "upsert", "path": "a/b.txt", "kind": "file", "blob": id(1).to_string()
        })
    );
    let delete = Change::Delete { path: path("c") };
    assert_eq!(
        serde_json::to_value(&delete).unwrap(),
        serde_json::json!({ "op": "delete", "path": "c" })
    );
    assert_eq!(upsert.path().as_str(), "a/b.txt");
    assert_eq!(delete.path().as_str(), "c");
}

#[test]
fn a_change_with_an_invalid_path_is_rejected() {
    let decoded =
        serde_json::from_value::<Change>(serde_json::json!({ "op": "delete", "path": "a//b" }));
    assert!(decoded.is_err());
}

#[test]
fn create_request_defaults_its_metadata() {
    let decoded: CreateChangeset = serde_json::from_value(serde_json::json!({
        "parents": [], "changes": [], "author": "agent", "message": "init"
    }))
    .unwrap();
    assert_eq!(
        decoded,
        CreateChangeset::new(vec![], vec![], "agent", "init")
    );
}

#[test]
fn stored_changesets_sort_changes_by_path() {
    let request = CreateChangeset::new(
        vec![id(9)],
        vec![
            Change::Delete { path: path("z") },
            Change::Delete { path: path("a") },
        ],
        "agent",
        "tidy",
    );
    let stored = Changeset::from_request(request, 42);
    let paths: Vec<_> = stored.changes.iter().map(|c| c.path().as_str()).collect();
    assert_eq!(paths, ["a", "z"]);
    assert_eq!(stored.created_at, 42);
    assert_eq!(stored.parents, [id(9)]);
}

#[test]
fn info_serializes_to_its_wire_form() {
    let info = ChangesetInfo::new(id(1), 7, id(2));
    assert_eq!(
        serde_json::to_value(info).unwrap(),
        serde_json::json!({ "id": id(1).to_string(), "lsn": 7, "root_tree": id(2).to_string() })
    );
}

#[test]
fn records_flatten_the_summary_and_the_changeset() {
    let changeset = Changeset::from_request(CreateChangeset::new(vec![], vec![], "a", "m"), 5);
    let record = ChangesetRecord::new(ChangesetInfo::new(id(1), 0, id(2)), changeset);
    let encoded = serde_json::to_value(&record).unwrap();
    assert_eq!(encoded["lsn"], 0);
    assert_eq!(encoded["created_at"], 5);
    assert_eq!(encoded["author"], "a");
    let decoded: ChangesetRecord = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, record);
}

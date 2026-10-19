//! Unit tests pinning the query payloads' wire forms.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{
    AncestorQuery, AncestorResult, DiffRequest, DiffResult, Entry, LogPage, LogRequest,
    MergeBaseQuery, MergeBaseResult, StatResult, TreeListing,
};
use crate::{Change, ChangesetSummary, EntryKind, ObjectId, RepoPath};

fn id(byte: u8) -> ObjectId {
    ObjectId::from_bytes([byte; 32])
}

fn path(text: &str) -> RepoPath {
    RepoPath::new(text).unwrap()
}

#[test]
fn an_entry_pins_its_wire_form() {
    let entry = Entry::new("src/main.rs", EntryKind::Executable, id(3));
    let json = serde_json::json!({
        "path": "src/main.rs",
        "kind": "executable",
        "id": id(3).to_string(),
    });
    assert_eq!(serde_json::to_value(&entry).unwrap(), json);
    assert_eq!(serde_json::from_value::<Entry>(json).unwrap(), entry);
}

#[test]
fn a_stat_result_carries_an_optional_entry() {
    let present = StatResult::new(Some(Entry::new("a", EntryKind::File, id(1))));
    assert_eq!(
        serde_json::to_value(&present).unwrap(),
        serde_json::json!({ "entry": { "path": "a", "kind": "file", "id": id(1).to_string() } })
    );
    let absent = StatResult::new(None);
    assert_eq!(
        serde_json::to_value(&absent).unwrap(),
        serde_json::json!({ "entry": null })
    );
    let decoded: StatResult = serde_json::from_value(serde_json::json!({ "entry": null })).unwrap();
    assert_eq!(decoded, absent);
}

#[test]
fn a_tree_listing_is_a_sequence_of_entries() {
    let listing = TreeListing::new(vec![
        Entry::new("a", EntryKind::Tree, id(1)),
        Entry::new("a/b", EntryKind::File, id(2)),
    ]);
    let decoded: TreeListing =
        serde_json::from_value(serde_json::to_value(&listing).unwrap()).unwrap();
    assert_eq!(decoded, listing);
    assert_eq!(TreeListing::new(vec![]).entries.len(), 0);
}

#[test]
fn a_diff_request_defaults_its_prefixes() {
    let decoded: DiffRequest = serde_json::from_value(serde_json::json!({
        "from": id(1).to_string(), "to": id(2).to_string()
    }))
    .unwrap();
    assert_eq!(decoded, DiffRequest::new(id(1), id(2), vec![]));
    let with = DiffRequest::new(id(1), id(2), vec!["src".to_owned()]);
    assert_eq!(
        serde_json::to_value(&with).unwrap()["prefixes"],
        serde_json::json!(["src"])
    );
}

#[test]
fn a_diff_result_carries_changes() {
    let result = DiffResult::new(vec![Change::Delete { path: path("gone") }]);
    assert_eq!(
        serde_json::to_value(&result).unwrap(),
        serde_json::json!({ "changes": [{ "op": "delete", "path": "gone" }] })
    );
    let decoded: DiffResult =
        serde_json::from_value(serde_json::to_value(&result).unwrap()).unwrap();
    assert_eq!(decoded, result);
}

#[test]
fn a_log_request_defaults_its_bounds_and_filter() {
    let decoded: LogRequest = serde_json::from_value(serde_json::json!({
        "from": id(5).to_string(), "limit": 10
    }))
    .unwrap();
    assert_eq!(decoded, LogRequest::new(id(5), 10));
    assert_eq!(decoded.until, None);
    assert_eq!(decoded.path, None);

    let mut full = LogRequest::new(id(5), 3);
    full.until = Some(id(1));
    full.path = Some(path("src/lib.rs"));
    let encoded = serde_json::to_value(&full).unwrap();
    assert_eq!(encoded["until"], id(1).to_string());
    assert_eq!(encoded["path"], "src/lib.rs");
    assert_eq!(serde_json::from_value::<LogRequest>(encoded).unwrap(), full);
}

#[test]
fn a_log_request_with_an_invalid_path_is_rejected() {
    let decoded = serde_json::from_value::<LogRequest>(serde_json::json!({
        "from": id(5).to_string(), "path": "a//b", "limit": 1
    }));
    assert!(decoded.is_err());
}

#[test]
fn a_log_page_is_a_sequence_of_summaries() {
    let page = LogPage::new(vec![ChangesetSummary::new(id(9), vec![], "a", "m", 1)]);
    let decoded: LogPage = serde_json::from_value(serde_json::to_value(&page).unwrap()).unwrap();
    assert_eq!(decoded, page);
}

#[test]
fn an_ancestor_query_and_result_pin_their_wire_forms() {
    let query = AncestorQuery::new(id(1), id(2));
    assert_eq!(
        serde_json::to_value(query).unwrap(),
        serde_json::json!({ "ancestor": id(1).to_string(), "descendant": id(2).to_string() })
    );
    let decoded: AncestorQuery =
        serde_json::from_value(serde_json::to_value(query).unwrap()).unwrap();
    assert_eq!(decoded, query);

    let yes = AncestorResult::new(true);
    assert_eq!(
        serde_json::to_value(yes).unwrap(),
        serde_json::json!({ "is_ancestor": true })
    );
    assert!(yes.is_ancestor);
}

#[test]
fn a_merge_base_query_and_result_pin_their_wire_forms() {
    let query = MergeBaseQuery::new(id(1), id(2));
    assert_eq!(
        serde_json::to_value(query).unwrap(),
        serde_json::json!({ "a": id(1).to_string(), "b": id(2).to_string() })
    );
    let decoded: MergeBaseQuery =
        serde_json::from_value(serde_json::to_value(query).unwrap()).unwrap();
    assert_eq!(decoded, query);

    let some = MergeBaseResult::new(Some(id(7)));
    assert_eq!(
        serde_json::to_value(some).unwrap(),
        serde_json::json!({ "base": id(7).to_string() })
    );
    let none = MergeBaseResult::new(None);
    assert_eq!(
        serde_json::to_value(none).unwrap(),
        serde_json::json!({ "base": null })
    );
    assert_eq!(
        serde_json::from_value::<MergeBaseResult>(serde_json::json!({ "base": null })).unwrap(),
        none
    );
}

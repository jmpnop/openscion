//! Unit tests pinning the landing payloads' wire forms.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{LandRequest, LandResult, RebaseRequest, RebaseResult, RevertRequest};
use crate::ObjectId;

fn id(byte: u8) -> ObjectId {
    ObjectId::from_bytes([byte; 32])
}

#[test]
fn a_rebase_request_pins_its_wire_form() {
    let request = RebaseRequest::new(id(1), id(2));
    let json = serde_json::json!({
        "changeset": id(1).to_string(),
        "onto": id(2).to_string(),
    });
    assert_eq!(serde_json::to_value(request).unwrap(), json);
    assert_eq!(
        serde_json::from_value::<RebaseRequest>(json).unwrap(),
        request
    );
}

#[test]
fn a_revert_request_pins_its_wire_form() {
    let request = RevertRequest::new(id(3), id(4), "agent", "undo it");
    let json = serde_json::json!({
        "changeset": id(3).to_string(),
        "onto": id(4).to_string(),
        "author": "agent",
        "message": "undo it",
    });
    assert_eq!(serde_json::to_value(&request).unwrap(), json);
    assert_eq!(
        serde_json::from_value::<RevertRequest>(json).unwrap(),
        request
    );
}

#[test]
fn a_rebase_result_carries_one_changeset() {
    let result = RebaseResult::new(id(7));
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        serde_json::json!({ "changeset": id(7).to_string() })
    );
    assert_eq!(result.changeset, id(7));
}

#[test]
fn a_land_request_pins_its_wire_form() {
    let request = LandRequest::new(id(5), "main", 10);
    let json = serde_json::json!({
        "changeset": id(5).to_string(),
        "ref_name": "main",
        "max_attempts": 10,
    });
    assert_eq!(serde_json::to_value(&request).unwrap(), json);
    assert_eq!(
        serde_json::from_value::<LandRequest>(json).unwrap(),
        request
    );
}

#[test]
fn a_land_result_carries_the_changeset_and_version() {
    let result = LandResult::new(id(6), 3);
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        serde_json::json!({ "changeset": id(6).to_string(), "version": 3 })
    );
    let decoded: LandResult =
        serde_json::from_value(serde_json::to_value(result).unwrap()).unwrap();
    assert_eq!(decoded, result);
}

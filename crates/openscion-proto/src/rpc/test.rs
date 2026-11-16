//! Tests pinning the RPC envelope's serde representation.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{Request, Response};
use crate::failure::ErrorBody;
use crate::object::ObjectId;

#[test]
fn a_newtype_request_round_trips() {
    let request = Request::GetRef("refs/heads/main".to_owned());
    let json = serde_json::to_string(&request).unwrap();
    assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), request);
}

#[test]
fn a_struct_variant_request_round_trips() {
    let request = Request::ListTree {
        changeset: ObjectId::from_bytes([1u8; 32]),
        path: Some("src".to_owned()),
        recursive: true,
    };
    let json = serde_json::to_string(&request).unwrap();
    assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), request);
}

#[test]
fn an_error_response_round_trips() {
    let response = Response::Error(ErrorBody::new("NotFound", "object not found"));
    let json = serde_json::to_string(&response).unwrap();
    assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), response);
}

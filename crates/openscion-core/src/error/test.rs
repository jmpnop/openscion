//! Unit tests for the crate-wide error type.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use openscion_proto::RepoPath;

fn id(byte: u8) -> ObjectId {
    ObjectId::from_bytes([byte; 32])
}

fn all() -> Vec<Error> {
    vec![
        Error::InvalidPath(RepoPath::new("a//b").unwrap_err()),
        Error::InvalidChange {
            path: "a".to_owned(),
            rule: "no-op upsert",
        },
        Error::NotFound(id(1)),
        Error::NoSuchRef("main".to_owned()),
        Error::RefConflict {
            name: "main".to_owned(),
            current: None,
        },
        Error::CorruptObject {
            expected: id(1),
            actual: id(2),
        },
        Error::Malformed {
            what: "tree",
            reason: "truncated",
        },
        Error::Storage {
            path: "/x".to_owned(),
            kind: std::io::ErrorKind::PermissionDenied,
        },
        Error::Database {
            message: "disk I/O error".to_owned(),
        },
    ]
}

#[test]
fn renders_human_readable_messages() {
    let messages: Vec<_> = all().iter().map(ToString::to_string).collect();
    assert_eq!(messages[0], "invalid path \"a//b\": empty component");
    assert_eq!(messages[1], "invalid change at \"a\": no-op upsert");
    assert_eq!(messages[2], format!("object {} not found", id(1)));
    assert_eq!(messages[3], "ref \"main\" not found");
    assert_eq!(messages[4], "ref \"main\" conflict: it points at nothing");
    assert_eq!(messages[5], format!("object {} hashes to {}", id(1), id(2)));
    assert_eq!(messages[6], "malformed tree: truncated");
    assert_eq!(messages[7], "storage error at /x: permission denied");
    assert_eq!(messages[8], "metadata store error: disk I/O error");
    let conflict = Error::RefConflict {
        name: "main".to_owned(),
        current: Some(id(3)),
    };
    assert!(conflict.to_string().ends_with(&id(3).to_string()));
}

#[test]
fn every_variant_has_a_stable_code() {
    let codes: Vec<_> = all().iter().map(Error::code).collect();
    assert_eq!(
        codes,
        [
            "InvalidPath",
            "InvalidChange",
            "NotFound",
            "NotFound",
            "RefConflict",
            "CorruptObject",
            "Malformed",
            "Storage",
            "Database"
        ]
    );
}

#[test]
fn is_a_standard_error() {
    fn assert_error<E: std::error::Error>(_: &E) {}

    assert_error(&Error::NotFound(id(0)));
}

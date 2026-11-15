//! Unit tests for the canonical encodings and the frame stream.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use openscion_proto::{
    Change, Changeset, CreateChangeset, EntryKind, ObjectId, RepoPath, TreeEntry,
};

use super::{FrameWriter, decode_frames, decode_tree, encode_changeset, encode_tree};
use crate::Error;

fn id(byte: u8) -> ObjectId {
    ObjectId::from_bytes([byte; 32])
}

fn reason(result: crate::Result<impl std::fmt::Debug>) -> &'static str {
    match result.unwrap_err() {
        Error::Malformed { reason, .. } => reason,
        other => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn tree_encoding_matches_the_spec_layout() {
    let entries = vec![
        TreeEntry::new("a", EntryKind::File, id(1)),
        TreeEntry::new("b", EntryKind::Executable, id(2)),
        TreeEntry::new("c", EntryKind::Symlink, id(3)),
        TreeEntry::new("d", EntryKind::Tree, id(4)),
    ];
    let bytes = encode_tree(&entries);
    let mut expected = Vec::new();
    for (kind, name, byte) in [
        (b'f', b'a', 1),
        (b'x', b'b', 2),
        (b'l', b'c', 3),
        (b'd', b'd', 4),
    ] {
        expected.extend_from_slice(&[kind, name, 0]);
        expected.extend_from_slice(&[byte; 32]);
    }
    assert_eq!(bytes, expected);
    assert_eq!(decode_tree(&bytes).unwrap(), entries);
}

#[test]
fn the_empty_tree_encodes_to_nothing() {
    assert_eq!(encode_tree(&[]), [] as [u8; 0]);
    assert_eq!(
        decode_tree(&[]).unwrap(),
        [] as [openscion_proto::TreeEntry; 0]
    );
}

#[test]
fn tree_decoding_rejects_malformed_input() {
    let good = encode_tree(&[TreeEntry::new("a", EntryKind::File, id(1))]);
    assert_eq!(reason(decode_tree(b"za\0")), "unknown entry kind");
    assert_eq!(reason(decode_tree(b"fabc")), "unterminated name");
    assert_eq!(reason(decode_tree(b"f\xff\0")), "name is not UTF-8");
    assert_eq!(reason(decode_tree(b"f..\0")), "invalid entry name");
    assert_eq!(reason(decode_tree(&good[..10])), "truncated id");
    let twice = [good.clone(), good].concat();
    assert_eq!(reason(decode_tree(&twice)), "entries out of order");
}

#[test]
fn changeset_encoding_matches_the_spec_layout() {
    let mut request = CreateChangeset::new(
        vec![id(9)],
        vec![
            Change::Delete {
                path: RepoPath::new("z").unwrap(),
            },
            Change::Upsert {
                path: RepoPath::new("a").unwrap(),
                kind: EntryKind::File,
                blob: id(1),
            },
        ],
        "me",
        "hi",
    );
    request.metadata = BTreeMap::from([("k".to_owned(), "v".to_owned())]);
    let bytes = encode_changeset(&Changeset::from_request(request, 0x0102));

    let mut expected = vec![1, 0, 0, 0, 1];
    expected.extend_from_slice(&[9; 32]);
    expected.extend_from_slice(&[0, 0, 0, 2]);
    expected.extend_from_slice(&[b'u', 0, 0, 0, 1, b'a', b'f']);
    expected.extend_from_slice(&[1; 32]);
    expected.extend_from_slice(&[b'd', 0, 0, 0, 1, b'z']);
    expected.extend_from_slice(&[0, 0, 0, 2, b'm', b'e', 0, 0, 0, 2, b'h', b'i']);
    expected.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1, 2]);
    expected.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 1, b'k', 0, 0, 0, 1, b'v']);
    assert_eq!(bytes, expected);
}

#[test]
fn frames_round_trip() {
    let mut writer = FrameWriter::new();
    assert!(writer.is_empty());
    writer.push(id(1), b"one");
    writer.push(id(2), b"");
    assert_eq!(writer.len(), 2 * 40 + 3);
    let bytes = writer.finish();
    assert_eq!(
        decode_frames(&bytes).unwrap(),
        vec![(id(1), &b"one"[..]), (id(2), &b""[..])]
    );
}

#[test]
fn frame_decoding_rejects_truncation() {
    let mut writer = FrameWriter::new();
    writer.push(id(1), b"body");
    let bytes = writer.finish();
    assert_eq!(reason(decode_frames(&bytes[..10])), "truncated id");
    assert_eq!(reason(decode_frames(&bytes[..36])), "truncated length");
    assert_eq!(reason(decode_frames(&bytes[..42])), "truncated body");
    let mut huge = bytes[..32].to_vec();
    huge.extend_from_slice(&u64::MAX.to_be_bytes());
    let expected = if usize::BITS < 64 {
        "length out of range"
    } else {
        "truncated body"
    };
    assert_eq!(reason(decode_frames(&huge)), expected);
}

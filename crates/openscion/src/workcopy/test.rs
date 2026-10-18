//! Unit tests for working-copy scanning and state.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::fs;

use openscion_core::{EntryKind, ObjectId};

use super::{FileState, Scanned, State, load, save, scan, unchanged, write_file};
use crate::Error;
use crate::test_support::scratch;

#[test]
fn scan_lists_files_sorted_and_skips_the_state_directory() {
    let root = scratch("scan");
    fs::create_dir_all(root.join("b/c")).unwrap();
    fs::create_dir_all(root.join(".opnscn")).unwrap();
    fs::write(root.join("b/c/d.txt"), b"d").unwrap();
    fs::write(root.join("a.txt"), b"aa").unwrap();
    fs::write(root.join(".opnscn/state.json"), b"{}").unwrap();
    fs::create_dir_all(root.join("x/.opnscn")).unwrap();
    fs::write(root.join("x/.opnscn/kept"), b"").unwrap();
    let paths: Vec<_> = scan(&root).unwrap().into_iter().map(|f| f.path).collect();
    assert_eq!(paths, ["a.txt", "b/c/d.txt", "x/.opnscn/kept"]);
}

#[cfg(unix)]
#[test]
fn scan_detects_executables_and_symlinks() {
    use std::os::unix::fs::PermissionsExt;
    let root = scratch("kinds");
    fs::write(root.join("run.sh"), b"#!/bin/sh\n").unwrap();
    fs::set_permissions(root.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("run.sh", root.join("link")).unwrap();
    let files = scan(&root).unwrap();
    assert_eq!(files[0].kind, EntryKind::Symlink);
    assert_eq!(files[1].kind, EntryKind::Executable);
    assert_eq!(super::read_content(&root, &files[0]).unwrap(), b"run.sh");
}

#[test]
fn scanning_a_missing_directory_is_an_io_error() {
    let root = scratch("missing").join("nope");
    assert!(matches!(scan(&root), Err(Error::Io { .. })));
}

#[test]
fn state_round_trips_and_stamps_its_write_time() {
    let root = scratch("state");
    let mut state = State {
        server: "http://h:1".to_owned(),
        ref_name: "main".to_owned(),
        base: ObjectId::from_bytes([1; 32]),
        sparse: Some("dir".to_owned()),
        written_at: 0,
        files: BTreeMap::from([(
            "a".to_owned(),
            FileState {
                kind: EntryKind::File,
                blob: ObjectId::from_bytes([2; 32]),
                size: 1,
                mtime: 2,
                ctime: 3,
            },
        )]),
    };
    save(&root, &mut state).unwrap();
    assert!(state.written_at > 0);
    assert_eq!(load(&root).unwrap(), state);
    assert!(super::is_working_copy(&root));
}

#[test]
fn loading_a_missing_or_corrupt_state_fails() {
    let root = scratch("bad-state");
    assert!(matches!(load(&root), Err(Error::NotAWorkingCopy(_))));
    fs::create_dir_all(root.join(".opnscn")).unwrap();
    fs::write(root.join(".opnscn/state.json"), b"not json").unwrap();
    let error = load(&root).unwrap_err();
    assert!(matches!(error, Error::BadState { .. }));
    assert!(error.to_string().contains("corrupt working-copy state"));
    fs::remove_file(root.join(".opnscn/state.json")).unwrap();
    fs::create_dir(root.join(".opnscn/state.json")).unwrap();
    assert!(matches!(load(&root), Err(Error::Io { .. })));
}

#[test]
fn a_file_is_unchanged_only_if_its_stat_data_matches_and_predates_the_state() {
    let blob = ObjectId::from_bytes([3; 32]);
    let known = FileState {
        kind: EntryKind::File,
        blob,
        size: 5,
        mtime: 100,
        ctime: 50,
    };
    let file = Scanned {
        path: "a".to_owned(),
        kind: EntryKind::File,
        size: 5,
        mtime: 100,
        ctime: 50,
    };
    assert!(unchanged(&file, &known, 101));
    assert!(!unchanged(&file, &known, 100), "racily clean");
    assert!(!unchanged(
        &Scanned {
            size: 6,
            ..file.clone()
        },
        &known,
        101
    ));
    assert!(!unchanged(
        &Scanned {
            mtime: 99,
            ..file.clone()
        },
        &known,
        101
    ));
    // A same-size edit with a restored mtime still advances the change time,
    // so it is never trusted: this is the hole that issue #6 reported.
    assert!(
        !unchanged(
            &Scanned {
                ctime: 51,
                ..file.clone()
            },
            &known,
            101
        ),
        "a changed inode change time must be re-read"
    );
    // A state with no recorded change time (`0`) is never trusted, whatever a
    // scan reports, so an upgraded working copy re-hashes once.
    let legacy = FileState { ctime: 0, ..known };
    assert!(!unchanged(&file, &legacy, 101));
    assert!(!unchanged(
        &Scanned {
            ctime: 0,
            ..file.clone()
        },
        &legacy,
        101
    ));
    let executable = Scanned {
        kind: EntryKind::Executable,
        ..file
    };
    assert!(!unchanged(&executable, &known, 101));
}

/// Issue #6: before the change time joined the stat identity, rewriting a file
/// with different bytes of the same length and restoring its modification time
/// left it looking unchanged, so `commit` dropped the edit. A content write
/// always advances the inode change time, so a scan must now see the change.
#[cfg(unix)]
#[test]
fn a_same_size_edit_with_a_restored_mtime_is_detected() {
    use std::fs::File;
    use std::time::Duration;

    let root = scratch("racy");
    let full = root.join("a.txt");
    fs::write(&full, b"aaaaa").unwrap();
    let recorded = fs::metadata(&full).unwrap().modified().unwrap();
    let before = scan(&root).unwrap().remove(0);
    let known = FileState {
        kind: before.kind,
        blob: super::blob_id(b"aaaaa"),
        size: before.size,
        mtime: before.mtime,
        ctime: before.ctime,
    };

    // Rewrite with different bytes of the same length, then restore the exact
    // modification time, as `touch -r` or an archive extraction would.
    std::thread::sleep(Duration::from_millis(20));
    fs::write(&full, b"bbbbb").unwrap();
    File::options()
        .write(true)
        .open(&full)
        .unwrap()
        .set_modified(recorded)
        .unwrap();

    let after = scan(&root).unwrap().remove(0);
    assert_eq!(after.size, known.size);
    assert_eq!(after.mtime, known.mtime, "mtime was restored");
    assert_ne!(
        after.ctime, known.ctime,
        "the write advanced the change time"
    );
    // `u64::MAX` keeps the restored mtime strictly before the state write, so
    // the change time is the only thing that can reject this file.
    assert!(
        !unchanged(&after, &known, u64::MAX),
        "a same-size edit with a restored mtime must be re-read, not skipped"
    );
}

/// A `state.json` written before the change-time field existed must still load,
/// with the field defaulting to `0` so those files are re-hashed once.
#[test]
fn a_file_state_without_a_change_time_still_loads() {
    let json = br#"{"kind":"file","blob":"0000000000000000000000000000000000000000000000000000000000000000","size":5,"mtime":100}"#;
    let state: FileState = serde_json::from_slice(json).unwrap();
    assert_eq!(state.ctime, 0);
}

#[test]
fn write_file_creates_parents_and_honours_the_kind() {
    let root = scratch("write");
    write_file(&root, "a/b/c.txt", EntryKind::File, b"c").unwrap();
    write_file(&root, "a/run", EntryKind::Executable, b"x").unwrap();
    write_file(&root, "a/link", EntryKind::Symlink, b"run").unwrap();
    assert_eq!(fs::read(root.join("a/b/c.txt")).unwrap(), b"c");
    let kinds: Vec<_> = scan(&root).unwrap().into_iter().map(|f| f.kind).collect();
    if cfg!(unix) {
        assert_eq!(
            kinds,
            [EntryKind::File, EntryKind::Symlink, EntryKind::Executable]
        );
    }
    // A file cannot be written where a file already sits as its parent.
    let error = write_file(&root, "a/b/c.txt/d", EntryKind::File, b"d").unwrap_err();
    assert!(matches!(error, Error::Io { .. }));
}

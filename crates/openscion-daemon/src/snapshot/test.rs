//! Unit tests for the snapshot id-file codec.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use openscion_core::ObjectId;

use super::{read_ids, write_ids};

/// A fresh temp path for one test, cleaned up on drop.
struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "openscion-daemon-snapshot-{}-{}.ids",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        Self { path }
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn id(byte: u8) -> ObjectId {
    ObjectId::from_bytes([byte; 32])
}

#[test]
fn ids_round_trip_in_order() {
    let file = TempFile::new();
    let ids = vec![id(1), id(2), id(0xff)];
    write_ids(&file.path, &ids).unwrap();
    assert_eq!(read_ids(&file.path).unwrap(), ids);
}

#[test]
fn a_missing_file_is_an_empty_set() {
    let file = TempFile::new();
    assert!(read_ids(&file.path).unwrap().is_empty());
}

#[test]
fn blank_lines_and_comments_are_ignored() {
    let file = TempFile::new();
    let body = format!("# the warm set\n\n{}\n   \n{}\n", id(7), id(8));
    std::fs::write(&file.path, body).unwrap();
    assert_eq!(read_ids(&file.path).unwrap(), vec![id(7), id(8)]);
}

#[test]
fn a_corrupt_line_is_an_error_naming_the_file() {
    let file = TempFile::new();
    std::fs::write(&file.path, "not-a-hex-id\n").unwrap();
    let error = read_ids(&file.path).unwrap_err();
    assert!(error.to_string().contains(&file.path.display().to_string()));
}

#[test]
fn a_write_replaces_an_existing_file() {
    let file = TempFile::new();
    write_ids(&file.path, &[id(1), id(2)]).unwrap();
    write_ids(&file.path, &[id(9)]).unwrap();
    assert_eq!(read_ids(&file.path).unwrap(), vec![id(9)]);
}

#[test]
fn a_read_of_a_non_file_is_an_error() {
    // A directory at the path reads with an error that is not `NotFound`, so it
    // surfaces rather than being treated as an empty set.
    let file = TempFile::new();
    std::fs::create_dir(&file.path).unwrap();
    let error = read_ids(&file.path).unwrap_err();
    assert!(error.to_string().contains(&file.path.display().to_string()));
    let _ = std::fs::remove_dir(&file.path);
}

#[test]
fn a_write_into_a_missing_directory_is_an_error() {
    let path = std::env::temp_dir()
        .join(format!(
            "openscion-daemon-no-such-dir-{}",
            std::process::id()
        ))
        .join("warm.ids");
    let error = write_ids(&path, &[id(1)]).unwrap_err();
    assert!(matches!(error, crate::Error::Io { .. }));
}

#[test]
fn a_write_onto_an_existing_directory_is_an_error() {
    // The temporary file writes fine, but renaming it onto a directory fails —
    // exercising the rename error path.
    let file = TempFile::new();
    std::fs::create_dir(&file.path).unwrap();
    let error = write_ids(&file.path, &[id(1)]).unwrap_err();
    assert!(matches!(error, crate::Error::Io { .. }));
    let _ = std::fs::remove_dir(&file.path);
}

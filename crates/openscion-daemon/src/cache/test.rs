//! Unit tests for the on-disk object cache.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::ObjectCache;
use crate::test_support::{blob_id, scratch};

#[test]
fn writes_read_back_and_fan_out_by_id() {
    let cache = ObjectCache::open(scratch("cache")).unwrap();
    let a = blob_id(b"alpha");
    let b = blob_id(b"beta");
    assert!(!cache.contains(a).unwrap());
    assert!(cache.read(a).unwrap().is_none());

    cache.write(a, b"alpha").unwrap();
    cache.write(b, b"beta").unwrap();
    assert!(cache.contains(a).unwrap());
    assert_eq!(&*cache.read(a).unwrap().unwrap(), b"alpha");
    assert_eq!(&*cache.read(b).unwrap().unwrap(), b"beta");
}

#[test]
fn a_write_is_idempotent() {
    let cache = ObjectCache::open(scratch("cache-idem")).unwrap();
    let id = blob_id(b"same");
    cache.write(id, b"same").unwrap();
    cache.write(id, b"same").unwrap();
    assert_eq!(&*cache.read(id).unwrap().unwrap(), b"same");
}

#[test]
fn remove_deletes_and_tolerates_an_absent_id() {
    let cache = ObjectCache::open(scratch("cache-remove")).unwrap();
    let id = blob_id(b"gone");
    cache.write(id, b"gone").unwrap();
    assert!(cache.contains(id).unwrap());
    cache.remove(id).unwrap();
    assert!(!cache.contains(id).unwrap());
    // Removing an id the cache does not hold is a success.
    cache.remove(id).unwrap();
}

#[test]
fn read_and_remove_report_io_errors() {
    let dir = scratch("cache-io");
    let cache = ObjectCache::open(&dir).unwrap();
    let id = blob_id(b"x");
    // A directory sitting where the object file should be makes read and remove
    // fail with an I/O error rather than look absent.
    let hex = id.to_string();
    let path = dir.join(&hex[..2]).join(&hex[2..]);
    std::fs::create_dir_all(&path).unwrap();
    assert!(matches!(cache.read(id), Err(crate::Error::Io { .. })));
    assert!(matches!(cache.remove(id), Err(crate::Error::Io { .. })));
}

#[test]
fn write_reports_an_io_error_when_a_fanout_is_blocked() {
    let dir = scratch("cache-io-write");
    let cache = ObjectCache::open(&dir).unwrap();
    let id = blob_id(b"y");
    // A file where the fanout directory must go blocks the write.
    let hex = id.to_string();
    std::fs::write(dir.join(&hex[..2]), b"blocker").unwrap();
    assert!(matches!(
        cache.write(id, b"y"),
        Err(crate::Error::Io { .. })
    ));
}

#[test]
fn reopening_leaves_a_stray_root_file_alone() {
    let dir = scratch("cache-stray");
    {
        let cache = ObjectCache::open(&dir).unwrap();
        cache.write(blob_id(b"z"), b"z").unwrap();
    }
    // A stray non-directory entry at the cache root is skipped by the sweep.
    std::fs::write(dir.join("stray"), b"junk").unwrap();
    let cache = ObjectCache::open(&dir).unwrap();
    assert!(cache.contains(blob_id(b"z")).unwrap());
    assert!(dir.join("stray").exists(), "a stray file is left alone");
}

#[test]
fn reopening_sweeps_a_leftover_temp_file() {
    let dir = scratch("cache-sweep");
    let cache = ObjectCache::open(&dir).unwrap();
    let id = blob_id(b"real");
    cache.write(id, b"real").unwrap();
    // Simulate an interrupted write: a temp file left in a fanout directory.
    let hex = id.to_string();
    let fanout = dir.join(&hex[..2]);
    let leftover = fanout.join(".tmp-leftover");
    std::fs::write(&leftover, b"partial").unwrap();
    assert!(leftover.exists());

    // Reopening sweeps it, and the real object survives.
    let cache = ObjectCache::open(&dir).unwrap();
    assert!(!leftover.exists());
    assert_eq!(&*cache.read(id).unwrap().unwrap(), b"real");
}

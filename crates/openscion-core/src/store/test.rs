//! Unit tests for the object stores.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use openscion_proto::{ObjectId, ObjectKind};

use super::{FilesystemObjectStore, MemoryObjectStore, ObjectStore};
use crate::{Error, object_id};

/// A fresh, non-existent path under the temp directory, unique to this call.
fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "openscion-fs-{}-{name}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn stores_and_returns_objects_by_id() {
    let mut store = MemoryObjectStore::new();
    assert!(store.is_empty());
    let id = store.put(ObjectKind::Blob, b"hello").unwrap();
    assert_eq!(id, object_id(ObjectKind::Blob, b"hello"));
    assert_eq!(&*store.get(id).unwrap(), b"hello");
    assert!(store.contains(id).unwrap());
    assert_eq!(store.len(), 1);
    assert_eq!(store.total_bytes(), 5);
}

#[test]
fn storing_the_same_object_twice_is_a_no_op() {
    let mut store = MemoryObjectStore::new();
    store.put(ObjectKind::Blob, b"x").unwrap();
    store.put(ObjectKind::Blob, b"x").unwrap();
    assert_eq!(store.len(), 1);
    assert_eq!(store.total_bytes(), 1);
}

#[test]
fn verified_puts_reject_a_mismatched_id() {
    let mut store = MemoryObjectStore::new();
    let wrong = ObjectId::from_bytes([0; 32]);
    let error = store
        .put_verified(ObjectKind::Blob, wrong, b"hello")
        .unwrap_err();
    assert_eq!(
        error,
        Error::CorruptObject {
            expected: wrong,
            actual: object_id(ObjectKind::Blob, b"hello"),
        }
    );
    assert!(store.is_empty());
    let right = object_id(ObjectKind::Blob, b"hello");
    store
        .put_verified(ObjectKind::Blob, right, b"hello")
        .unwrap();
    assert!(store.contains(right).unwrap());
}

#[test]
fn missing_objects_are_not_found() {
    let store = MemoryObjectStore::new();
    let id = ObjectId::from_bytes([1; 32]);
    assert_eq!(store.get(id).unwrap_err(), Error::NotFound(id));
    assert!(!store.contains(id).unwrap());
}

#[test]
fn the_filesystem_store_round_trips_and_lays_out_a_fanout() {
    let root = scratch("round-trip");
    let mut store = FilesystemObjectStore::open(&root).unwrap();

    let id = store.put(ObjectKind::Blob, b"hello").unwrap();
    assert_eq!(id, object_id(ObjectKind::Blob, b"hello"));
    assert_eq!(&*store.get(id).unwrap(), b"hello");
    assert!(store.contains(id).unwrap());

    // The object sits at <root>/<first two hex>/<remaining hex>.
    let hex = id.to_string();
    let path = root.join(&hex[..2]).join(&hex[2..]);
    assert!(path.is_file());

    // Storing the same object again is a no-op.
    assert_eq!(store.put(ObjectKind::Blob, b"hello").unwrap(), id);

    // A verified put rejects a mismatched id and stores nothing new.
    let wrong = ObjectId::from_bytes([0; 32]);
    assert!(matches!(
        store.put_verified(ObjectKind::Blob, wrong, b"nope"),
        Err(Error::CorruptObject { .. })
    ));
    assert!(!store.contains(wrong).unwrap());

    // A verified put with the right id stores the object.
    let right = object_id(ObjectKind::Blob, b"verified");
    store
        .put_verified(ObjectKind::Blob, right, b"verified")
        .unwrap();
    assert_eq!(&*store.get(right).unwrap(), b"verified");

    // A missing object is not found.
    let absent = ObjectId::from_bytes([9; 32]);
    assert_eq!(store.get(absent).unwrap_err(), Error::NotFound(absent));
    assert!(!store.contains(absent).unwrap());
}

#[test]
fn filesystem_objects_survive_a_reopen() {
    let root = scratch("reopen");
    let id = {
        let mut store = FilesystemObjectStore::open(&root).unwrap();
        store.put(ObjectKind::Blob, b"durable").unwrap()
    };
    // A fresh store on the same directory reads the object back.
    let store = FilesystemObjectStore::open(&root).unwrap();
    assert_eq!(&*store.get(id).unwrap(), b"durable");
}

#[test]
fn opening_sweeps_orphaned_temp_files() {
    let root = scratch("sweep");
    std::fs::create_dir_all(root.join("ab")).unwrap();
    let orphan = root.join("ab").join(".tmp-123-0-456");
    std::fs::write(&orphan, b"half-written").unwrap();
    // A stray non-directory entry at the root is skipped, not an error.
    std::fs::write(root.join("stray-file"), b"ignore me").unwrap();
    // A real object next to the orphan must be kept.
    let kept = root.join("ab").join("keeper");
    std::fs::write(&kept, b"keep").unwrap();
    assert!(orphan.exists());

    // Opening the store removes the leftover temp file but nothing else.
    let _store = FilesystemObjectStore::open(&root).unwrap();
    assert!(!orphan.exists());
    assert!(kept.exists());
    assert!(root.join("stray-file").exists());
}

#[cfg(unix)]
#[test]
fn a_write_into_a_read_only_directory_is_a_storage_error() {
    use std::os::unix::fs::PermissionsExt;

    let root = scratch("read-only");
    let store = FilesystemObjectStore::open(&root).unwrap();
    let payload = b"into-a-locked-dir";
    let id = object_id(ObjectKind::Blob, payload);
    let dir = root.join(&id.to_string()[..2]);
    std::fs::create_dir_all(&dir).unwrap();
    // Read + execute, but no write: the temp file cannot be created.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    let mut store = store;
    // A storage error on a normal user, or success when run as root (which
    // bypasses directory permission bits) — anything else is a bug.
    match store.put(ObjectKind::Blob, payload) {
        Err(Error::Storage { .. }) | Ok(_) => {}
        other => panic!("expected a storage error, got {other:?}"),
    }

    // Restore write permission so the scratch directory can be cleaned up.
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755));
}

#[test]
fn filesystem_io_failures_are_storage_errors() {
    // Opening a store rooted at an existing file cannot create the directory.
    let file = scratch("not-a-dir");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"i am a file").unwrap();
    assert!(matches!(
        FilesystemObjectStore::open(&file),
        Err(Error::Storage { .. })
    ));

    // A read that is not a missing file surfaces as a storage error: put a
    // directory where an object would live, then try to read it.
    let root = scratch("read-dir");
    let store = FilesystemObjectStore::open(&root).unwrap();
    let id = object_id(ObjectKind::Blob, b"x");
    let hex = id.to_string();
    std::fs::create_dir_all(root.join(&hex[..2]).join(&hex[2..])).unwrap();
    assert!(matches!(store.get(id), Err(Error::Storage { .. })));
}

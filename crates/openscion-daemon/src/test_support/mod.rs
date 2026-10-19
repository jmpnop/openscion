//! Shared helpers for the daemon's unit tests: scratch directories and object
//! construction.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use openscion_core::{ObjectId, ObjectKind, object_id};

/// A fresh, empty scratch directory unique to this test.
pub(crate) fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "openscion-daemon-{}-{name}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The id a blob with these bytes hashes to.
pub(crate) fn blob_id(bytes: &[u8]) -> ObjectId {
    object_id(ObjectKind::Blob, bytes)
}

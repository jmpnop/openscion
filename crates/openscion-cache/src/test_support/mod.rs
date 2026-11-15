//! Shared helpers for the cache's unit tests: object construction and an
//! in-memory tier-above the cache can read through.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use openscion_core::{ObjectId, ObjectKind, object_id};

use crate::ObjectSource;

/// The id a blob with these bytes hashes to.
pub(crate) fn blob_id(bytes: &[u8]) -> ObjectId {
    object_id(ObjectKind::Blob, bytes)
}

/// A fresh, empty scratch directory unique to this test — for a Unix socket
/// path that needs an existing parent.
pub(crate) fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "openscion-cache-{}-{name}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An in-memory tier-above for tests: it serves the objects put into it.
#[derive(Clone, Default)]
pub(crate) struct MemorySource {
    objects: Arc<Mutex<HashMap<ObjectId, Arc<[u8]>>>>,
}

impl MemorySource {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Adds a blob and returns its id.
    pub(crate) fn add(&self, bytes: &[u8]) -> ObjectId {
        let id = blob_id(bytes);
        self.objects.lock().unwrap().insert(id, Arc::from(bytes));
        id
    }
}

impl ObjectSource for MemorySource {
    fn fetch(&self, id: ObjectId) -> crate::Result<Option<Arc<[u8]>>> {
        Ok(self.objects.lock().unwrap().get(&id).cloned())
    }
}

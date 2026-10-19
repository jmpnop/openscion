//! Crash-recovery tests for the persistent stores.
//!
//! A committed changeset log, its refs, and the objects they reference must
//! survive a process that dies without running any destructor — no clean
//! database close, no flush. The test spawns this same binary as a child, has
//! it write committed data to a data directory and then [`std::process::abort`]
//! (a crash: `SIGABRT`, no unwinding), and then, in the parent, reopens fresh
//! stores on that directory and asserts everything committed is still there.
//!
//! This exercises SQLite's WAL recovery and the filesystem store's
//! write-then-rename discipline across an abrupt termination, which is the
//! phase-4 durability guarantee for the log and refs.

#![cfg(feature = "sqlite")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use openscion_core::{
    Changeset, CreateChangeset, FilesystemObjectStore, MetadataStore, NewChangeset, ObjectId,
    ObjectKind, ObjectStore, SqliteMetadataStore, encode_changeset, object_id,
};

/// The environment variable the parent sets to turn a spawned test into the
/// crashing child, holding the data directory to write into.
const CRASH_DIR_ENV: &str = "OPENSCION_CRASH_DIR";

/// The blob the sample changeset stores, so the parent can look it up by hash.
const SAMPLE: &[u8] = b"crash-safe";

/// A fresh, non-existent data directory under the temp directory.
fn scratch_dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "openscion-crash-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Writes one committed object, changeset, and ref into the stores under
/// `data_dir`, and returns the changeset id.
fn write_sample(data_dir: &Path) -> ObjectId {
    let mut objects = FilesystemObjectStore::open(data_dir.join("objects")).unwrap();
    let mut meta = SqliteMetadataStore::open(data_dir.join("meta.db")).unwrap();
    let blob = objects.put(ObjectKind::Blob, SAMPLE).unwrap();
    let changeset =
        Changeset::from_request(CreateChangeset::new(vec![], vec![], "agent", "commit"), 1);
    let id = object_id(ObjectKind::Changeset, &encode_changeset(&changeset));
    meta.append(NewChangeset {
        id,
        root_tree: blob,
        changeset,
        generation: 1,
        skips: Vec::new(),
    })
    .unwrap();
    meta.update_ref("main", None, id).unwrap();
    id
}

/// Asserts the committed object, changeset, and ref are all readable through
/// fresh stores opened on `data_dir`.
fn assert_sample_present(data_dir: &Path) {
    let objects = FilesystemObjectStore::open(data_dir.join("objects")).unwrap();
    let meta = SqliteMetadataStore::open(data_dir.join("meta.db")).unwrap();
    assert_eq!(
        &*objects.get(object_id(ObjectKind::Blob, SAMPLE)).unwrap(),
        SAMPLE
    );
    assert_eq!(meta.log_len().unwrap(), 1);
    let target = meta.get_ref("main").unwrap().unwrap().target;
    assert!(meta.record(target).unwrap().is_some());
}

#[test]
fn the_sample_writer_persists_both_stores() {
    let dir = scratch_dir();
    write_sample(&dir);
    assert_sample_present(&dir);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The crashing child: when the parent sets `CRASH_DIR_ENV`, write committed
/// data and then abort without unwinding. Otherwise this is a no-op test.
#[test]
fn crash_child_aborts_after_writing() {
    if let Ok(dir) = std::env::var(CRASH_DIR_ENV) {
        write_sample(Path::new(&dir));
        std::process::abort();
    }
}

#[test]
fn committed_data_survives_a_crash() {
    let dir = scratch_dir();
    // Re-run this very test binary, filtered to the child test, with the data
    // directory in the environment; the child writes and aborts.
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["crash_child_aborts_after_writing", "--exact", "--nocapture"])
        .env(CRASH_DIR_ENV, &dir)
        // The child aborts, which would leave a half-written coverage profile;
        // don't let it emit one when the suite runs under instrumentation.
        .env_remove("LLVM_PROFILE_FILE")
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "the child should have crashed via abort, not exited cleanly"
    );

    // After the crash, fresh stores recover everything that was committed.
    assert_sample_present(&dir);
    let _ = std::fs::remove_dir_all(&dir);
}

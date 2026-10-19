//! The filesystem object store.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use openscion_proto::{ObjectId, ObjectKind};

use crate::{Error, Result, object_id};

use super::ObjectStore;

/// The prefix every temp file name starts with.
const TEMP_PREFIX: &str = ".tmp-";

/// Makes temp file names unique within a process.
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// An object store backed by a directory tree.
///
/// Each object lives at `<root>/<aa>/<rest>`, where `aa` is the first two hex
/// characters of its id and `rest` is the remaining sixty-two — the two-level
/// fanout git uses for loose objects, so no one directory holds every object.
///
/// Writes are atomic: an object is written to a temp file in its target
/// directory, flushed with `fsync`, and renamed into place, so a reader never
/// sees a partial object and a crash leaves at most an orphaned temp file,
/// which [`FilesystemObjectStore::open`] sweeps. `fsync` makes a write durable
/// across a process crash; power-loss durability on macOS would additionally
/// need `F_FULLFSYNC`, a later refinement tied to batched (group) commit.
#[derive(Debug)]
pub struct FilesystemObjectStore {
    root: PathBuf,
}

impl FilesystemObjectStore {
    /// Opens the object store rooted at `root`, creating the directory if it is
    /// absent and removing any temp files a previous crash left behind.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Storage`] when the directory cannot be created or read.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|error| storage(&root, &error))?;
        let store = Self { root };
        store.sweep_temps()?;
        Ok(store)
    }

    /// The fanout directory and the full path an object with `id` lives at.
    fn locate(&self, id: ObjectId) -> (PathBuf, PathBuf) {
        let hex = id.to_string();
        let (prefix, rest) = hex.split_at(2);
        let dir = self.root.join(prefix);
        let path = dir.join(rest);
        (dir, path)
    }

    /// Writes `payload` under `id` if it is not already stored.
    fn write(&self, id: ObjectId, payload: &[u8]) -> Result<()> {
        if self.contains(id)? {
            return Ok(());
        }
        let (dir, path) = self.locate(id);
        fs::create_dir_all(&dir).map_err(|error| storage(&dir, &error))?;
        let temp = dir.join(temp_name());
        if let Err(error) = write_temp(&temp, &path, payload) {
            // A partial or un-renamed temp file never becomes an object; drop it.
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
        // Make the new directory entry durable. A failure here is not fatal: the
        // object is already renamed into place and readable; only power-loss
        // durability of the name is at risk, which some filesystems cannot give.
        if let Ok(handle) = File::open(&dir) {
            let _ = handle.sync_all();
        }
        Ok(())
    }

    /// Removes temp files left by an interrupted write.
    fn sweep_temps(&self) -> Result<()> {
        let entries = fs::read_dir(&self.root).map_err(|error| storage(&self.root, &error))?;
        for fanout in entries.flatten() {
            if !fanout.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let Ok(objects) = fs::read_dir(fanout.path()) else {
                continue;
            };
            for object in objects.flatten() {
                if object
                    .file_name()
                    .to_string_lossy()
                    .starts_with(TEMP_PREFIX)
                {
                    let _ = fs::remove_file(object.path());
                }
            }
        }
        Ok(())
    }
}

impl ObjectStore for FilesystemObjectStore {
    fn put(&mut self, kind: ObjectKind, payload: &[u8]) -> Result<ObjectId> {
        let id = object_id(kind, payload);
        self.write(id, payload)?;
        Ok(id)
    }

    fn put_verified(&mut self, kind: ObjectKind, expected: ObjectId, payload: &[u8]) -> Result<()> {
        let actual = object_id(kind, payload);
        if actual != expected {
            return Err(Error::CorruptObject { expected, actual });
        }
        self.write(expected, payload)
    }

    fn get(&self, id: ObjectId) -> Result<Arc<[u8]>> {
        let (_, path) = self.locate(id);
        match fs::read(&path) {
            Ok(bytes) => Ok(Arc::from(bytes)),
            Err(error) if error.kind() == ErrorKind::NotFound => Err(Error::NotFound(id)),
            Err(error) => Err(storage(&path, &error)),
        }
    }

    fn contains(&self, id: ObjectId) -> Result<bool> {
        let (_, path) = self.locate(id);
        path.try_exists().map_err(|error| storage(&path, &error))
    }
}

/// Writes `payload` to `temp`, flushes it, and renames it onto `path`.
fn write_temp(temp: &Path, path: &Path, payload: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp)
        .map_err(|error| storage(temp, &error))?;
    file.write_all(payload)
        .and_then(|()| file.sync_all())
        .map_err(|error| storage(temp, &error))?;
    drop(file);
    fs::rename(temp, path).map_err(|error| storage(path, &error))
}

/// A temp file name unique to this process and call.
fn temp_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!(
        "{TEMP_PREFIX}{}-{}-{nanos}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    )
}

/// A storage error naming the path and the kind of I/O failure.
fn storage(path: &Path, error: &std::io::Error) -> Error {
    Error::Storage {
        path: path.display().to_string(),
        kind: error.kind(),
    }
}

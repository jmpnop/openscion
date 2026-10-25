//! A tier's local byte store: object bytes keyed by id, behind a port so one
//! read-through [`Store`](crate::Store) serves both an on-disk daemon and an
//! in-memory cache.
//!
//! [`ObjectBacking`] is the port. [`ObjectCache`] is the on-disk
//! implementation: objects live at `<root>/<aa>/<rest>`, the two-level fanout
//! git uses for loose objects, written atomically — to a temp file, flushed
//! with `fsync`, then renamed into place — so a reader never sees a partial
//! object and a crash leaves at most an orphaned temp file, which
//! [`ObjectCache::open`] sweeps.
//!
//! Unlike the engine's append-only `ObjectStore`, a backing supports
//! [`remove`](ObjectBacking::remove), so a store can evict. It is keyed purely
//! by id: it stores and serves bytes and never needs an object's kind.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use openscion_core::ObjectId;

use crate::{Error, Result};

/// A tier's local store of object bytes keyed by id, with removal for eviction.
///
/// Implementations are content-addressed byte stores: they neither verify nor
/// interpret the bytes, only hold and return them. A store caches verified
/// objects here and evicts by removing them.
pub trait ObjectBacking: Send + Sync {
    /// Whether the backing holds `id`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the backing cannot be queried.
    fn contains(&self, id: ObjectId) -> Result<bool>;

    /// The bytes of `id`, or `None` when the backing does not hold it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the backing cannot be read.
    fn read(&self, id: ObjectId) -> Result<Option<Arc<[u8]>>>;

    /// Writes `payload` under `id` if it is not already stored.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the backing cannot be written.
    fn write(&self, id: ObjectId, payload: &[u8]) -> Result<()>;

    /// Removes `id`. Removing an id the backing does not hold is a success, so a
    /// racing double-eviction is harmless.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the removal fails for a reason other than the
    /// id being absent.
    fn remove(&self, id: ObjectId) -> Result<()>;
}

/// The prefix every temp file name starts with.
const TEMP_PREFIX: &str = ".tmp-";

/// Makes temp file names unique within a process.
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// An on-disk [`ObjectBacking`]: object bytes keyed by id, with removal.
#[derive(Debug)]
pub struct ObjectCache {
    root: PathBuf,
}

impl ObjectCache {
    /// Opens the cache rooted at `root`, creating the directory if absent and
    /// removing any temp files a previous crash left behind.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when `root` cannot be created or read.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(io(&root))?;
        let cache = Self { root };
        cache.sweep_temps()?;
        Ok(cache)
    }

    /// The fanout directory and the full path an object with `id` lives at.
    fn locate(&self, id: ObjectId) -> (PathBuf, PathBuf) {
        let hex = id.to_string();
        let (prefix, rest) = hex.split_at(2);
        let dir = self.root.join(prefix);
        let path = dir.join(rest);
        (dir, path)
    }

    /// Removes temp files left by an interrupted write.
    fn sweep_temps(&self) -> Result<()> {
        let entries = fs::read_dir(&self.root).map_err(io(&self.root))?;
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

impl ObjectBacking for ObjectCache {
    fn contains(&self, id: ObjectId) -> Result<bool> {
        let (_, path) = self.locate(id);
        path.try_exists().map_err(io(&path))
    }

    fn read(&self, id: ObjectId) -> Result<Option<Arc<[u8]>>> {
        let (_, path) = self.locate(id);
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(Arc::from(bytes))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Error::Io {
                path,
                source: error,
            }),
        }
    }

    fn write(&self, id: ObjectId, payload: &[u8]) -> Result<()> {
        if self.contains(id)? {
            return Ok(());
        }
        let (dir, path) = self.locate(id);
        fs::create_dir_all(&dir).map_err(io(&dir))?;
        let temp = dir.join(temp_name());
        if let Err(error) = write_temp(&temp, &path, payload) {
            // A partial or un-renamed temp file never becomes an object; drop it.
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
        // Make the new directory entry durable; a failure here is not fatal, as
        // the object is already renamed into place and readable.
        if let Ok(handle) = File::open(&dir) {
            let _ = handle.sync_all();
        }
        Ok(())
    }

    fn remove(&self, id: ObjectId) -> Result<()> {
        let (_, path) = self.locate(id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Error::Io {
                path,
                source: error,
            }),
        }
    }
}

/// Writes `payload` to `temp`, flushes it, and renames it onto `path`.
fn write_temp(temp: &Path, path: &Path, payload: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp)
        .map_err(io(temp))?;
    file.write_all(payload)
        .and_then(|()| file.sync_all())
        .map_err(io(temp))?;
    drop(file);
    fs::rename(temp, path).map_err(io(path))
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

/// Maps an I/O failure at `path` to [`Error::Io`].
fn io(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod test;

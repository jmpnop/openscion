//! The working copy: files on disk plus the state that ties them to a
//! changeset.
//!
//! A working copy is a directory with a `.opnscn/state.json` recording the
//! server, the ref, the base changeset, and every tracked file's blob id and
//! stat data. A scan walks the directory and reports every file with its
//! kind, size, modification time, and inode change time; a file whose stat
//! data matches the state is not re-read, which is what keeps `opnscn commit`
//! proportional to what changed rather than to the size of the tree.
//!
//! The cache is trusted only when it cannot hide a change: the modification
//! time alone can be restored by user space (an archive extraction or a
//! `touch -r` can leave a same-size edit looking untouched), so a file counts
//! as unchanged only when its inode change time matches too. A content write
//! always advances the change time and nothing can reset it, so the only
//! residual is a platform or filesystem that does not report one, where the
//! change time is `0`, is never trusted, and the file is re-read. The cost is
//! paid toward re-reading, never toward a missed change.
//!
//! Symlinks are stored as their target. On Unix a file with any execute bit
//! is an executable. Paths that are not UTF-8 are rejected.

mod types;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use openscion_core::{EntryKind, ObjectId, ObjectKind, object_id};

use crate::{Error, Result};
pub(crate) use types::{FileState, Scanned, State};

/// The directory `opnscn` keeps its state in, at the working-copy root.
pub(crate) const STATE_DIR: &str = ".opnscn";
const STATE_FILE: &str = "state.json";

pub(crate) fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn state_path(root: &Path) -> PathBuf {
    root.join(STATE_DIR).join(STATE_FILE)
}

/// Whether `root` already holds a working copy.
pub(crate) fn is_working_copy(root: &Path) -> bool {
    state_path(root).exists()
}

/// Reads the state of the working copy at `root`.
pub(crate) fn load(root: &Path) -> Result<State> {
    let path = state_path(root);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::NotAWorkingCopy(root.to_path_buf()));
        }
        Err(source) => return Err(Error::Io { path, source }),
    };
    serde_json::from_slice(&bytes).map_err(|error| Error::BadState {
        path,
        message: error.to_string(),
    })
}

/// Writes `state`, stamping `written_at` with the current time.
pub(crate) fn save(root: &Path, state: &mut State) -> Result<()> {
    let dir = root.join(STATE_DIR);
    fs::create_dir_all(&dir).map_err(io_error(&dir))?;
    state.written_at = now();
    let path = state_path(root);
    let bytes = serde_json::to_vec(state).map_err(|error| Error::BadState {
        path: path.clone(),
        message: error.to_string(),
    })?;
    fs::write(&path, bytes).map_err(io_error(&path))
}

fn now() -> u64 {
    nanos(SystemTime::now())
}

fn nanos(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |elapsed| {
        u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
    })
}

/// Every file under `root` except the state directory, sorted by path.
pub(crate) fn scan(root: &Path) -> Result<Vec<Scanned>> {
    let mut out = Vec::new();
    walk(root, "", &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<Scanned>) -> Result<()> {
    for entry in fs::read_dir(dir).map_err(io_error(dir))? {
        let entry = entry.map_err(io_error(dir))?;
        let full = entry.path();
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| Error::NonUtf8Path(full.clone()))?;
        if prefix.is_empty() && name == STATE_DIR {
            continue;
        }
        let path = format!("{prefix}{name}");
        let meta = fs::symlink_metadata(&full).map_err(io_error(&full))?;
        let kind = if meta.is_dir() {
            walk(&full, &format!("{path}/"), out)?;
            continue;
        } else if meta.file_type().is_symlink() {
            EntryKind::Symlink
        } else if is_executable(&meta) {
            EntryKind::Executable
        } else {
            EntryKind::File
        };
        out.push(Scanned {
            path,
            kind,
            size: meta.len(),
            mtime: meta.modified().map_or(0, nanos),
            ctime: change_time(&meta),
        });
    }
    Ok(())
}

/// The inode change time of `meta` in nanoseconds since the epoch, or `0` on a
/// platform that does not expose one. A content write always advances it and
/// user space cannot set it, which is why it, not `mtime`, decides whether a
/// file can be trusted from its stat data.
#[cfg(unix)]
fn change_time(meta: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    let secs = u64::try_from(meta.ctime()).unwrap_or(0);
    secs.saturating_mul(1_000_000_000)
        .saturating_add(u64::from(meta.ctime_nsec().try_into().unwrap_or(0u32)))
}

#[cfg(not(unix))]
fn change_time(_: &fs::Metadata) -> u64 {
    0
}

#[cfg(unix)]
fn is_executable(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_: &fs::Metadata) -> bool {
    false
}

/// The bytes a file is stored as: its contents, or a symlink's target.
pub(crate) fn read_content(root: &Path, file: &Scanned) -> Result<Vec<u8>> {
    let full = root.join(&file.path);
    if file.kind == EntryKind::Symlink {
        let target = fs::read_link(&full).map_err(io_error(&full))?;
        Ok(target.as_os_str().as_encoded_bytes().to_vec())
    } else {
        fs::read(&full).map_err(io_error(&full))
    }
}

/// Whether `file` can be trusted to still hold `known` without re-reading it:
/// same kind, size, modification time, and inode change time, and modified
/// strictly before the state was written.
///
/// A known change time of `0` means none was recorded (an older state, or a
/// platform without one) and is never trusted, so the file is re-read. The
/// change time is what makes this safe: `mtime` alone can be restored to hide
/// a same-size edit, but a write always advances the change time.
pub(crate) fn unchanged(file: &Scanned, known: &FileState, written_at: u64) -> bool {
    known.ctime != 0
        && file.ctime == known.ctime
        && file.kind == known.kind
        && file.size == known.size
        && file.mtime == known.mtime
        && file.mtime < written_at
}

/// How a scanned file compares to the base changeset the state records.
///
/// Both of [`classify`]'s callers decide a file's state through this one
/// function — `opnscn status` to report it and `opnscn commit` to record it —
/// so the two can never disagree about whether a file changed.
pub(crate) enum Classified {
    /// The file matches the base. `refreshed` is `Some` only when the file had
    /// to be re-read because its stat data had moved though its bytes had not;
    /// a caller that keeps a stat cache stores it so the next scan can skip the
    /// re-read, and a read-only caller ignores it.
    Clean {
        /// A refreshed [`FileState`] to replace the cached one.
        refreshed: Option<FileState>,
    },
    /// The file is new, or its bytes or kind differ from the base.
    Changed {
        /// Whether the base already tracked this path: `false` is an addition,
        /// `true` a modification.
        known_before: bool,
        /// The blob the current contents hash to.
        blob: ObjectId,
        /// The current contents — a symlink's target for a symlink — ready to
        /// upload.
        bytes: Vec<u8>,
        /// The stat data and blob to record for the file.
        fresh: FileState,
    },
}

/// Classifies one scanned file against the base changeset recorded in `state`.
///
/// Applies the [`unchanged`] stat fast path first; only when that cannot prove
/// the file clean does it re-read the file and compare blob ids, so an
/// unchanged tree costs no reads. Deciding by blob id is what makes the answer
/// reflect content rather than a timestamp: a file whose modification time
/// moved but whose bytes did not classifies [`Classified::Clean`]. See
/// [`Classified`] for what each outcome carries.
///
/// # Errors
///
/// Returns [`Error::Io`] when the file cannot be read.
pub(crate) fn classify(root: &Path, file: &Scanned, state: &State) -> Result<Classified> {
    let known = state.files.get(&file.path);
    if known.is_some_and(|known| unchanged(file, known, state.written_at)) {
        return Ok(Classified::Clean { refreshed: None });
    }
    let bytes = read_content(root, file)?;
    let blob = blob_id(&bytes);
    let fresh = FileState {
        kind: file.kind,
        blob,
        size: file.size,
        mtime: file.mtime,
        ctime: file.ctime,
    };
    // Re-read but identical to the base: clean, with its stat data refreshed.
    if known.is_some_and(|known| known.kind == file.kind && known.blob == blob) {
        return Ok(Classified::Clean {
            refreshed: Some(fresh),
        });
    }
    Ok(Classified::Changed {
        known_before: known.is_some(),
        blob,
        bytes,
        fresh,
    })
}

/// Hashes `bytes` as a blob.
pub(crate) fn blob_id(bytes: &[u8]) -> ObjectId {
    object_id(ObjectKind::Blob, bytes)
}

/// Writes one file of a checkout, creating its parent directories.
pub(crate) fn write_file(root: &Path, path: &str, kind: EntryKind, bytes: &[u8]) -> Result<()> {
    let full = root.join(path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).map_err(io_error(parent))?;
    }
    match kind {
        EntryKind::Symlink => write_symlink(&full, bytes),
        EntryKind::Executable => {
            fs::write(&full, bytes).map_err(io_error(&full))?;
            make_executable(&full)
        }
        EntryKind::File | EntryKind::Tree => fs::write(&full, bytes).map_err(io_error(&full)),
    }
}

#[cfg(unix)]
fn write_symlink(full: &Path, target: &[u8]) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(target), full).map_err(io_error(full))
}

#[cfg(not(unix))]
fn write_symlink(full: &Path, target: &[u8]) -> Result<()> {
    fs::write(full, target).map_err(io_error(full))
}

#[cfg(unix)]
fn make_executable(full: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(full, fs::Permissions::from_mode(0o755)).map_err(io_error(full))
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<()> {
    Ok(())
}

/// The stat data of a file just written, for the state: size, modification
/// time, and inode change time.
pub(crate) fn stat(root: &Path, path: &str) -> Result<(u64, u64, u64)> {
    let full = root.join(path);
    let meta = fs::symlink_metadata(&full).map_err(io_error(&full))?;
    Ok((
        meta.len(),
        meta.modified().map_or(0, nanos),
        change_time(&meta),
    ))
}

#[cfg(test)]
mod test;

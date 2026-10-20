//! The snapshot file: the warm set as lowercase hex object ids, one per line.
//!
//! [`read_ids`] loads the ids a previous daemon persisted — a missing file is an
//! empty set, so a first boot is not an error — and [`write_ids`] replaces the
//! file atomically with the current set. Blank lines and `#` comments are
//! ignored on read, so the file stays inspectable and hand-editable.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use openscion_core::ObjectId;

use crate::{Error, Result};

/// Reads the object ids in the snapshot file at `path`, in file order.
///
/// A file that does not exist is an empty set. Blank lines and lines beginning
/// with `#` are ignored.
///
/// # Errors
///
/// Returns [`Error::Io`] when the file exists but cannot be read, or when a
/// non-blank, non-comment line is not a 64-character lowercase hex object id.
pub(crate) fn read_ids(path: &Path) -> Result<Vec<ObjectId>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(Error::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let mut ids = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let id = line.parse::<ObjectId>().map_err(|error| Error::Io {
            path: path.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()),
        })?;
        ids.push(id);
    }
    Ok(ids)
}

/// Writes `ids` to the snapshot file at `path`, one lowercase hex id per line,
/// replacing any existing file atomically: a temporary file in the same
/// directory, then a rename.
///
/// # Errors
///
/// Returns [`Error::Io`] when the temporary file cannot be written or the
/// rename fails.
pub(crate) fn write_ids(path: &Path, ids: &[ObjectId]) -> Result<()> {
    let mut body = String::with_capacity(ids.len() * (2 * ObjectId::LEN + 1));
    for id in ids {
        // Writing a hex id and a newline into a `String` cannot fail.
        let _ = writeln!(body, "{id}");
    }
    let tmp = temp_path(path);
    fs::write(&tmp, body.as_bytes()).map_err(|source| Error::Io {
        path: tmp.clone(),
        source,
    })?;
    fs::rename(&tmp, path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// A sibling temporary path for the atomic write: the target's name with a
/// process-specific suffix, in the same directory so the rename stays on one
/// filesystem.
fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp.{}", std::process::id()));
    path.with_file_name(name)
}

#[cfg(test)]
mod test;

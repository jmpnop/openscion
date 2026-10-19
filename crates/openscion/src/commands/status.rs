//! `opnscn status`: report a working copy's pending changes, read-only.

use std::collections::HashSet;
use std::path::Path;

use crate::Result;
use crate::workcopy::{self, Classified};

/// One of the three ways a path can differ from the base changeset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusCode {
    /// On disk, not tracked in the base.
    Added,
    /// Tracked, but its bytes or kind differ from the base.
    Modified,
    /// Tracked in the base, absent from disk.
    Deleted,
}

impl StatusCode {
    /// The single letter the command prints for this state: `A`, `M`, or `D`.
    pub(crate) fn letter(self) -> char {
        match self {
            Self::Added => 'A',
            Self::Modified => 'M',
            Self::Deleted => 'D',
        }
    }
}

/// The pending changes in the working copy at `root`, sorted by path.
///
/// Compares the files on disk to the base changeset recorded in the state,
/// deciding by content: a file is [`StatusCode::Modified`] only when its bytes
/// or kind differ, never for a moved timestamp alone. It reads the state and
/// the files on disk and nothing else — no server request — and writes
/// nothing. The changes it returns are exactly the set `opnscn commit` would
/// record from the same state, since both classify through
/// [`workcopy::classify`].
///
/// # Errors
///
/// Returns [`Error::NotAWorkingCopy`] when `root` holds no working-copy state,
/// and [`Error::Io`] or [`Error::NonUtf8Path`] when the tree cannot be scanned
/// or a changed file cannot be read.
///
/// [`Error::NotAWorkingCopy`]: crate::Error::NotAWorkingCopy
/// [`Error::Io`]: crate::Error::Io
/// [`Error::NonUtf8Path`]: crate::Error::NonUtf8Path
pub(crate) fn status(root: &Path) -> Result<Vec<(String, StatusCode)>> {
    let state = workcopy::load(root)?;
    let scanned = workcopy::scan(root)?;
    let mut present = HashSet::with_capacity(scanned.len());
    let mut changes = Vec::new();
    for file in &scanned {
        present.insert(file.path.as_str());
        if let Classified::Changed { known_before, .. } = workcopy::classify(root, file, &state)? {
            let code = if known_before {
                StatusCode::Modified
            } else {
                StatusCode::Added
            };
            changes.push((file.path.clone(), code));
        }
    }
    for path in state.files.keys() {
        if !present.contains(path.as_str()) {
            changes.push((path.clone(), StatusCode::Deleted));
        }
    }
    changes.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(changes)
}

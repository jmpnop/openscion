//! The watcher's event type and its pure, carried-forward session state.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::Result;
use crate::commands::StatusCode;
use crate::workcopy::{self, Classified, State};

/// A filesystem change, as the pure session consumes it. The operating-system
/// adapter turns native events into these; the session never sees a native
/// event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WatchEvent {
    /// These paths — relative to the working-copy root, `/`-separated — may have
    /// changed and should be re-examined on the next render.
    Changed(Vec<String>),
    /// The event stream may be incomplete (an overflow, a rename, a
    /// directory-level change, a path outside the root), so the next render must
    /// be a full scan rather than trust the named paths.
    Rescan,
}

/// The watcher's carried-forward view of a working copy's pending changes.
///
/// It holds the base changeset's state and the current report, and folds events
/// into it: a [`WatchEvent::Changed`] marks candidate paths to re-examine, a
/// [`WatchEvent::Rescan`] marks that the next refresh must scan the whole tree.
/// A [`refresh`](Self::refresh) re-examines only the candidates (or rescans),
/// deciding each path through the same [`workcopy::classify`] that `status` and
/// `commit` use, so the report it carries always equals a full `opnscn status`.
pub(crate) struct WatchSession {
    root: PathBuf,
    state: State,
    report: BTreeMap<String, StatusCode>,
    candidates: BTreeSet<String>,
    full_rescan: bool,
}

impl WatchSession {
    /// Opens a session on the working copy at `root`. The first
    /// [`refresh`](Self::refresh) is a full scan, so a copy that is already
    /// dirty is reported in full.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotAWorkingCopy`](crate::Error::NotAWorkingCopy) when
    /// `root` holds no working-copy state.
    pub(crate) fn open(root: &Path) -> Result<Self> {
        let state = workcopy::load(root)?;
        Ok(Self {
            root: root.to_path_buf(),
            state,
            report: BTreeMap::new(),
            candidates: BTreeSet::new(),
            full_rescan: true,
        })
    }

    /// Folds one event into the pending state, to be reflected by the next
    /// [`refresh`](Self::refresh).
    pub(crate) fn apply(&mut self, event: WatchEvent) {
        match event {
            WatchEvent::Changed(paths) => self.candidates.extend(paths),
            WatchEvent::Rescan => self.full_rescan = true,
        }
    }

    /// Brings the report up to date and returns it, sorted by path.
    ///
    /// Re-scans the whole tree when a rescan is pending; otherwise re-examines
    /// only the candidate paths accumulated since the last refresh, carrying
    /// every other path's state forward. A candidate that re-reads to the base
    /// drops off, a tracked candidate now absent becomes a deletion, and a
    /// candidate that differs is an addition or a modification.
    ///
    /// # Errors
    ///
    /// Returns the working-copy error when the tree or a candidate cannot be
    /// read.
    pub(crate) fn refresh(&mut self) -> Result<&BTreeMap<String, StatusCode>> {
        if self.full_rescan {
            self.report = crate::commands::status(&self.root)?.into_iter().collect();
            self.full_rescan = false;
            self.candidates.clear();
            return Ok(&self.report);
        }
        for path in std::mem::take(&mut self.candidates) {
            match workcopy::scan_one(&self.root, &path)? {
                Some(scanned) => match workcopy::classify(&self.root, &scanned, &self.state)? {
                    Classified::Changed { known_before, .. } => {
                        let code = if known_before {
                            StatusCode::Modified
                        } else {
                            StatusCode::Added
                        };
                        self.report.insert(path, code);
                    }
                    Classified::Clean { .. } => {
                        self.report.remove(&path);
                    }
                },
                None if self.state.files.contains_key(&path) => {
                    self.report.insert(path, StatusCode::Deleted);
                }
                None => {
                    self.report.remove(&path);
                }
            }
        }
        Ok(&self.report)
    }
}

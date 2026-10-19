//! `opnscn sparse`: show and edit which paths a working copy materializes.
//!
//! `show` reads the recorded set. `add` widens the checkout, fetching the named
//! paths from the server at the working copy's base — reusing the same
//! [`fetch_into`](super::clone::fetch_into) that `clone` uses — and tracking
//! them. `remove` narrows it, deleting the named entries' files from disk and
//! from tracking, without touching the repository. All three keep the recorded
//! set and the tracked files in step.

use std::path::Path;

use openscion_core::RepoPath;

use super::clone::fetch_into;
use crate::workcopy::{self, Sparse};
use crate::{Error, Result, ServerClient};

/// The working copy's current sparse set.
///
/// # Errors
///
/// Returns [`Error::NotAWorkingCopy`](crate::Error::NotAWorkingCopy) when `root`
/// is not a working copy.
pub(crate) fn show(root: &Path) -> Result<Sparse> {
    Ok(workcopy::load(root)?.sparse)
}

/// Widens the sparse checkout at `root` to include `paths`, fetching and
/// materializing each from the server at the working copy's base changeset, and
/// tracking the files. Returns the number of files materialized.
///
/// A path already covered by the set is skipped. A path that names nothing in
/// the snapshot adds the entry and zero files.
///
/// # Errors
///
/// Returns [`Error::FullCheckout`] on a full checkout, the client errors when
/// the fetch fails, and the working-copy write errors.
pub(crate) fn add(root: &Path, paths: &[RepoPath]) -> Result<usize> {
    let mut state = workcopy::load(root)?;
    if state.sparse.is_full() {
        return Err(Error::FullCheckout);
    }
    let client = ServerClient::new(&state.server);
    let root_tree = client.changeset(state.base)?.info.root_tree;
    let mut added = 0;
    for path in paths {
        if state.sparse.covers(path.as_str()) {
            continue;
        }
        let filter: Vec<&str> = path.components().collect();
        let files = fetch_into(&client, root_tree, &filter, root)?;
        added += files.len();
        for (name, file) in files {
            state.files.insert(name, file);
        }
        state.sparse.insert(path.as_str().to_owned());
    }
    workcopy::save(root, &mut state)?;
    Ok(added)
}

/// Narrows the sparse checkout at `root` by dropping `paths`, deleting each
/// entry's materialized files from disk and from tracking and pruning emptied
/// directories. Returns the number of files removed. Makes no server request.
///
/// # Errors
///
/// Returns [`Error::FullCheckout`] on a full checkout, [`Error::NotInSparse`]
/// when a path is not an entry in the set, and the working-copy removal errors.
pub(crate) fn remove(root: &Path, paths: &[RepoPath]) -> Result<usize> {
    let mut state = workcopy::load(root)?;
    if state.sparse.is_full() {
        return Err(Error::FullCheckout);
    }
    let mut removed = 0;
    for path in paths {
        let prefix = path.as_str();
        if !state.sparse.remove(prefix) {
            return Err(Error::NotInSparse(prefix.to_owned()));
        }
        // The entry's files are exactly those a one-entry cone of it covers —
        // the same membership rule the rest of the set uses.
        let entry = Sparse::cone([prefix.to_owned()]);
        let under: Vec<String> = state
            .files
            .keys()
            .filter(|tracked| entry.covers(tracked))
            .cloned()
            .collect();
        for tracked in under {
            workcopy::remove_file(root, &tracked)?;
            state.files.remove(&tracked);
            removed += 1;
        }
    }
    workcopy::save(root, &mut state)?;
    Ok(removed)
}

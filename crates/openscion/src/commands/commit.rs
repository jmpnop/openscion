//! `opnscn commit`: record what changed since the base changeset.

use std::collections::HashSet;
use std::path::Path;

use openscion_core::{Change, ChangesetInfo, CreateChangeset, RepoPath, UpdateRef};

use super::upload;
use crate::workcopy::{self, FileState};
use crate::{Error, Result, ServerClient};

/// Commits every change in the working copy at `root` and moves its ref.
/// Returns the new changeset and the number of changes.
pub(crate) fn commit(root: &Path, author: &str, message: &str) -> Result<(ChangesetInfo, usize)> {
    let mut state = workcopy::load(root)?;
    let client = ServerClient::new(&state.server);
    let scanned = workcopy::scan(root)?;

    let mut changes = Vec::new();
    let mut blobs = Vec::new();
    let mut seen = HashSet::new();
    let mut present = HashSet::with_capacity(scanned.len());
    for file in &scanned {
        present.insert(file.path.as_str());
        let known = state.files.get(&file.path);
        if known.is_some_and(|known| workcopy::unchanged(file, known, state.written_at)) {
            continue;
        }
        let bytes = workcopy::read_content(root, file)?;
        let blob = workcopy::blob_id(&bytes);
        let fresh = FileState {
            kind: file.kind,
            blob,
            size: file.size,
            mtime: file.mtime,
            ctime: file.ctime,
        };
        // Touched but identical: refresh the stat data, record no change.
        if known.is_some_and(|known| known.kind == file.kind && known.blob == blob) {
            state.files.insert(file.path.clone(), fresh);
            continue;
        }
        changes.push(Change::Upsert {
            path: RepoPath::new(file.path.as_str())?,
            kind: file.kind,
            blob,
        });
        state.files.insert(file.path.clone(), fresh);
        if seen.insert(blob) {
            blobs.push((blob, bytes));
        }
    }
    let deleted: Vec<String> = state
        .files
        .keys()
        .filter(|path| !present.contains(path.as_str()))
        .cloned()
        .collect();
    for path in deleted {
        changes.push(Change::Delete {
            path: RepoPath::new(path.as_str())?,
        });
        state.files.remove(&path);
    }
    if changes.is_empty() {
        return Err(Error::NothingToCommit);
    }

    upload(&client, &blobs)?;
    let count = changes.len();
    let request = CreateChangeset::new(vec![state.base], changes, author, message);
    let info = client.create_changeset(&request)?;
    client.update_ref(&state.ref_name, UpdateRef::new(Some(state.base), info.id))?;
    state.base = info.id;
    workcopy::save(root, &mut state)?;
    Ok((info, count))
}

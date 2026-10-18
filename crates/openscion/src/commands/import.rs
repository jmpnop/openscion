//! `opnscn import`: record a directory as the first changeset.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use openscion_core::{Change, ChangesetInfo, CreateChangeset, RepoPath, UpdateRef};

use super::upload;
use crate::workcopy::{self, FileState, State};
use crate::{Error, Result, ServerClient};

/// Imports every file under `root` as a parentless changeset, points
/// `ref_name` at it, and makes `root` a working copy.
pub(crate) fn import(
    client: &ServerClient,
    root: &Path,
    ref_name: &str,
    author: &str,
    message: &str,
) -> Result<(ChangesetInfo, usize)> {
    if workcopy::is_working_copy(root) {
        return Err(Error::AlreadyExists(root.to_path_buf()));
    }
    let scanned = workcopy::scan(root)?;
    let mut blobs = Vec::new();
    let mut seen = HashSet::new();
    let mut changes = Vec::with_capacity(scanned.len());
    let mut files = BTreeMap::new();
    for file in &scanned {
        let bytes = workcopy::read_content(root, file)?;
        let blob = workcopy::blob_id(&bytes);
        changes.push(Change::Upsert {
            path: RepoPath::new(file.path.as_str())?,
            kind: file.kind,
            blob,
        });
        files.insert(
            file.path.clone(),
            FileState {
                kind: file.kind,
                blob,
                size: file.size,
                mtime: file.mtime,
                ctime: file.ctime,
            },
        );
        if seen.insert(blob) {
            blobs.push((blob, bytes));
        }
    }
    upload(client, &blobs)?;
    let info = client.create_changeset(&CreateChangeset::new(vec![], changes, author, message))?;
    client.update_ref(ref_name, UpdateRef::new(None, info.id))?;
    let mut state = State {
        server: client.base().to_owned(),
        ref_name: ref_name.to_owned(),
        base: info.id,
        sparse: None,
        written_at: 0,
        files,
    };
    workcopy::save(root, &mut state)?;
    Ok((info, scanned.len()))
}

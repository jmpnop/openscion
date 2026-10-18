//! `opnscn clone`: materialize a snapshot, or one directory of it.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use openscion_core::{
    EntryKind, ObjectId, ObjectKind, RepoPath, decode_frames, decode_tree, object_id,
};

use super::FETCH_BATCH;
use crate::workcopy::{self, FileState, State};
use crate::{Error, Result, ServerClient};

/// A file to fetch: its path, kind, and blob.
type Wanted = (String, EntryKind, ObjectId);

/// Clones `ref_name` into `dest`, limited to the directory or file `sparse`
/// when given. Returns the changeset cloned and the number of files written.
pub(crate) fn clone(
    client: &ServerClient,
    ref_name: &str,
    dest: &Path,
    sparse: Option<&RepoPath>,
) -> Result<(ObjectId, usize)> {
    if dest.exists() && !is_empty_dir(dest)? {
        return Err(Error::AlreadyExists(dest.to_path_buf()));
    }
    let target = client.get_ref(ref_name)?.target;
    let root_tree = client.changeset(target)?.info.root_tree;
    let filter: Vec<&str> = sparse.map_or_else(Vec::new, |path| path.components().collect());
    let wanted = walk_trees(client, root_tree, &filter)?;

    std::fs::create_dir_all(dest).map_err(workcopy::io_error(dest))?;
    let mut files = BTreeMap::new();
    for batch in wanted.chunks(FETCH_BATCH) {
        let ids = unique(batch.iter().map(|(_, _, blob)| *blob));
        let bytes = client.fetch_objects(ids.clone())?;
        let blobs: BTreeMap<ObjectId, &[u8]> = verified(&bytes, ObjectKind::Blob, &ids)?;
        for (path, kind, blob) in batch {
            // `verified` returned every requested id, so the lookup holds.
            let content = blobs.get(blob).copied().unwrap_or_default();
            workcopy::write_file(dest, path, *kind, content)?;
            let (size, mtime, ctime) = workcopy::stat(dest, path)?;
            files.insert(
                path.clone(),
                FileState {
                    kind: *kind,
                    blob: *blob,
                    size,
                    mtime,
                    ctime,
                },
            );
        }
    }
    let count = files.len();
    let mut state = State {
        server: client.base().to_owned(),
        ref_name: ref_name.to_owned(),
        base: target,
        sparse: sparse.map(|path| path.as_str().to_owned()),
        written_at: 0,
        files,
    };
    workcopy::save(dest, &mut state)?;
    Ok((target, count))
}

fn is_empty_dir(path: &Path) -> Result<bool> {
    let mut entries = std::fs::read_dir(path).map_err(workcopy::io_error(path))?;
    Ok(entries.next().is_none())
}

/// Walks the snapshot breadth first, one fetch per level, and returns every
/// file under `filter` (everything when it is empty).
fn walk_trees(client: &ServerClient, root: ObjectId, filter: &[&str]) -> Result<Vec<Wanted>> {
    let mut wanted = Vec::new();
    let mut level: Vec<(String, ObjectId)> = vec![(String::new(), root)];
    let mut depth = 0;
    while !level.is_empty() {
        let mut next = Vec::new();
        for batch in level.chunks(FETCH_BATCH) {
            let ids = unique(batch.iter().map(|(_, id)| *id));
            let bytes = client.fetch_objects(ids.clone())?;
            let trees = verified(&bytes, ObjectKind::Tree, &ids)?;
            for (prefix, id) in batch {
                let entries = decode_tree(trees.get(id).copied().unwrap_or_default())?;
                for entry in entries {
                    // Inside the filter everything is wanted; above it, only
                    // the entry on the way down.
                    if depth < filter.len() && entry.name != filter[depth] {
                        continue;
                    }
                    let path = format!("{prefix}{}", entry.name);
                    if entry.kind == EntryKind::Tree {
                        next.push((format!("{path}/"), entry.id));
                    } else if depth + 1 >= filter.len() {
                        wanted.push((path, entry.kind, entry.id));
                    }
                }
            }
        }
        level = next;
        depth += 1;
    }
    Ok(wanted)
}

fn unique(ids: impl Iterator<Item = ObjectId>) -> Vec<ObjectId> {
    let mut seen = HashSet::new();
    ids.filter(|id| seen.insert(*id)).collect()
}

/// Decodes a fetched frame stream, checking that it holds exactly the
/// requested objects and that each one hashes to its id.
pub(super) fn verified<'a>(
    bytes: &'a [u8],
    kind: ObjectKind,
    requested: &[ObjectId],
) -> Result<BTreeMap<ObjectId, &'a [u8]>> {
    let frames = decode_frames(bytes)?;
    let mut out = BTreeMap::new();
    for (id, body) in frames {
        let actual = object_id(kind, body);
        if actual != id {
            return Err(openscion_core::Error::CorruptObject {
                expected: id,
                actual,
            }
            .into());
        }
        out.insert(id, body);
    }
    if let Some(missing) = requested.iter().find(|id| !out.contains_key(id)) {
        return Err(openscion_core::Error::NotFound(*missing).into());
    }
    Ok(out)
}

//! Root-tree derivation by path copying.
//!
//! A changeset's root tree is its first parent's root tree with the changes
//! applied. Only the trees on the path from each changed file to the root are
//! loaded and rewritten; every other subtree stays an unloaded
//! `(kind, id)` entry and keeps its id, so deriving costs the number of
//! changes times their depth, never the size of the repository
//! (`docs/specs/openscion-data-model.md#root-trees`).
//!
//! Deletes are applied before upserts. Changes arrive sorted by path, and
//! applying every delete first lets one changeset replace a file with a
//! directory, or a directory with a file, in either order.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use openscion_proto::{Change, EntryKind, ObjectId, ObjectKind, TreeEntry};

use crate::{Error, ObjectStore, Result, decode_tree, encode_tree};

/// One directory while it is being rewritten.
#[derive(Debug, Default)]
struct Node {
    entries: BTreeMap<String, Slot>,
}

/// A directory entry: untouched (only its kind and id are known), or a loaded
/// subdirectory that changes pass through.
#[derive(Debug)]
enum Slot {
    Entry(EntryKind, ObjectId),
    Dir(Node),
}

/// Applies `changes` to the tree `base` and writes every new tree to
/// `objects`, returning the new root tree's id.
pub(super) fn derive_root_tree(
    objects: &mut dyn ObjectStore,
    base: ObjectId,
    changes: &[Change],
) -> Result<ObjectId> {
    let mut root = load(objects, base)?;
    for change in changes {
        if let Change::Delete { path } = change {
            let components: Vec<_> = path.components().collect();
            delete(objects, &mut root, &components, path.as_str())?;
        }
    }
    for change in changes {
        if let Change::Upsert { path, kind, blob } = change {
            let components: Vec<_> = path.components().collect();
            upsert(
                objects,
                &mut root,
                &components,
                (*kind, *blob),
                path.as_str(),
            )?;
        }
    }
    match write(objects, root)? {
        Some(id) => Ok(id),
        None => objects.put(ObjectKind::Tree, &[]),
    }
}

fn load(objects: &dyn ObjectStore, id: ObjectId) -> Result<Node> {
    let entries = decode_tree(&objects.get(id)?)?
        .into_iter()
        .map(|entry| (entry.name, Slot::Entry(entry.kind, entry.id)))
        .collect();
    Ok(Node { entries })
}

fn invalid(path: &str, rule: &'static str) -> Error {
    Error::InvalidChange {
        path: path.to_owned(),
        rule,
    }
}

/// Returns the loaded subdirectory `name` of `node`, loading it if it is still
/// an untouched tree entry, or `None` if `name` is absent or not a directory.
fn subdir<'a>(
    objects: &dyn ObjectStore,
    node: &'a mut Node,
    name: &str,
) -> Result<Option<&'a mut Node>> {
    let Some(slot) = node.entries.get_mut(name) else {
        return Ok(None);
    };
    if let Slot::Entry(EntryKind::Tree, id) = *slot {
        *slot = Slot::Dir(load(objects, id)?);
    }
    Ok(match slot {
        Slot::Dir(child) => Some(child),
        Slot::Entry(..) => None,
    })
}

fn delete(
    objects: &dyn ObjectStore,
    node: &mut Node,
    components: &[&str],
    path: &str,
) -> Result<()> {
    let (name, rest) = components
        .split_first()
        .ok_or_else(|| invalid(path, "empty path"))?;
    if rest.is_empty() {
        return match node.entries.get(*name) {
            Some(Slot::Entry(kind, _)) if *kind != EntryKind::Tree => {
                node.entries.remove(*name);
                Ok(())
            }
            Some(_) => Err(invalid(path, "deletes a directory")),
            None => Err(invalid(path, "deletes a missing file")),
        };
    }
    let child =
        subdir(objects, node, name)?.ok_or_else(|| invalid(path, "deletes a missing file"))?;
    delete(objects, child, rest, path)?;
    if child.entries.is_empty() {
        node.entries.remove(*name);
    }
    Ok(())
}

fn upsert(
    objects: &dyn ObjectStore,
    node: &mut Node,
    components: &[&str],
    target: (EntryKind, ObjectId),
    path: &str,
) -> Result<()> {
    let (name, rest) = components
        .split_first()
        .ok_or_else(|| invalid(path, "empty path"))?;
    if rest.is_empty() {
        return match node.entries.entry((*name).to_owned()) {
            Entry::Vacant(vacant) => {
                vacant.insert(Slot::Entry(target.0, target.1));
                Ok(())
            }
            Entry::Occupied(mut occupied) => match occupied.get() {
                Slot::Entry(kind, id) if (*kind, *id) == target => {
                    Err(invalid(path, "upsert does not change the file"))
                }
                Slot::Entry(kind, _) if *kind != EntryKind::Tree => {
                    occupied.insert(Slot::Entry(target.0, target.1));
                    Ok(())
                }
                _ => Err(invalid(path, "path is a directory")),
            },
        };
    }
    if !node.entries.contains_key(*name) {
        node.entries
            .insert((*name).to_owned(), Slot::Dir(Node::default()));
    }
    let child = subdir(objects, node, name)?.ok_or_else(|| invalid(path, "a parent is a file"))?;
    upsert(objects, child, rest, target, path)
}

/// Writes every loaded directory under `node`, bottom up, and returns the
/// tree id, or `None` if the directory ended up empty.
fn write(objects: &mut dyn ObjectStore, node: Node) -> Result<Option<ObjectId>> {
    let mut entries = Vec::with_capacity(node.entries.len());
    for (name, slot) in node.entries {
        match slot {
            Slot::Entry(kind, id) => entries.push(TreeEntry::new(name, kind, id)),
            Slot::Dir(child) => {
                if let Some(id) = write(objects, child)? {
                    entries.push(TreeEntry::new(name, EntryKind::Tree, id));
                }
            }
        }
    }
    if entries.is_empty() {
        return Ok(None);
    }
    Ok(Some(objects.put(ObjectKind::Tree, &encode_tree(&entries))?))
}

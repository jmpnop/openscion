//! Snapshot and history queries answered on the server without a checkout.
//!
//! These read-only methods resolve a path against a changeset's root tree
//! ([`Repository::stat`], [`Repository::list_tree`], [`Repository::read_file`]),
//! compare two snapshots ([`Repository::diff`]), and walk history
//! ([`Repository::log`], [`Repository::is_ancestor`],
//! [`Repository::merge_base`]). They load only the trees on the paths they
//! touch, so answering a question costs the size of the answer, not the size of
//! the repository (`docs/specs/openscion-api.md`).
//!
//! Ancestry walks are bounded by generation numbers: a changeset's generation
//! is strictly greater than every ancestor's, so a walk toward the ancestors of
//! a target never descends past the target's generation.

use std::collections::{BinaryHeap, HashSet};
use std::sync::Arc;

use openscion_proto::{Change, ChangesetSummary, EntryKind, ObjectId, RepoPath};

use crate::{Result, decode_tree};

use super::Repository;

impl Repository {
    /// The kind and object id at `path` in a changeset's snapshot, or `None`
    /// when nothing is there.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`](crate::Error::NotFound) when no changeset has this id, and
    /// [`Error::Malformed`](crate::Error::Malformed) when a tree on the path does not decode.
    pub fn stat(
        &self,
        changeset: ObjectId,
        path: &RepoPath,
    ) -> Result<Option<(EntryKind, ObjectId)>> {
        let root = self.entry(changeset)?.info.root_tree;
        self.resolve(root, path)
    }

    /// The contents at `path` in a changeset's snapshot with its kind, or
    /// `None` when `path` is absent or is a directory.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`](crate::Error::NotFound) when no changeset has this id, and
    /// [`Error::Malformed`](crate::Error::Malformed) when a tree on the path does not decode.
    pub fn read_file(
        &self,
        changeset: ObjectId,
        path: &RepoPath,
    ) -> Result<Option<(EntryKind, Arc<[u8]>)>> {
        match self.stat(changeset, path)? {
            Some((kind, id)) if kind != EntryKind::Tree => Ok(Some((kind, self.objects.get(id)?))),
            _ => Ok(None),
        }
    }

    /// Every entry directly under `path` in a changeset's snapshot, or under
    /// the root when `path` is `None`, sorted by path. With `recursive`, every
    /// entry beneath them too. A `path` that names a file lists just that file.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`](crate::Error::NotFound) when no changeset has this id, and
    /// [`Error::Malformed`](crate::Error::Malformed) when a tree on the path does not decode.
    pub fn list_tree(
        &self,
        changeset: ObjectId,
        path: Option<&RepoPath>,
        recursive: bool,
    ) -> Result<Vec<(String, EntryKind, ObjectId)>> {
        let root = self.entry(changeset)?.info.root_tree;
        let (tree, prefix) = match path {
            None => (root, String::new()),
            Some(path) => match self.resolve(root, path)? {
                Some((EntryKind::Tree, id)) => (id, format!("{}/", path.as_str())),
                Some((kind, id)) => return Ok(vec![(path.as_str().to_owned(), kind, id)]),
                None => return Ok(Vec::new()),
            },
        };
        let mut out = Vec::new();
        self.list_into(tree, &prefix, recursive, &mut out)?;
        Ok(out)
    }

    /// The file-level changes that turn `from`'s snapshot into `to`'s, as
    /// `Upsert` and `Delete` changes sorted by path. With `prefixes`, only
    /// changes whose path lies under one of them; an empty `prefixes` keeps
    /// every change. Subtrees with equal ids are identical and are skipped.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`](crate::Error::NotFound) when either changeset is unknown, and
    /// [`Error::Malformed`](crate::Error::Malformed) when a tree does not decode.
    pub fn diff(&self, from: ObjectId, to: ObjectId, prefixes: &[&str]) -> Result<Vec<Change>> {
        let from_root = self.entry(from)?.info.root_tree;
        let to_root = self.entry(to)?.info.root_tree;
        let mut out = Vec::new();
        self.diff_into(Some(from_root), Some(to_root), "", prefixes, &mut out)?;
        out.sort_by(|a, b| a.path().cmp(b.path()));
        Ok(out)
    }

    /// The first-parent history from `from`, newest first, excluding `until`
    /// and stopping after `limit` changesets. With `path`, only the changesets
    /// that changed that path relative to their first parent.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`](crate::Error::NotFound) when a changeset on the walk is unknown, and
    /// [`Error::Malformed`](crate::Error::Malformed) when a tree does not decode.
    pub fn log(
        &self,
        from: ObjectId,
        until: Option<ObjectId>,
        path: Option<&RepoPath>,
        limit: usize,
    ) -> Result<Vec<ChangesetSummary>> {
        let mut out = Vec::new();
        let mut cursor = Some(from);
        while let Some(id) = cursor {
            if out.len() >= limit || Some(id) == until {
                break;
            }
            let entry = self.entry(id)?;
            let first_parent = entry.changeset.parents.first().copied();
            if path.is_none_or(|path| self.touched(entry.info.root_tree, first_parent, path)) {
                let changeset = &entry.changeset;
                out.push(ChangesetSummary::new(
                    entry.info.id,
                    changeset.parents.clone(),
                    changeset.author.clone(),
                    changeset.message.clone(),
                    changeset.created_at,
                ));
            }
            cursor = first_parent;
        }
        Ok(out)
    }

    /// Whether `ancestor` is `descendant` or lies on some parent path back from
    /// it. Generation numbers bound the search.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`](crate::Error::NotFound) when either changeset is unknown.
    pub fn is_ancestor(&self, ancestor: ObjectId, descendant: ObjectId) -> Result<bool> {
        let floor = self.entry(ancestor)?.generation;
        if ancestor == descendant {
            return Ok(true);
        }
        let mut seen = HashSet::new();
        let mut stack = vec![descendant];
        while let Some(id) = stack.pop() {
            if id == ancestor {
                return Ok(true);
            }
            for parent in &self.entry(id)?.changeset.parents {
                // A changeset older than `ancestor` cannot reach it.
                if self.entry(*parent)?.generation >= floor && seen.insert(*parent) {
                    stack.push(*parent);
                }
            }
        }
        Ok(false)
    }

    /// A lowest common ancestor of `a` and `b`: a common ancestor no common
    /// ancestor descends from. `None` when they share none. When several tie at
    /// the greatest generation, the one with the greatest id, so the answer is
    /// deterministic.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`](crate::Error::NotFound) when either changeset is unknown.
    pub fn merge_base(&self, a: ObjectId, b: ObjectId) -> Result<Option<ObjectId>> {
        let ancestors_of_a = self.ancestors(a)?;
        // Walk b's ancestors greatest-generation first; the first one that is
        // also an ancestor of a is a lowest common ancestor.
        let mut seen = HashSet::new();
        let mut frontier = BinaryHeap::new();
        seen.insert(b);
        frontier.push((self.entry(b)?.generation, b));
        while let Some((_, id)) = frontier.pop() {
            if ancestors_of_a.contains(&id) {
                return Ok(Some(id));
            }
            for parent in &self.entry(id)?.changeset.parents {
                if seen.insert(*parent) {
                    frontier.push((self.entry(*parent)?.generation, *parent));
                }
            }
        }
        Ok(None)
    }

    /// Every ancestor of `id`, including `id` itself.
    fn ancestors(&self, id: ObjectId) -> Result<HashSet<ObjectId>> {
        let mut seen = HashSet::new();
        let mut stack = vec![id];
        seen.insert(id);
        while let Some(current) = stack.pop() {
            for parent in &self.entry(current)?.changeset.parents {
                if seen.insert(*parent) {
                    stack.push(*parent);
                }
            }
        }
        Ok(seen)
    }

    /// Whether `path` resolves differently in the tree `root` than in the tree
    /// of `first_parent` (nothing, for a root changeset).
    fn touched(&self, root: ObjectId, first_parent: Option<ObjectId>, path: &RepoPath) -> bool {
        let here = self.resolve(root, path).ok().flatten();
        let there = match first_parent {
            Some(parent) => self
                .entry(parent)
                .and_then(|entry| self.resolve(entry.info.root_tree, path))
                .ok()
                .flatten(),
            None => None,
        };
        here != there
    }

    /// Resolves `path` against the tree `root`, returning the entry it names or
    /// `None` when any component is missing or a non-final component is a file.
    pub(super) fn resolve(
        &self,
        root: ObjectId,
        path: &RepoPath,
    ) -> Result<Option<(EntryKind, ObjectId)>> {
        let components: Vec<&str> = path.components().collect();
        let mut tree = root;
        for (depth, name) in components.iter().enumerate() {
            let Some(entry) = decode_tree(&self.objects.get(tree)?)?
                .into_iter()
                .find(|entry| entry.name == *name)
            else {
                return Ok(None);
            };
            if depth + 1 == components.len() {
                return Ok(Some((entry.kind, entry.id)));
            }
            if entry.kind != EntryKind::Tree {
                return Ok(None);
            }
            tree = entry.id;
        }
        Ok(None)
    }

    /// Appends every entry under the tree `id` to `out`, recursing when asked.
    fn list_into(
        &self,
        id: ObjectId,
        prefix: &str,
        recursive: bool,
        out: &mut Vec<(String, EntryKind, ObjectId)>,
    ) -> Result<()> {
        for entry in decode_tree(&self.objects.get(id)?)? {
            let path = format!("{prefix}{}", entry.name);
            let is_dir = entry.kind == EntryKind::Tree;
            out.push((path.clone(), entry.kind, entry.id));
            if recursive && is_dir {
                self.list_into(entry.id, &format!("{path}/"), recursive, out)?;
            }
        }
        Ok(())
    }

    /// Diffs two trees into `out`. Equal ids (including two absent trees) are
    /// identical and pruned; otherwise each differing file becomes a change.
    fn diff_into(
        &self,
        from: Option<ObjectId>,
        to: Option<ObjectId>,
        prefix: &str,
        prefixes: &[&str],
        out: &mut Vec<Change>,
    ) -> Result<()> {
        if from == to {
            return Ok(());
        }
        let from_entries = self.entries_by_name(from)?;
        let to_entries = self.entries_by_name(to)?;
        let mut names: Vec<&String> = from_entries.keys().chain(to_entries.keys()).collect();
        names.sort_unstable();
        names.dedup();
        for name in names {
            let path = format!("{prefix}{name}");
            match (from_entries.get(name), to_entries.get(name)) {
                (Some(before), Some(after)) if before == after => {}
                (before, after) => {
                    self.diff_entry(before.copied(), after.copied(), &path, prefixes, out)?;
                }
            }
        }
        Ok(())
    }

    /// Diffs one name present on either side, descending into trees.
    fn diff_entry(
        &self,
        before: Option<(EntryKind, ObjectId)>,
        after: Option<(EntryKind, ObjectId)>,
        path: &str,
        prefixes: &[&str],
        out: &mut Vec<Change>,
    ) -> Result<()> {
        let before_tree = subtree(before);
        let after_tree = subtree(after);
        if before_tree.is_some() || after_tree.is_some() {
            // At least one side is a directory: diff the file side as absent
            // against the directory side, recursing on the directories.
            self.diff_into(before_tree, after_tree, &format!("{path}/"), prefixes, out)?;
            if before.is_some_and(|(kind, _)| kind != EntryKind::Tree) {
                emit_delete(path, prefixes, out);
            }
            if let Some((kind, id)) = after.filter(|(kind, _)| *kind != EntryKind::Tree) {
                emit_upsert(path, prefixes, out, kind, id)?;
            }
            return Ok(());
        }
        match (before, after) {
            (_, Some((kind, id))) => emit_upsert(path, prefixes, out, kind, id)?,
            (Some(_), None) => emit_delete(path, prefixes, out),
            (None, None) => {}
        }
        Ok(())
    }

    /// A tree's entries as a name-keyed map, or empty when the tree is absent.
    fn entries_by_name(
        &self,
        id: Option<ObjectId>,
    ) -> Result<std::collections::BTreeMap<String, (EntryKind, ObjectId)>> {
        let Some(id) = id else {
            return Ok(std::collections::BTreeMap::new());
        };
        Ok(decode_tree(&self.objects.get(id)?)?
            .into_iter()
            .map(|entry| (entry.name, (entry.kind, entry.id)))
            .collect())
    }
}

/// The id of `entry` when it is a directory.
fn subtree(entry: Option<(EntryKind, ObjectId)>) -> Option<ObjectId> {
    match entry {
        Some((EntryKind::Tree, id)) => Some(id),
        _ => None,
    }
}

/// Whether `path` lies under one of `prefixes`, or `prefixes` is empty.
fn included(path: &str, prefixes: &[&str]) -> bool {
    prefixes.is_empty()
        || prefixes
            .iter()
            .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

fn emit_upsert(
    path: &str,
    prefixes: &[&str],
    out: &mut Vec<Change>,
    kind: EntryKind,
    blob: ObjectId,
) -> Result<()> {
    if included(path, prefixes) {
        out.push(Change::Upsert {
            path: RepoPath::new(path)?,
            kind,
            blob,
        });
    }
    Ok(())
}

fn emit_delete(path: &str, prefixes: &[&str], out: &mut Vec<Change>) {
    if included(path, prefixes)
        && let Ok(path) = RepoPath::new(path)
    {
        out.push(Change::Delete { path });
    }
}

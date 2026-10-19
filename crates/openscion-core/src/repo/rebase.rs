//! Server-side rebase, revert, and landing.
//!
//! [`Repository::rebase`] moves a changeset's changes onto another changeset,
//! detecting conflicts at the file level. [`Repository::revert`] rebases the
//! inverse of a changeset. [`Repository::land`] moves a ref to include a
//! changeset through a compare-and-swap retry loop, rebasing onto the tip when
//! another writer got there first, and never taking a global lock
//! (`docs/specs/openscion-api.md#rebase-revert-and-landing`).
//!
//! Conflict detection compares, for each path a changeset touches, the base it
//! is relative to against the target it is moving onto: a path unchanged
//! between them applies as-is, an identical change on both sides drops, and
//! anything else conflicts. A conflict fails the whole operation and creates
//! nothing.

use openscion_proto::{Change, CreateChangeset, EntryKind, ObjectId, UpdateRef};

use crate::{Error, Result};

use super::Repository;

impl Repository {
    /// Rebases `changeset` onto `onto`: a new changeset with first parent
    /// `onto` and `changeset`'s changes, carrying `rebased-from`. Returns
    /// `onto` unchanged when every change is already present there.
    ///
    /// # Errors
    ///
    /// Returns [`Error::MergeConflict`] listing every conflicting path, having
    /// created nothing, and [`Error::NotFound`] when a changeset is unknown.
    pub fn rebase(&mut self, changeset: ObjectId, onto: ObjectId) -> Result<ObjectId> {
        let base = self.first_parent_tree(changeset)?;
        let changes = self.entry(changeset)?.changeset.changes.clone();
        let kept = self.replay(base, &changes, onto)?;
        if kept.is_empty() {
            return Ok(onto);
        }
        let entry = self.entry(changeset)?;
        let mut request = CreateChangeset::new(
            vec![onto],
            kept,
            entry.changeset.author.clone(),
            entry.changeset.message.clone(),
        );
        request.metadata = entry.changeset.metadata.clone();
        request
            .metadata
            .insert("rebased-from".to_owned(), changeset.to_string());
        Ok(self.create_changeset(request)?.id)
    }

    /// Reverts `changeset` onto `onto`: a new changeset that restores, on top
    /// of `onto`, what `changeset`'s first parent held on every path it
    /// touched, carrying `reverts`. Returns `onto` unchanged when the revert is
    /// already present there.
    ///
    /// # Errors
    ///
    /// Returns [`Error::MergeConflict`] listing every conflicting path, having
    /// created nothing, and [`Error::NotFound`] when a changeset is unknown.
    pub fn revert(
        &mut self,
        changeset: ObjectId,
        onto: ObjectId,
        author: &str,
        message: &str,
    ) -> Result<ObjectId> {
        let inverse = self.inverse_changes(changeset)?;
        // The inverse is relative to `changeset`'s own snapshot, so that is the
        // base the replay compares against.
        let base = Some(self.entry(changeset)?.info.root_tree);
        let kept = self.replay(base, &inverse, onto)?;
        if kept.is_empty() {
            return Ok(onto);
        }
        let mut request = CreateChangeset::new(vec![onto], kept, author, message);
        request
            .metadata
            .insert("reverts".to_owned(), changeset.to_string());
        Ok(self.create_changeset(request)?.id)
    }

    /// Moves `ref_name` to include `changeset`, rebasing onto the tip whenever
    /// another writer moved it first, for up to `max_attempts` tries. Returns
    /// the landed changeset and the ref's new version. The result always has
    /// the prior tip as an ancestor, because it is either the tip's descendant
    /// by rebase or the changeset whose first parent is the tip.
    ///
    /// # Errors
    ///
    /// Returns [`Error::MergeConflict`] when the rebase conflicts (the caller
    /// resolves and retries), [`Error::RefConflict`] when contention outlasts
    /// `max_attempts`, and [`Error::NotFound`] when a changeset is unknown.
    pub fn land(
        &mut self,
        changeset: ObjectId,
        ref_name: &str,
        max_attempts: u32,
    ) -> Result<(ObjectId, u64)> {
        let first_parent = self.entry(changeset)?.changeset.parents.first().copied();
        for _ in 0..max_attempts {
            let tip = self.get_ref(ref_name).ok().map(|value| value.target);
            // Land as-is when the ref is absent or already at the first parent;
            // otherwise rebase onto wherever another writer moved the tip.
            let landed = match tip {
                Some(target) if Some(target) != first_parent => self.rebase(changeset, target)?,
                _ => changeset,
            };
            match self.update_ref(ref_name, UpdateRef::new(tip, landed)) {
                Ok(value) => return Ok((landed, value.version)),
                Err(Error::RefConflict { .. }) => {}
                Err(other) => return Err(other),
            }
        }
        Err(Error::RefConflict {
            name: ref_name.to_owned(),
            current: self.get_ref(ref_name).ok().map(|value| value.target),
        })
    }

    /// The root tree of `changeset`'s first parent, or `None` when it is a
    /// root changeset (its base snapshot is empty).
    fn first_parent_tree(&self, changeset: ObjectId) -> Result<Option<ObjectId>> {
        match self.entry(changeset)?.changeset.parents.first() {
            Some(parent) => Ok(Some(self.entry(*parent)?.info.root_tree)),
            None => Ok(None),
        }
    }

    /// The changes that undo `changeset`: each touched path restored to what
    /// its first parent held (an [`Change::Upsert`], or a [`Change::Delete`]
    /// when the first parent had nothing there).
    fn inverse_changes(&self, changeset: ObjectId) -> Result<Vec<Change>> {
        let base = self.first_parent_tree(changeset)?;
        let entry = self.entry(changeset)?;
        let mut inverse = Vec::with_capacity(entry.changeset.changes.len());
        for change in &entry.changeset.changes {
            let path = change.path().clone();
            let restore = match base {
                Some(tree) => self.resolve(tree, &path)?,
                None => None,
            };
            inverse.push(match restore {
                Some((kind, blob)) => Change::Upsert { path, kind, blob },
                None => Change::Delete { path },
            });
        }
        Ok(inverse)
    }

    /// Which of `changes`, relative to the `base` snapshot they describe, still
    /// apply when moved onto `onto`: a path unchanged between `base` and `onto`
    /// keeps its change, a path already holding the change's result drops it,
    /// and anything else conflicts.
    fn replay(
        &self,
        base: Option<ObjectId>,
        changes: &[Change],
        onto: ObjectId,
    ) -> Result<Vec<Change>> {
        let onto_tree = self.entry(onto)?.info.root_tree;
        let mut kept = Vec::new();
        let mut conflicts = Vec::new();
        for change in changes {
            let path = change.path();
            let base_entry = match base {
                Some(tree) => self.resolve(tree, path)?,
                None => None,
            };
            let onto_entry = self.resolve(onto_tree, path)?;
            if base_entry == onto_entry {
                kept.push(change.clone());
            } else if onto_entry != result_entry(change) {
                conflicts.push(path.as_str().to_owned());
            }
        }
        if !conflicts.is_empty() {
            conflicts.sort();
            conflicts.dedup();
            return Err(Error::MergeConflict { paths: conflicts });
        }
        Ok(kept)
    }
}

/// The entry a change leaves at its path: the upserted file, or nothing for a
/// delete.
fn result_entry(change: &Change) -> Option<(EntryKind, ObjectId)> {
    match change {
        Change::Upsert { kind, blob, .. } => Some((*kind, *blob)),
        Change::Delete { .. } => None,
    }
}

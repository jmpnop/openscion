//! The repository: the object store, the changeset log, and the refs.
//!
//! [`Repository`] is the engine the server wraps. It accepts blobs, validates
//! and appends changesets — deriving each one's root tree before the append is
//! acknowledged — and moves refs by compare-and-swap
//! (`docs/specs/openscion-data-model.md`). It holds its state behind two ports:
//! blobs and trees in an [`ObjectStore`](crate::ObjectStore), and the changeset
//! log and refs in a [`MetadataStore`](crate::MetadataStore). The default
//! stores are in memory; persistent stores plug in behind the same ports
//! through [`Repository::with_stores`].
//!
//! Each appended changeset carries a generation number, which bounds the
//! ancestry walks, and a first-parent skip list, which lets a first-parent
//! ancestry question jump along the history in O(log n) rather than step by
//! step. The read queries ([`query`]) — `stat`, `list_tree`, `read_file`,
//! `diff`, `log`, `is_ancestor`, `merge_base` — resolve paths and walk history
//! on the server without a checkout. Landing ([`rebase`]) — rebase, revert, and
//! `land` with its compare-and-swap retry loop — moves work onto a ref without a
//! global lock.
//!
//! Not here yet: line-level merge.

mod derive;
mod query;
mod rebase;

use std::sync::Arc;

use openscion_proto::{
    Change, Changeset, ChangesetInfo, ChangesetRecord, CreateChangeset, EntryKind, ObjectId,
    ObjectKind, RefValue, RepoPath, TreeEntry, UpdateRef,
};

use crate::{
    Ancestry, Clock, Error, MemoryMetadataStore, MemoryObjectStore, MetadataStore, NewChangeset,
    ObjectStore, Result, StoredChangeset, SystemClock, decode_tree, encode_changeset, object_id,
};

/// The most changes one changeset may carry.
pub const MAX_CHANGES: usize = 100_000;

/// The longest an author, message, or metadata string may be, in bytes.
pub const MAX_TEXT: usize = 1 << 20;

/// An OpenScion repository over an object store and a metadata store.
///
/// # Examples
///
/// ```
/// use openscion_core::{
///     Change, CreateChangeset, EntryKind, FixedClock, Repository, RepoPath, UpdateRef,
/// };
///
/// let mut repo = Repository::with_clock(FixedClock(0));
/// let blob = repo.put_blob(b"fn main() {}\n")?;
/// let change = Change::Upsert { path: RepoPath::new("src/main.rs")?, kind: EntryKind::File, blob };
/// let root = repo.create_changeset(CreateChangeset::new(vec![], vec![change], "agent", "init"))?;
/// let main = repo.update_ref("main", UpdateRef::new(None, root.id))?;
/// assert_eq!(main.version, 1);
///
/// let top = repo.tree(root.root_tree)?;
/// assert_eq!(top[0].name, "src");
/// # Ok::<(), openscion_core::Error>(())
/// ```
#[derive(Debug)]
pub struct Repository {
    objects: Box<dyn ObjectStore>,
    meta: Box<dyn MetadataStore>,
    clock: Box<dyn Clock>,
}

impl Default for Repository {
    fn default() -> Self {
        Self::new()
    }
}

impl Repository {
    /// An empty in-memory repository on the system clock.
    #[must_use]
    pub fn new() -> Self {
        Self::with_clock(SystemClock)
    }

    /// An empty in-memory repository reading time from `clock`.
    #[must_use]
    pub fn with_clock(clock: impl Clock + 'static) -> Self {
        Self::with_stores(
            Box::new(MemoryObjectStore::new()),
            Box::new(MemoryMetadataStore::new()),
            Box::new(clock),
        )
    }

    /// A repository over the given stores and clock. The stores may be empty or
    /// may already hold a repository's history; the engine does not care which.
    #[must_use]
    pub fn with_stores(
        objects: Box<dyn ObjectStore>,
        meta: Box<dyn MetadataStore>,
        clock: Box<dyn Clock>,
    ) -> Self {
        Self {
            objects,
            meta,
            clock,
        }
    }

    /// Stores a blob and returns its id.
    ///
    /// # Errors
    ///
    /// Returns a store error when the blob cannot be persisted.
    pub fn put_blob(&mut self, bytes: &[u8]) -> Result<ObjectId> {
        self.objects.put(ObjectKind::Blob, bytes)
    }

    /// Stores a blob a client uploaded under `id`, after checking the id.
    ///
    /// # Errors
    ///
    /// Returns [`Error::CorruptObject`] when `bytes` do not hash to `id`.
    pub fn put_blob_verified(&mut self, id: ObjectId, bytes: &[u8]) -> Result<()> {
        self.objects.put_verified(ObjectKind::Blob, id, bytes)
    }

    /// The bytes of a blob or tree.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] when no object has this id.
    pub fn object(&self, id: ObjectId) -> Result<Arc<[u8]>> {
        self.objects.get(id)
    }

    /// A tree's entries.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] when no object has this id, and
    /// [`Error::Malformed`] when the object is not a tree.
    pub fn tree(&self, id: ObjectId) -> Result<Vec<TreeEntry>> {
        decode_tree(&self.objects.get(id)?)
    }

    /// The number of changesets in the log.
    ///
    /// # Errors
    ///
    /// Returns a store error when the count cannot be read.
    pub fn log_len(&self) -> Result<u64> {
        self.meta.log_len()
    }

    /// Validates a changeset, derives its root tree, and appends it to the log.
    ///
    /// Submitting a request identical to an existing changeset, including its
    /// creation time, returns the existing one.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] for a missing parent or blob, and
    /// [`Error::InvalidChange`] for any other broken validity rule from
    /// `docs/specs/openscion-data-model.md#validity`.
    pub fn create_changeset(&mut self, request: CreateChangeset) -> Result<ChangesetInfo> {
        let changeset = Changeset::from_request(request, self.clock.now_millis());
        self.validate(&changeset)?;
        let id = object_id(ObjectKind::Changeset, &encode_changeset(&changeset));
        if let Some(info) = self.meta.info(id)? {
            return Ok(info);
        }
        let base = match changeset.parents.first() {
            Some(parent) => self.info_of(*parent)?.root_tree,
            None => self.objects.put(ObjectKind::Tree, &[])?,
        };
        let root_tree = derive::derive_root_tree(self.objects.as_mut(), base, &changeset.changes)?;
        let generation = self.next_generation(&changeset.parents)?;
        let skips = self.next_skips(&changeset.parents)?;
        self.meta.append(NewChangeset {
            id,
            root_tree,
            changeset,
            generation,
            skips,
        })
    }

    /// The generation a new changeset with these parents gets: `1` for a root,
    /// else `1 + max(parent generations)`. Parents were validated to exist.
    fn next_generation(&self, parents: &[ObjectId]) -> Result<u64> {
        let mut max = 0;
        for parent in parents {
            max = max.max(self.ancestry_of(*parent)?.generation);
        }
        Ok(max + 1)
    }

    /// The first-parent skip list a new changeset with these parents gets:
    /// index `k` is the first-parent ancestor `2^k` changesets back. Empty for a
    /// root. Built by binary lifting off the first parent's own skip list, so it
    /// costs O(log n) lookups, not the depth of history. Parents were validated
    /// to exist.
    fn next_skips(&self, parents: &[ObjectId]) -> Result<Vec<ObjectId>> {
        let Some(&first) = parents.first() else {
            return Ok(Vec::new());
        };
        // `skips[0]` is the first parent; `skips[k]` is `2^(k-1)` steps back
        // from the node `2^(k-1)` steps back, i.e. `2^k` steps back. The chain
        // stops once the node `2^k` back has no `2^k` ancestor of its own.
        let mut skips = vec![first];
        loop {
            let k = skips.len() - 1;
            match self.first_parent_skips_of(skips[k])?.get(k) {
                Some(&next) => skips.push(next),
                None => break,
            }
        }
        Ok(skips)
    }

    fn validate(&self, changeset: &Changeset) -> Result<()> {
        if changeset.parents.is_empty() && self.meta.log_len()? != 0 {
            return Err(invalid("", "only the first changeset may have no parents"));
        }
        for parent in &changeset.parents {
            if self.meta.info(*parent)?.is_none() {
                return Err(Error::NotFound(*parent));
            }
        }
        if changeset.changes.is_empty() && changeset.parents.len() < 2 {
            return Err(invalid("", "only a merge may have no changes"));
        }
        if changeset.changes.len() > MAX_CHANGES {
            return Err(invalid("", "too many changes"));
        }
        let texts = [&changeset.author, &changeset.message];
        let metadata = changeset
            .metadata
            .iter()
            .flat_map(|(key, value)| [key, value]);
        if changeset.metadata.len() > MAX_CHANGES
            || texts
                .into_iter()
                .chain(metadata)
                .any(|text| text.len() > MAX_TEXT)
        {
            return Err(invalid("", "text or metadata too large"));
        }
        for pair in changeset.changes.windows(2) {
            if pair[0].path() == pair[1].path() {
                return Err(invalid(pair[0].path().as_str(), "path changed twice"));
            }
        }
        for change in &changeset.changes {
            if let Change::Upsert { path, kind, blob } = change {
                if *kind == EntryKind::Tree {
                    return Err(invalid(path.as_str(), "upsert of a tree"));
                }
                if !self.objects.contains(*blob)? {
                    return Err(Error::NotFound(*blob));
                }
            }
        }
        Ok(())
    }

    /// The summary of the changeset with `id`.
    pub(super) fn info_of(&self, id: ObjectId) -> Result<ChangesetInfo> {
        self.meta.info(id)?.ok_or(Error::NotFound(id))
    }

    /// The generation and parents of the changeset with `id`.
    pub(super) fn ancestry_of(&self, id: ObjectId) -> Result<Ancestry> {
        self.meta.ancestry(id)?.ok_or(Error::NotFound(id))
    }

    /// The first-parent skip list of the changeset with `id`.
    pub(super) fn first_parent_skips_of(&self, id: ObjectId) -> Result<Vec<ObjectId>> {
        self.meta.first_parent_skips(id)?.ok_or(Error::NotFound(id))
    }

    /// The whole changeset with `id`.
    pub(super) fn record(&self, id: ObjectId) -> Result<StoredChangeset> {
        self.meta.record(id)?.ok_or(Error::NotFound(id))
    }

    /// A changeset from the log, with its summary.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] when no changeset has this id.
    pub fn changeset(&self, id: ObjectId) -> Result<ChangesetRecord> {
        let record = self.record(id)?;
        Ok(ChangesetRecord::new(record.info, record.changeset))
    }

    /// A changeset's generation: the longest path in changesets from a root to
    /// it, `1` for a root. A descendant always has a strictly greater
    /// generation, which is what bounds ancestry searches.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] when no changeset has this id.
    pub fn generation(&self, id: ObjectId) -> Result<u64> {
        Ok(self.ancestry_of(id)?.generation)
    }

    /// A changeset's first-parent skip list: index `k` is the first-parent
    /// ancestor `2^k` changesets back, so [`is_first_parent_ancestor`] reaches
    /// any first-parent ancestor in O(log n). Empty for a root.
    ///
    /// [`is_first_parent_ancestor`]: Repository::is_first_parent_ancestor
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] when no changeset has this id.
    pub fn first_parent_skips(&self, id: ObjectId) -> Result<Vec<ObjectId>> {
        self.first_parent_skips_of(id)
    }

    /// A ref's current value.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidPath`] for an invalid name and
    /// [`Error::NoSuchRef`] when the ref does not exist.
    pub fn get_ref(&self, name: &str) -> Result<RefValue> {
        RepoPath::new(name)?;
        self.meta
            .get_ref(name)?
            .ok_or_else(|| Error::NoSuchRef(name.to_owned()))
    }

    /// Moves a ref by compare-and-swap.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidPath`] for an invalid name,
    /// [`Error::NotFound`] when the new target is not in the log, and
    /// [`Error::RefConflict`] when the ref does not hold `update.expected`;
    /// nothing changes on any error.
    pub fn update_ref(&mut self, name: &str, update: UpdateRef) -> Result<RefValue> {
        RepoPath::new(name)?;
        self.info_of(update.new)?;
        self.meta.update_ref(name, update.expected, update.new)
    }
}

fn invalid(path: &str, rule: &'static str) -> Error {
    Error::InvalidChange {
        path: path.to_owned(),
        rule,
    }
}

#[cfg(test)]
mod test;

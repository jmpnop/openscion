//! The repository: the object store, the changeset log, and the refs.
//!
//! [`Repository`] is the engine the server wraps. It accepts blobs, validates
//! and appends changesets — deriving each one's root tree before the append is
//! acknowledged — and moves refs by compare-and-swap
//! (`docs/specs/openscion-data-model.md`). Everything is in memory; there is no
//! persistence yet.
//!
//! Each appended changeset carries a generation number, which bounds the
//! ancestry walks. The read queries ([`query`]) — `stat`, `list_tree`,
//! `read_file`, `diff`, `log`, `is_ancestor`, `merge_base` — resolve paths and
//! walk history on the server without a checkout. Landing ([`rebase`]) — rebase,
//! revert, and `land` with its compare-and-swap retry loop — moves work onto a
//! ref without a global lock.
//!
//! Not here yet: persistent stores, first-parent skip lists (the generation
//! numbers already bound the walks), and line-level merge.

mod derive;
mod query;
mod rebase;
mod types;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use openscion_proto::{
    Change, Changeset, ChangesetInfo, ChangesetRecord, CreateChangeset, EntryKind, ObjectId,
    ObjectKind, RefValue, RepoPath, TreeEntry, UpdateRef,
};

use crate::{
    Clock, Error, MemoryObjectStore, Result, SystemClock, decode_tree, encode_changeset, object_id,
};
use types::LogEntry;

/// The most changes one changeset may carry.
pub const MAX_CHANGES: usize = 100_000;

/// The longest an author, message, or metadata string may be, in bytes.
pub const MAX_TEXT: usize = 1 << 20;

/// An in-memory OpenScion repository.
///
/// # Examples
///
/// ```
/// use openscion_core::{
///     Change, CreateChangeset, EntryKind, FixedClock, Repository, RepoPath, UpdateRef,
/// };
///
/// let mut repo = Repository::with_clock(FixedClock(0));
/// let blob = repo.put_blob(b"fn main() {}\n");
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
    objects: MemoryObjectStore,
    log: Vec<LogEntry>,
    index: HashMap<ObjectId, usize>,
    refs: BTreeMap<String, RefValue>,
    clock: Box<dyn Clock>,
}

impl Default for Repository {
    fn default() -> Self {
        Self::new()
    }
}

impl Repository {
    /// An empty repository on the system clock.
    #[must_use]
    pub fn new() -> Self {
        Self::with_clock(SystemClock)
    }

    /// An empty repository reading time from `clock`.
    #[must_use]
    pub fn with_clock(clock: impl Clock + 'static) -> Self {
        Self {
            objects: MemoryObjectStore::new(),
            log: Vec::new(),
            index: HashMap::new(),
            refs: BTreeMap::new(),
            clock: Box::new(clock),
        }
    }

    /// Stores a blob and returns its id.
    pub fn put_blob(&mut self, bytes: &[u8]) -> ObjectId {
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

    /// The object store, for size and count statistics.
    #[must_use]
    pub fn objects(&self) -> &MemoryObjectStore {
        &self.objects
    }

    /// The number of changesets in the log.
    #[must_use]
    pub fn log_len(&self) -> usize {
        self.log.len()
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
        if let Some(&position) = self.index.get(&id) {
            return Ok(self.log[position].info);
        }
        let base = match changeset.parents.first() {
            Some(parent) => self.entry(*parent)?.info.root_tree,
            None => self.objects.put(ObjectKind::Tree, &[]),
        };
        let root_tree = derive::derive_root_tree(&mut self.objects, base, &changeset.changes)?;
        let info = ChangesetInfo::new(id, self.log.len() as u64 + 1, root_tree);
        // Parents were validated to be in the log, so `entry` cannot fail here.
        let generation = changeset
            .parents
            .iter()
            .map(|parent| self.entry(*parent).map(|e| e.generation))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .max()
            .unwrap_or(0)
            + 1;
        self.index.insert(id, self.log.len());
        self.log.push(LogEntry {
            info,
            changeset,
            generation,
        });
        Ok(info)
    }

    fn validate(&self, changeset: &Changeset) -> Result<()> {
        if changeset.parents.is_empty() && !self.log.is_empty() {
            return Err(invalid("", "only the first changeset may have no parents"));
        }
        for parent in &changeset.parents {
            self.entry(*parent)?;
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
                if !self.objects.contains(*blob) {
                    return Err(Error::NotFound(*blob));
                }
            }
        }
        Ok(())
    }

    fn entry(&self, id: ObjectId) -> Result<&LogEntry> {
        self.index
            .get(&id)
            .map(|&position| &self.log[position])
            .ok_or(Error::NotFound(id))
    }

    /// A changeset from the log, with its summary.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] when no changeset has this id.
    pub fn changeset(&self, id: ObjectId) -> Result<ChangesetRecord> {
        let entry = self.entry(id)?;
        Ok(ChangesetRecord::new(entry.info, entry.changeset.clone()))
    }

    /// A changeset's generation: the longest path in changesets from a root to
    /// it, `1` for a root. A descendant always has a strictly greater
    /// generation, which is what bounds ancestry searches.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`] when no changeset has this id.
    pub fn generation(&self, id: ObjectId) -> Result<u64> {
        Ok(self.entry(id)?.generation)
    }

    /// A ref's current value.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidPath`] for an invalid name and
    /// [`Error::NoSuchRef`] when the ref does not exist.
    pub fn get_ref(&self, name: &str) -> Result<RefValue> {
        RepoPath::new(name)?;
        self.refs
            .get(name)
            .cloned()
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
        self.entry(update.new)?;
        let current = self.refs.get(name);
        if current.map(|value| value.target) != update.expected {
            return Err(Error::RefConflict {
                name: name.to_owned(),
                current: current.map(|value| value.target),
            });
        }
        let version = current.map_or(1, |value| value.version + 1);
        let value = RefValue::new(name, update.new, version);
        self.refs.insert(name.to_owned(), value.clone());
        Ok(value)
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

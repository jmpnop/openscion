//! The in-memory metadata store.

use std::collections::{BTreeMap, HashMap};

use openscion_proto::{ChangesetInfo, ObjectId, RefValue};

use crate::{Error, Result};

use super::{Ancestry, MetadataStore, NewChangeset, StoredChangeset};

/// The changeset log and the refs, in memory.
#[derive(Debug, Default)]
pub struct MemoryMetadataStore {
    log: Vec<StoredChangeset>,
    index: HashMap<ObjectId, usize>,
    refs: BTreeMap<String, RefValue>,
}

impl MemoryMetadataStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn entry(&self, id: ObjectId) -> Option<&StoredChangeset> {
        self.index.get(&id).map(|&position| &self.log[position])
    }
}

impl MetadataStore for MemoryMetadataStore {
    fn append(&mut self, new: NewChangeset) -> Result<ChangesetInfo> {
        let lsn = self.log.len() as u64 + 1;
        let info = ChangesetInfo::new(new.id, lsn, new.root_tree);
        self.index.insert(new.id, self.log.len());
        self.log.push(StoredChangeset {
            info,
            changeset: new.changeset,
            generation: new.generation,
            skips: new.skips,
        });
        Ok(info)
    }

    fn info(&self, id: ObjectId) -> Result<Option<ChangesetInfo>> {
        Ok(self.entry(id).map(|entry| entry.info))
    }

    fn ancestry(&self, id: ObjectId) -> Result<Option<Ancestry>> {
        Ok(self.entry(id).map(|entry| Ancestry {
            generation: entry.generation,
            parents: entry.changeset.parents.clone(),
        }))
    }

    fn first_parent_skips(&self, id: ObjectId) -> Result<Option<Vec<ObjectId>>> {
        Ok(self.entry(id).map(|entry| entry.skips.clone()))
    }

    fn record(&self, id: ObjectId) -> Result<Option<StoredChangeset>> {
        Ok(self.entry(id).cloned())
    }

    fn log_len(&self) -> Result<u64> {
        Ok(self.log.len() as u64)
    }

    fn get_ref(&self, name: &str) -> Result<Option<RefValue>> {
        Ok(self.refs.get(name).cloned())
    }

    fn update_ref(
        &mut self,
        name: &str,
        expected: Option<ObjectId>,
        new: ObjectId,
    ) -> Result<RefValue> {
        let current = self.refs.get(name);
        if current.map(|value| value.target) != expected {
            return Err(Error::RefConflict {
                name: name.to_owned(),
                current: current.map(|value| value.target),
            });
        }
        let version = current.map_or(1, |value| value.version + 1);
        let value = RefValue::new(name, new, version);
        self.refs.insert(name.to_owned(), value.clone());
        Ok(value)
    }
}

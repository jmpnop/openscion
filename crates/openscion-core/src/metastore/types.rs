//! The metadata store's value types.

use openscion_proto::{Changeset, ChangesetInfo, ObjectId};

/// A validated changeset about to be appended to the log. The engine has
/// already computed its id, root tree, generation, and first-parent skip list;
/// the store assigns the log sequence number.
#[derive(Debug, Clone)]
pub struct NewChangeset {
    /// The changeset's content id.
    pub id: ObjectId,
    /// The root tree of its snapshot, already derived into the object store.
    pub root_tree: ObjectId,
    /// The changeset itself.
    pub changeset: Changeset,
    /// Its generation: `1` for a root, else `1 + max(parent generations)`.
    pub generation: u64,
    /// Its first-parent skip list: index `k` is the first-parent ancestor `2^k`
    /// changesets back. Empty for a root. See [`Ancestry`].
    pub skips: Vec<ObjectId>,
}

/// A changeset read back from the log, with its summary and generation.
#[derive(Debug, Clone)]
pub struct StoredChangeset {
    /// Id, log sequence number, and root tree.
    pub info: ChangesetInfo,
    /// The changeset itself.
    pub changeset: Changeset,
    /// Its generation.
    pub generation: u64,
    /// Its first-parent skip list; see [`NewChangeset::skips`].
    pub skips: Vec<ObjectId>,
}

/// The ancestry facts an ancestry walk needs about one changeset, fetched in a
/// single lookup so the walks never load the whole changeset.
#[derive(Debug, Clone)]
pub struct Ancestry {
    /// Longest path, in changesets, from a root to this one: `1` for a root,
    /// else `1 + max(parent generations)`. A changeset with a smaller
    /// generation than another can never be its descendant, so ancestry walks
    /// never descend below the target's generation.
    pub generation: u64,
    /// Its parents, first parent first.
    pub parents: Vec<ObjectId>,
}

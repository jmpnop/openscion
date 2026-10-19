//! The log entry type.

use openscion_proto::{Changeset, ChangesetInfo};

/// One appended changeset and its summary.
#[derive(Debug, Clone)]
pub(super) struct LogEntry {
    pub(super) info: ChangesetInfo,
    pub(super) changeset: Changeset,
    /// Longest path, in changesets, from a root to this one: `1` for a root,
    /// else `1 + max(parent generations)`. Set once at append. A changeset
    /// with a smaller generation than another can never be its descendant, so
    /// ancestry walks never descend below the target's generation.
    pub(super) generation: u64,
}

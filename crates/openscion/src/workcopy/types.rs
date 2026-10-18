//! The working-copy state types.

use std::collections::BTreeMap;

use openscion_core::{EntryKind, ObjectId};
use serde::{Deserialize, Serialize};

/// What `opnscn` remembers about a working copy, in `.opnscn/state.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct State {
    /// The server the working copy came from.
    pub(crate) server: String,
    /// The ref commits move.
    pub(crate) ref_name: String,
    /// The changeset the files on disk were last synced with.
    pub(crate) base: ObjectId,
    /// The directory a sparse clone was limited to, if any.
    pub(crate) sparse: Option<String>,
    /// When the state was written, in nanoseconds since the Unix epoch. A
    /// file modified at or after this instant cannot be trusted from its stat
    /// data alone, the same "racily clean" rule git applies to its index.
    pub(crate) written_at: u64,
    /// Every tracked file, by path.
    pub(crate) files: BTreeMap<String, FileState>,
}

/// One tracked file: its content id and the stat data that lets a scan skip
/// re-hashing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FileState {
    /// File, executable, or symlink.
    pub(crate) kind: EntryKind,
    /// The blob holding its contents.
    pub(crate) blob: ObjectId,
    /// Its size in bytes when last hashed.
    pub(crate) size: u64,
    /// Its modification time when last hashed, in nanoseconds since the epoch.
    pub(crate) mtime: u64,
    /// Its inode change time when last hashed, in nanoseconds since the epoch,
    /// or `0` where the platform has none. Unlike `mtime`, a content write
    /// always advances it and user space cannot restore it, so it closes the
    /// gap where a same-size edit with a reset `mtime` reads as unchanged. A
    /// state written before this field existed deserializes it as `0`, which
    /// is never trusted, so those files are re-hashed once.
    #[serde(default)]
    pub(crate) ctime: u64,
}

/// A file found on disk by a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Scanned {
    /// The path relative to the working-copy root, `/`-separated.
    pub(crate) path: String,
    /// File, executable, or symlink.
    pub(crate) kind: EntryKind,
    /// Its size in bytes.
    pub(crate) size: u64,
    /// Its modification time, in nanoseconds since the epoch.
    pub(crate) mtime: u64,
    /// Its inode change time, in nanoseconds since the epoch, or `0` where the
    /// platform has none.
    pub(crate) ctime: u64,
}

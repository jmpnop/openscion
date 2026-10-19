//! The OpenScion engine: a server-centric, content-addressed source control
//! system for monorepos whose clients are mostly always-online agents and jobs.
//!
//! The design is specified in `docs/specs/`: the architecture, the data model
//! (objects, changesets, the log, refs, derived data), and the API. This crate
//! is the engine those specs describe, as a plain library usable in-process
//! with no transport. Today it holds:
//!
//! - [`object_id`] — BLAKE3 ids over canonical encodings.
//! - [`encode_tree`], [`decode_tree`], [`encode_changeset`] — the canonical
//!   encodings, and [`FrameWriter`] / [`decode_frames`] for batched transfer.
//! - [`ObjectStore`] — the content-addressed blob and tree port, with the
//!   in-memory [`MemoryObjectStore`] and the on-disk [`FilesystemObjectStore`].
//! - [`MetadataStore`] — the changeset-log and ref port, with the in-memory
//!   [`MemoryMetadataStore`].
//! - [`Repository`] — validation, root-tree derivation by path copying, the
//!   append-only log, compare-and-swap refs, generation numbers, the snapshot
//!   and history queries, and server-side rebase, revert, and landing, over the
//!   two stores.
//! - [`Clock`] — the time port, with [`SystemClock`] and [`FixedClock`].
//!
//! # What is deliberately not here
//!
//! The SQLite metadata store, the network tiers, git ingestion, and
//! line-level blame and merge arrive in later phases. There is no transport:
//! `openscion-server` puts this engine behind HTTP.
//!
//! # Layout
//!
//! This is the engine half of a two-crate pair:
//!
//! - [`openscion_proto`] — the protocol. Payload types and the protocol
//!   version, with no transport and no behavior. A client that only makes calls
//!   depends on that crate alone.
//! - `openscion-core` — this crate. The behavior and the crate-wide error type.
//!
//! Every public item is re-exported from here — including all of
//! [`openscion_proto`] — so `openscion_core::ObjectId` is the *same type* as
//! `openscion_proto::ObjectId`.
//!
//! # Example
//!
//! ```
//! use openscion_core::{Change, CreateChangeset, EntryKind, Repository, RepoPath, UpdateRef};
//!
//! let mut repo = Repository::new();
//! let blob = repo.put_blob(b"hello\n")?;
//! let path = RepoPath::new("docs/hello.txt")?;
//! let first = repo.create_changeset(CreateChangeset::new(
//!     vec![],
//!     vec![Change::Upsert { path: path.clone(), kind: EntryKind::File, blob }],
//!     "agent",
//!     "add hello",
//! ))?;
//! let second = repo.create_changeset(CreateChangeset::new(
//!     vec![first.id],
//!     vec![Change::Delete { path }],
//!     "agent",
//!     "remove hello",
//! ))?;
//! repo.update_ref("main", UpdateRef::new(None, second.id))?;
//! assert!(repo.tree(second.root_tree)?.is_empty());
//! # Ok::<(), openscion_core::Error>(())
//! ```

mod clock;
mod encoding;
mod error;
mod hash;
mod metastore;
mod repo;
mod store;

pub use clock::{Clock, FixedClock, SystemClock};
pub use encoding::{FrameWriter, decode_frames, decode_tree, encode_changeset, encode_tree};
pub use error::{Error, Result};
pub use hash::object_id;
pub use metastore::{Ancestry, MemoryMetadataStore, MetadataStore, NewChangeset, StoredChangeset};
pub use repo::{MAX_CHANGES, MAX_TEXT, Repository};
pub use store::{FilesystemObjectStore, MemoryObjectStore, ObjectStore};

// The protocol, re-exported by module rather than by item so every path
// through this crate resolves to the same definitions the protocol crate
// publishes. A client may depend on `openscion-proto` directly and get exactly
// these types; nothing here redefines them.
pub use openscion_proto;
pub use openscion_proto::{
    AncestorQuery, AncestorResult, Change, Changeset, ChangesetInfo, ChangesetRecord,
    ChangesetSummary, CreateChangeset, DiffRequest, DiffResult, Entry, EntryKind, ErrorBody,
    FetchObjects, LandRequest, LandResult, LogPage, LogRequest, MergeBaseQuery, MergeBaseResult,
    ObjectId, ObjectKind, PROTOCOL_VERSION, ParseIdError, PathError, RebaseRequest, RebaseResult,
    RefValue, RepoPath, RevertRequest, ServerInfo, StatResult, StoredObjects, TreeEntry,
    TreeListing, UpdateRef, is_compatible, server, version,
};

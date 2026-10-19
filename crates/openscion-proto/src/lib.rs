//! The OpenScion protocol: every payload type that crosses a process boundary.
//!
//! OpenScion is a server-centric source control system: the server owns history
//! and computes commits, rebases, and queries, while clients fetch exactly the
//! objects they need. Every component — the engine, the node daemon, the
//! regional cache, the server, and the `opnscn` client — speaks the same API. This
//! crate is that API's vocabulary, so a tool that only makes calls depends on
//! it alone. See `docs/specs/openscion-api.md` for the operations it carries.
//!
//! # What is here
//!
//! - [`object`] — [`ObjectId`], the content-derived name of every object.
//! - [`path`] — [`RepoPath`], a validated path relative to the repository root.
//! - [`tree`] — [`TreeEntry`] and [`EntryKind`], what one directory holds.
//! - [`changeset`] — [`CreateChangeset`], [`Changeset`], and their summaries.
//! - [`refs`] — [`RefValue`] and the compare-and-swap [`UpdateRef`].
//! - [`query`] — snapshot reads and history questions: [`StatResult`],
//!   [`TreeListing`], [`DiffRequest`], [`LogRequest`], and their answers.
//! - [`landing`] — [`RebaseRequest`], [`RevertRequest`], [`LandRequest`], and
//!   their results.
//! - [`transfer`] — the JSON halves of batched object upload and fetch.
//! - [`failure`] — [`ErrorBody`], what every failed request carries.
//! - [`server`] — [`ServerInfo`], what a server reports about itself.
//! - [`version`] — [`PROTOCOL_VERSION`] and the [`is_compatible`] bind rule.
//!
//! # What is deliberately not here
//!
//! **No behavior.** Hashing, validation, and every operation live in
//! `crates/openscion-core`, which depends on this crate and re-exports it. A payload
//! type describes what a request carries, not what the server does with it.
//!
//! **No transport.** This crate holds no connection, client, or codec, and
//! depends on no async runtime, HTTP client, or native library. CI asserts it.
//! Whatever transport carries these payloads — a local socket to the daemon or
//! a network hop between tiers — it carries the same types.
//!
//! # This crate sits underneath the engine, not beside it
//!
//! `openscion-core` **depends on this crate and re-exports all of it**, so
//! `openscion_core::ObjectId` and `openscion_proto::object::ObjectId` are
//! the *same type*, not structural twins. A parallel set of payload types for
//! clients would mean a conversion at every call site that nothing checks.
//!
//! # Example
//!
//! ```
//! use openscion_proto::{CreateChangeset, PROTOCOL_VERSION, RefValue, is_compatible};
//!
//! let body: CreateChangeset = serde_json::from_value(serde_json::json!({
//!     "parents": [], "changes": [], "author": "agent", "message": "init"
//! }))?;
//! assert_eq!(body.author, "agent");
//! assert!(is_compatible(PROTOCOL_VERSION));
//!
//! let main: RefValue = serde_json::from_value(serde_json::json!({
//!     "name": "main", "target": "ab".repeat(32), "version": 1
//! }))?;
//! assert_eq!(main.version, 1);
//! # Ok::<(), serde_json::Error>(())
//! ```

pub mod changeset;
pub mod failure;
pub mod landing;
pub mod object;
pub mod path;
pub mod query;
pub mod refs;
pub mod server;
pub mod transfer;
pub mod tree;
pub mod version;

pub use changeset::{
    Change, Changeset, ChangesetInfo, ChangesetRecord, ChangesetSummary, CreateChangeset,
};
pub use failure::ErrorBody;
pub use landing::{LandRequest, LandResult, RebaseRequest, RebaseResult, RevertRequest};
pub use object::{ObjectId, ObjectKind, ParseIdError};
pub use path::{PathError, RepoPath};
pub use query::{
    AncestorQuery, AncestorResult, DiffRequest, DiffResult, Entry, LogPage, LogRequest,
    MergeBaseQuery, MergeBaseResult, StatResult, TreeListing,
};
pub use refs::{RefValue, UpdateRef};
pub use server::ServerInfo;
pub use transfer::{FetchObjects, StoredObjects};
pub use tree::{EntryKind, TreeEntry};
pub use version::{PROTOCOL_VERSION, is_compatible};

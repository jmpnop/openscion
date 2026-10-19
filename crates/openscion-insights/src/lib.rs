//! An optional analytics plane over OpenScion history, backed by DuckDB.
//!
//! This crate is **not** part of the serving path and backs neither storage
//! port. It ingests an append-only export of changeset records — the same
//! [`ChangesetRecord`](openscion_proto::ChangesetRecord)s the log holds — into
//! an embedded columnar database and
//! answers analytical questions across history: which paths change most, who
//! changes them, and which paths tend to change together. Serving queries like
//! `Log` and `Diff` stay in the engine; this is for cross-history aggregation
//! that a columnar engine does well and a graph walk does not. See
//! `docs/adr/0002-duckdb-analytics-plane.md`.
//!
//! The analytics are eventually consistent with the log by design: an [`Insights`]
//! reads a snapshot it was given, not the live authoritative state.
//!
//! # Example
//!
//! ```
//! use openscion_insights::Insights;
//! use openscion_proto::{
//!     Change, Changeset, ChangesetInfo, ChangesetRecord, EntryKind, ObjectId, RepoPath,
//! };
//!
//! let id = ObjectId::from_bytes([1; 32]);
//! let blob = ObjectId::from_bytes([2; 32]);
//! let record = ChangesetRecord::new(
//!     ChangesetInfo::new(id, 1, ObjectId::from_bytes([3; 32])),
//!     Changeset::from_request(
//!         openscion_proto::CreateChangeset::new(
//!             vec![],
//!             vec![Change::Upsert { path: RepoPath::new("src/main.rs")?, kind: EntryKind::File, blob }],
//!             "agent",
//!             "init",
//!         ),
//!         0,
//!     ),
//! );
//!
//! let mut insights = Insights::in_memory()?;
//! insights.ingest(&[record])?;
//! assert_eq!(insights.hotspots(10)?[0].path, "src/main.rs");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod analytics;
mod error;

pub use analytics::{AuthorActivity, Insights, PathChurn};
pub use error::{Error, Result};

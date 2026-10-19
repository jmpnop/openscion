//! The OpenScion node daemon: a per-machine, bounded, read-through object store.
//!
//! One daemon runs per machine and holds a node-wide store of immutable,
//! content-addressed objects — blobs, trees, and changesets — shared by every
//! job on that node. It answers the same object reads as the regional cache and
//! the server, so a job talks to whichever tier is nearest without knowing the
//! topology.
//!
//! The store is *bounded*: it keeps the recently used objects within a byte
//! budget and lets the rest fall back to the tier above. [`NodeStore::get`]
//! serves a local copy or, on a miss, fetches the object from an
//! [`ObjectSource`] (the tier above), verifies it against its id, caches it, and
//! returns it. [`NodeStore::snapshot`] and [`NodeStore::warm`] let a fresh
//! daemon start warm from a saved set instead of cold.
//!
//! This crate deliberately does **not** hold the authoritative log, refs, or
//! derived data, and does not answer mutable queries — those are the server's,
//! forwarded upstream unchanged by the daemon's RPC layer (a later piece). It
//! also does not yet speak a wire protocol: the tier above is the in-process
//! [`ObjectSource`] trait, and the framed transport that implements it over a
//! socket arrives with the transport sub-chunk (see `docs/adr/0004`).
//!
//! # Example
//!
//! ```
//! use std::sync::Arc;
//! use openscion_core::ObjectId;
//! use openscion_daemon::{NodeStore, ObjectSource, Result};
//!
//! // A tier above that holds nothing.
//! struct Empty;
//! impl ObjectSource for Empty {
//!     fn fetch(&self, _: ObjectId) -> Result<Option<Arc<[u8]>>> {
//!         Ok(None)
//!     }
//! }
//!
//! let dir = std::env::temp_dir().join("openscion-daemon-doctest");
//! let store = NodeStore::open(&dir, Empty, 1 << 20)?;
//! assert!(store.snapshot().is_empty());
//! # Ok::<(), openscion_daemon::Error>(())
//! ```

mod cache;
mod error;
mod store;
#[cfg(test)]
mod test_support;

pub use error::{Error, Result};
pub use store::{NodeStore, ObjectSource, Warmed};

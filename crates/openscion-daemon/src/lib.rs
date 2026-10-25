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
//! forwarded upstream unchanged by the daemon's RPC layer (a later piece).
//!
//! Tiering is strict and bounded: the store's fetches to the tier above pass
//! through a [`BoundedSource`], which caps how many run at once and sheds the
//! overflow with [`Error::Unavailable`] rather than queueing. The shed crosses
//! the wire as a distinct response, so a daemon reading through a saturated
//! daemon receives `Unavailable` and re-sheds instead of stampeding it.
//!
//! The [`transport`](serve_connection) carries the object RPCs over a byte
//! stream — a Unix socket locally, TCP between machines ([ADR
//! 0004](../../../docs/adr/0004-daemon-transport.md)). [`serve_tcp`] and
//! [`serve_unix`] serve a [`NodeStore`] to connecting clients, and
//! [`SocketSource`] is an [`ObjectSource`] that dials a daemon, so a node store
//! on one tier uses the tier above over the same wire.
//!
//! The `openscion-daemon` binary wires these together into a process:
//! [`DaemonConfig`] reads flags and `OPENSCION_DAEMON_*` environment variables,
//! and [`Daemon::bind`] opens the store on a data directory against an optional
//! tier above (`--upstream`), warms it from a snapshot file (`--snapshot`), and
//! binds the [`Endpoint`] (`--listen tcp:…`/`unix:…`); [`Daemon::serve`] then
//! serves until a shutdown signal and persists the snapshot.
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

mod config;
mod daemon;
mod snapshot;
#[cfg(test)]
mod test_support;

#[cfg(unix)]
pub use openscion_tier::serve_unix;
pub use openscion_tier::{
    BoundedSource, Error, ObjectSource, Result, SocketSource, Warmed, serve_connection, serve_tcp,
};

pub use config::{
    DEFAULT_BUDGET_BYTES, DEFAULT_LISTEN, DEFAULT_UPSTREAM_CONCURRENCY, DaemonConfig, Endpoint,
    EndpointParseError,
};
pub use daemon::Daemon;

/// The daemon's node-local object store: the shared read-through
/// [`Store`](openscion_tier::Store) over the on-disk
/// [`ObjectCache`](openscion_tier::ObjectCache), re-exported under the daemon's
/// own name.
pub type NodeStore<S> = openscion_tier::Store<S, openscion_tier::ObjectCache>;

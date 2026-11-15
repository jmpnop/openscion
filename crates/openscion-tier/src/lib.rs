//! The shared OpenScion tier layer.
//!
//! The daemon, the regional cache, and the server all answer the **same object
//! RPCs with the same semantics** over the **same framed wire**
//! ([ADR 0004](../../../docs/adr/0004-daemon-transport.md)). This crate holds
//! the machinery they share, so each tier is a thin assembly over it rather
//! than a copy.
//!
//! - [`ObjectSource`] is the port for the tier above: an id resolves to bytes or
//!   exists nowhere above.
//! - [`ObjectBacking`] is the port for a tier's local byte store, keyed by id.
//!   [`ObjectCache`] is the on-disk implementation; a tier that keeps objects in
//!   memory supplies its own.
//! - [`Store`] is the read-through object store: it serves a local copy or, on a
//!   miss, fetches from the tier above, verifies the bytes against the id,
//!   caches them within a byte budget, and evicts least-recently-used. It is
//!   generic over its [`ObjectBacking`], so one implementation serves both an
//!   on-disk daemon and an in-memory cache.
//! - [`BoundedSource`] caps a tier's concurrent upstream fetches and sheds the
//!   overflow with [`Error::Unavailable`], so a saturated tier does not stampede
//!   the one above it.
//! - [`ObjectService`] is what the transport serves: an id resolves to bytes, is
//!   absent, is shed, or fails. [`Store`] implements it, so the daemon and the
//!   cache serve their store; an origin that holds authoritative objects — the
//!   server — implements it over its own store, and the same transport serves
//!   either.
//! - The [transport](serve_connection) carries the object RPCs as length-framed
//!   `openscion-proto` payloads over a Unix socket locally or TCP between
//!   machines; [`SocketSource`] is itself an [`ObjectSource`], so a store on one
//!   tier reads through a server on the tier above over the same wire.
//! - [`Endpoint`] names where a tier listens or the tier above it is dialed —
//!   one scheme-prefixed `tcp:<addr>`/`unix:<path>` form shared by every tier so
//!   a single operator-facing flag spans them; [`dial`](Endpoint::dial) turns
//!   one into the [`SocketSource`] that reads through it.
//!
//! This crate deliberately holds no process assembly, configuration, or
//! signal handling — those belong to the daemon, the cache, and the server that
//! build on it.
//!
//! # Example
//!
//! ```
//! use std::sync::Arc;
//! use openscion_core::ObjectId;
//! use openscion_tier::{ObjectSource, Result, Store};
//!
//! // A tier above that holds nothing.
//! struct Empty;
//! impl ObjectSource for Empty {
//!     fn fetch(&self, _: ObjectId) -> Result<Option<Arc<[u8]>>> {
//!         Ok(None)
//!     }
//! }
//!
//! let dir = std::env::temp_dir().join("openscion-tier-doctest");
//! let store = Store::open(&dir, Empty, 1 << 20)?;
//! assert!(store.snapshot().is_empty());
//! # Ok::<(), openscion_tier::Error>(())
//! ```

mod backing;
mod bound;
mod endpoint;
mod error;
mod service;
mod source;
mod store;
#[cfg(test)]
mod test_support;
mod transport;

pub use backing::{ObjectBacking, ObjectCache};
pub use bound::BoundedSource;
pub use endpoint::{Endpoint, EndpointParseError};
pub use error::{Error, Result};
pub use service::ObjectService;
pub use source::ObjectSource;
pub use store::{Store, Warmed};
#[cfg(unix)]
pub use transport::serve_unix;
pub use transport::{SocketSource, serve_connection, serve_tcp};

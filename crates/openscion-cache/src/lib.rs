//! The OpenScion regional object cache: an in-memory, read-through object tier.
//!
//! A regional cache sits between a machine's [daemon](openscion_tier) and the
//! authoritative server, holding recently used immutable objects — blobs,
//! trees, and changesets — in memory so a region's nodes read them from nearby
//! instead of from the server. It answers the same object reads as the daemon
//! and the server, so a caller talks to whichever tier is nearest without
//! knowing the topology.
//!
//! Because an object can never change, the cache needs no invalidation and no
//! long-lived state: it holds nothing durable, starts empty, and can be
//! restarted at any time, losing only warmth. That is the one way it differs
//! from the daemon, which boots warm from a snapshot on disk.
//!
//! The cache is a thin assembly over [`openscion_tier`]. It adds only
//! [`MemoryBacking`] — the in-memory [`ObjectBacking`] — and its own process
//! wiring; the read-through [`Store`](openscion_tier::Store), the
//! upstream-concurrency [`BoundedSource`], and the framed
//! [transport](serve_connection) are the tier's, shared with the daemon and the
//! server. [`CacheStore`] names the store over the in-memory backing, as the
//! daemon names its disk store `NodeStore`.
//!
//! Tiering is strict and bounded: the store's fetches to the tier above pass
//! through a [`BoundedSource`], which caps how many run at once and sheds the
//! overflow with [`Error::Unavailable`] rather than queueing. The shed crosses
//! the wire as a distinct response, so a daemon reading through a saturated
//! cache receives `Unavailable` and re-sheds instead of stampeding it.
//!
//! The `openscion-cache` binary wires these into a process: [`CacheConfig`]
//! reads flags and `OPENSCION_CACHE_*` environment variables, and
//! [`RegionalCache::bind`] dials the required tier above (`--upstream`), opens
//! the in-memory store over it, and binds the [`Endpoint`] (`--listen tcp:…`/
//! `unix:…`); [`RegionalCache::serve`] then serves until a shutdown signal.
//!
//! # Example
//!
//! ```
//! use openscion_cache::{MemoryBacking, ObjectBacking};
//! use openscion_core::{ObjectKind, object_id};
//!
//! let backing = MemoryBacking::new();
//! let bytes = b"an object";
//! let id = object_id(ObjectKind::Blob, bytes);
//! backing.write(id, bytes)?;
//! assert_eq!(backing.read(id)?.as_deref(), Some(&bytes[..]));
//! # Ok::<(), openscion_cache::Error>(())
//! ```

mod backing;
mod cache;
mod config;
#[cfg(test)]
mod test_support;

#[cfg(unix)]
pub use openscion_tier::serve_unix;
pub use openscion_tier::{
    BoundedSource, Endpoint, EndpointParseError, Error, ObjectBacking, ObjectSource, Result,
    SocketSource, serve_connection, serve_tcp,
};

pub use backing::MemoryBacking;
pub use cache::{CacheStore, RegionalCache};
pub use config::{CacheConfig, DEFAULT_BUDGET_BYTES, DEFAULT_LISTEN, DEFAULT_UPSTREAM_CONCURRENCY};

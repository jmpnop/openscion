//! The forwarded-RPC envelope over the framed tier wire.
//!
//! The daemon, the cache, and the server answer the object RPCs directly. The
//! mutable queries — creating changesets, moving refs, the log, and the
//! derived-data lookups — are answered only by the server; a lower tier forwards
//! them upstream unchanged and never caches them. [`Request`] and [`Response`]
//! are that envelope: one request type a lower tier relays without understanding
//! and the server dispatches on, and the matching reply, with an engine failure
//! carried as [`Response::Error`].
//!
//! The types live here, in the protocol crate, so adding a forwarded operation
//! is adding a variant and a dispatch arm — never a second schema. The framing
//! and serialization are the tier transport's, never the protocol's.

mod types;

pub use types::{Request, Response};

#[cfg(test)]
mod test;

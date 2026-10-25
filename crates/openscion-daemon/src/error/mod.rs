//! Crate-wide error and result types for the daemon.

use std::path::PathBuf;

use openscion_core::ObjectId;

/// Errors returned by the node daemon.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// No tier holds the requested object: it is absent locally and the tier
    /// above returned nothing.
    #[error("object {0} not found in any tier")]
    NotFound(ObjectId),
    /// The tier above returned bytes that do not hash to the requested id under
    /// any object kind, so they are rejected rather than cached.
    #[error("object {id} fetched from upstream did not match its id")]
    Corrupt {
        /// The id whose fetched bytes failed verification.
        id: ObjectId,
    },
    /// A cache file could not be read or written.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or directory involved.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// A message on the wire was malformed, truncated, or not the reply the
    /// request expected.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// The tier above reported an error for a request, relayed over the wire.
    #[error("upstream error: {0}")]
    Upstream(String),
    /// This tier shed the request because its upstream-fetch concurrency bound
    /// was reached — it is declining work, not reporting a failure of the tier
    /// above. A lower tier that receives it sheds in turn rather than retrying.
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// An I/O failure connecting to, reading from, or writing to a peer over
    /// the socket.
    #[error("transport failed: {0}")]
    Transport(#[source] std::io::Error),
}

/// The daemon's standard result type.
pub type Result<T> = std::result::Result<T, Error>;

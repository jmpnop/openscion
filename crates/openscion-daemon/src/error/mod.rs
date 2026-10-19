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
}

/// The daemon's standard result type.
pub type Result<T> = std::result::Result<T, Error>;

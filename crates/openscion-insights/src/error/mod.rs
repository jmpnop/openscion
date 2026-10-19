//! The crate's error and result types.

/// Errors returned by the analytics plane.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The embedded analytics database failed.
    #[error("analytics database error: {0}")]
    Database(#[from] duckdb::Error),
}

/// The crate's standard result type.
pub type Result<T> = std::result::Result<T, Error>;

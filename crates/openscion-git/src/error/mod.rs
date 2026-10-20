//! Crate-wide error and result types for git ingestion.

/// Errors returned while importing a git repository.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The git repository could not be opened, a revision could not be
    /// resolved, or a git object could not be read. The message carries the
    /// `gix` failure.
    #[error("git: {0}")]
    Git(String),
    /// A path in the git tree is not valid UTF-8, so it cannot become an
    /// OpenScion path. The bytes are shown lossily.
    #[error("non-UTF-8 path in the git tree: {}", String::from_utf8_lossy(.0))]
    NonUtf8Path(Vec<u8>),
    /// The OpenScion engine rejected a path, object, or changeset.
    #[error(transparent)]
    Core(#[from] openscion_core::Error),
}

impl From<openscion_core::PathError> for Error {
    fn from(error: openscion_core::PathError) -> Self {
        Self::Core(error.into())
    }
}

/// The ingester's standard result type.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod test;

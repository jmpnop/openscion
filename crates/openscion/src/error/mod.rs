//! Crate-wide error and result types for the client.

use std::path::PathBuf;

use openscion_core::PROTOCOL_VERSION;

/// Errors returned by `opnscn`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The server could not be reached, or answered with an error status and
    /// no error body.
    #[error("request to {url} failed: {message}")]
    Request {
        /// The URL that was requested.
        url: String,
        /// What went wrong, as reported by the HTTP client.
        message: String,
    },
    /// The server answered, but not with the payload the protocol defines.
    #[error("unexpected response from {url}: {message}")]
    Decode {
        /// The URL that was requested.
        url: String,
        /// Why the body could not be decoded.
        message: String,
    },
    /// The server speaks a protocol version this client cannot bind to.
    #[error(
        "server speaks protocol {}.{}, which this client ({}.{}) cannot use",
        server.0, server.1, PROTOCOL_VERSION.0, PROTOCOL_VERSION.1
    )]
    IncompatibleProtocol {
        /// The server's protocol version.
        server: (u32, u32),
    },
    /// The server rejected a request and said why.
    #[error("{url} answered {status} {code}: {message}")]
    Server {
        /// The URL that was requested.
        url: String,
        /// The HTTP status.
        status: u16,
        /// The error code, such as `RefConflict`.
        code: String,
        /// The server's explanation.
        message: String,
    },
    /// The engine rejected local data: a bad path, or an object whose bytes
    /// do not match its id.
    #[error(transparent)]
    Engine(#[from] openscion_core::Error),
    /// A file in the working copy could not be read or written.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or directory involved.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// A file name in the working copy is not UTF-8.
    #[error("{} is not a UTF-8 path", .0.display())]
    NonUtf8Path(PathBuf),
    /// The directory is not an `opnscn` working copy.
    #[error("{} is not an opnscn working copy (no .opnscn/state.json)", .0.display())]
    NotAWorkingCopy(PathBuf),
    /// The directory is already an `opnscn` working copy, or is not empty.
    #[error("{} already holds a working copy or other files", .0.display())]
    AlreadyExists(PathBuf),
    /// The working copy's state file is not valid.
    #[error("{}: corrupt working-copy state: {message}", path.display())]
    BadState {
        /// The state file.
        path: PathBuf,
        /// Why it could not be read.
        message: String,
    },
    /// `opnscn commit` found no changes.
    #[error("nothing to commit")]
    NothingToCommit,
    /// The filesystem watcher behind `opnscn watch` could not be set up or
    /// failed while running.
    #[error("filesystem watch failed: {0}")]
    Watch(String),
    /// `opnscn sparse add` or `remove` was run on a full checkout, which has no
    /// sparse set to edit.
    #[error("the working copy is a full checkout; there are no sparse paths to edit")]
    FullCheckout,
    /// `opnscn sparse remove` was given a path that is not an entry in the
    /// sparse set.
    #[error("{0} is not an entry in the sparse set")]
    NotInSparse(String),
    /// Writing command output failed.
    #[error("cannot write output: {0}")]
    Output(#[source] std::io::Error),
}

impl From<openscion_core::PathError> for Error {
    fn from(error: openscion_core::PathError) -> Self {
        Self::Engine(error.into())
    }
}

/// The client's standard result type.
pub type Result<T> = std::result::Result<T, Error>;

//! Crate-wide error and result types for the server.

use std::net::SocketAddr;

/// Errors returned by the server.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The listen address could not be bound, usually because it is in use or
    /// not local to this machine.
    #[error("cannot listen on {addr}: {source}")]
    Bind {
        /// The address that was requested.
        addr: SocketAddr,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The listener failed while serving.
    #[error("server stopped: {0}")]
    Serve(#[source] std::io::Error),
    /// The persistent data stores under `--data-dir` could not be opened.
    #[error("cannot open the data store: {0}")]
    Store(#[source] openscion_core::Error),
}

/// The server's standard result type.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod test;

//! The HTTP surface: routes, the listener, and graceful shutdown.
//!
//! [`router`] is a plain [`axum::Router`] over a shared repository, so it can
//! be served by [`Server`] or embedded elsewhere. [`Server`] separates binding from serving so a caller —
//! a test, or the binary printing its address — learns the bound address
//! before the first request arrives, even when it asked for port 0.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use axum::routing::get;
use axum::{Json, Router};
use openscion_core::{
    FilesystemObjectStore, PROTOCOL_VERSION, Repository, ServerInfo, SqliteMetadataStore,
    SystemClock,
};
use tokio::net::TcpListener;

use crate::{Error, Result, ServerConfig, SharedRepository, api};

/// Builds the server's HTTP routes over `repository`.
///
/// # Examples
///
/// ```
/// use std::sync::{Arc, RwLock};
/// use openscion_core::Repository;
///
/// let app = openscion_server::router(Arc::new(RwLock::new(Repository::new())));
/// # let _ = app;
/// ```
pub fn router(repository: SharedRepository) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/version", get(version))
        .merge(api::routes())
        .with_state(repository)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn version() -> Json<ServerInfo> {
    Json(ServerInfo::new(env!("CARGO_PKG_VERSION"), PROTOCOL_VERSION))
}

/// A bound, not-yet-serving server with its repository: persistent when the
/// configuration names a data directory, in memory otherwise.
#[derive(Debug)]
pub struct Server {
    listener: TcpListener,
    local_addr: SocketAddr,
    repository: SharedRepository,
}

impl Server {
    /// Binds the configured listen address.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Bind`] when the address cannot be bound.
    pub async fn bind(config: &ServerConfig) -> Result<Self> {
        // Open the stores before binding, so a bad data directory fails fast.
        let repository = Arc::new(RwLock::new(repository(config)?));
        let bind_error = |source| Error::Bind {
            addr: config.listen,
            source,
        };
        let listener = TcpListener::bind(config.listen).await.map_err(bind_error)?;
        let local_addr = listener.local_addr().map_err(bind_error)?;
        Ok(Self {
            listener,
            local_addr,
            repository,
        })
    }

    /// The address the server is bound to, with any port 0 resolved.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Serves [`router`] until `shutdown` resolves, then finishes in-flight
    /// requests and returns.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Serve`] when the listener fails.
    pub async fn serve(self, shutdown: impl Future<Output = ()> + Send + 'static) -> Result<()> {
        axum::serve(self.listener, router(self.repository))
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(Error::Serve)
    }
}

/// Builds the repository the server serves: the filesystem and SQLite stores
/// under the data directory, or in-memory stores when none is configured.
fn repository(config: &ServerConfig) -> Result<Repository> {
    match &config.data_dir {
        None => Ok(Repository::new()),
        Some(dir) => {
            let objects = FilesystemObjectStore::open(dir.join("objects")).map_err(Error::Store)?;
            let meta = SqliteMetadataStore::open(dir.join("meta.db")).map_err(Error::Store)?;
            Ok(Repository::with_stores(
                Box::new(objects),
                Box::new(meta),
                Box::new(SystemClock),
            ))
        }
    }
}

/// Resolves on Ctrl-C, or on SIGTERM on Unix.
///
/// If a signal handler cannot be installed, that signal is ignored rather than
/// shutting the server down immediately.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}

#[cfg(test)]
mod test;

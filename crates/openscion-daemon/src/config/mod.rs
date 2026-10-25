//! The daemon's configuration, read from the command line and environment.
//!
//! Every option has a flag and an `OPENSCION_DAEMON_*` environment variable, so
//! the same binary runs unchanged from a shell, a container, or a service
//! manager — as `openscion-server`.

mod types;

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

pub use types::{Endpoint, EndpointParseError};

/// The endpoint the daemon listens on when none is configured.
///
/// Loopback only: exposing the daemon beyond the machine is a deliberate
/// `--listen` choice, never a default. TCP so the binary runs with only
/// `--data-dir`; a Unix socket is one `--listen unix:/…` away.
pub const DEFAULT_LISTEN: &str = "tcp:127.0.0.1:7979";

/// The resident byte budget when none is configured: one gibibyte.
pub const DEFAULT_BUDGET_BYTES: u64 = 1 << 30;

/// The maximum concurrent upstream fetches when none is configured.
///
/// A conservative bound, well under how many connections the daemon serves at
/// once, so a burst of misses protects the tier above rather than mirroring the
/// node's own load onto it; past it, fetches shed with `Unavailable`.
pub const DEFAULT_UPSTREAM_CONCURRENCY: usize = 32;

/// How the node daemon is configured.
#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(
    name = "openscion-daemon",
    version,
    about = "Serve a node-local OpenScion object store"
)]
#[non_exhaustive]
pub struct DaemonConfig {
    /// Endpoint to accept connections on: `tcp:<addr>` or `unix:<path>`.
    #[arg(long, env = "OPENSCION_DAEMON_LISTEN", default_value = DEFAULT_LISTEN)]
    pub listen: Endpoint,

    /// Directory the store reads cached objects from and writes them to.
    #[arg(long, env = "OPENSCION_DAEMON_DATA_DIR")]
    pub data_dir: PathBuf,

    /// Resident byte budget; least-recently-used objects evict past it.
    #[arg(long, env = "OPENSCION_DAEMON_BUDGET_BYTES", default_value_t = DEFAULT_BUDGET_BYTES)]
    pub budget_bytes: u64,

    /// Tier above to read through for objects not held locally, if any.
    #[arg(long, env = "OPENSCION_DAEMON_UPSTREAM")]
    pub upstream: Option<Endpoint>,

    /// Maximum concurrent fetches to the tier above; past it, fetches shed with
    /// `Unavailable` rather than queueing.
    #[arg(
        long,
        env = "OPENSCION_DAEMON_UPSTREAM_CONCURRENCY",
        default_value_t = DEFAULT_UPSTREAM_CONCURRENCY
    )]
    pub upstream_concurrency: usize,

    /// Warm-set file to boot from and persist to on a clean shutdown.
    #[arg(long, env = "OPENSCION_DAEMON_SNAPSHOT")]
    pub snapshot: Option<PathBuf>,
}

impl DaemonConfig {
    /// A configuration serving the default loopback endpoint from `data_dir`,
    /// with the default budget, no tier above, and no snapshot.
    ///
    /// # Examples
    ///
    /// ```
    /// # use openscion_daemon::{DaemonConfig, DEFAULT_LISTEN};
    /// let config = DaemonConfig::new("/var/lib/openscion");
    /// assert_eq!(config.listen, DEFAULT_LISTEN.parse().unwrap());
    /// assert!(config.upstream.is_none());
    /// ```
    #[must_use]
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            listen: Endpoint::Tcp(SocketAddr::from(([127, 0, 0, 1], 7979))),
            data_dir: data_dir.into(),
            budget_bytes: DEFAULT_BUDGET_BYTES,
            upstream: None,
            upstream_concurrency: DEFAULT_UPSTREAM_CONCURRENCY,
            snapshot: None,
        }
    }

    /// Accepts connections on `listen` instead of the default.
    #[must_use]
    pub fn with_listen(mut self, listen: Endpoint) -> Self {
        self.listen = listen;
        self
    }

    /// Reads through `upstream` for objects not held locally.
    #[must_use]
    pub fn with_upstream(mut self, upstream: Endpoint) -> Self {
        self.upstream = Some(upstream);
        self
    }

    /// Caps concurrent fetches to the tier above at `limit`.
    #[must_use]
    pub fn with_upstream_concurrency(mut self, limit: usize) -> Self {
        self.upstream_concurrency = limit;
        self
    }

    /// Boots from and persists the warm set to `snapshot`.
    #[must_use]
    pub fn with_snapshot(mut self, snapshot: impl Into<PathBuf>) -> Self {
        self.snapshot = Some(snapshot.into());
        self
    }
}

#[cfg(test)]
mod test;

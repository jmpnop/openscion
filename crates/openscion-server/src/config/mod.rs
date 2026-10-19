//! Server configuration, read from the command line and the environment.
//!
//! Every option has a flag and an `OPENSCION_*` environment variable, so the
//! same binary runs unchanged from a shell, a container, or a service manager.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

/// The address the server listens on when none is configured.
///
/// Loopback only: exposing a server beyond the machine is a deliberate choice
/// made with `--listen`, never a default.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:7878";

/// How the server is configured.
#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(
    name = "openscion-server",
    version,
    about = "Serve the OpenScion API over HTTP"
)]
#[non_exhaustive]
pub struct ServerConfig {
    /// Address to listen on. Port 0 picks a free port.
    #[arg(long, env = "OPENSCION_LISTEN", default_value = DEFAULT_LISTEN)]
    pub listen: SocketAddr,

    /// Directory to persist history in. Objects go under `objects/` and the
    /// changeset log and refs in `meta.db`. Without it the server keeps
    /// everything in memory and a restart starts empty.
    #[arg(long, env = "OPENSCION_DATA_DIR")]
    pub data_dir: Option<PathBuf>,
}

impl ServerConfig {
    /// Builds a configuration that listens on `listen` and keeps its history in
    /// memory.
    ///
    /// # Examples
    ///
    /// ```
    /// # use openscion_server::ServerConfig;
    /// let config = ServerConfig::new("127.0.0.1:0".parse().unwrap());
    /// assert_eq!(config.listen.port(), 0);
    /// assert!(config.data_dir.is_none());
    /// ```
    #[must_use]
    pub fn new(listen: SocketAddr) -> Self {
        Self {
            listen,
            data_dir: None,
        }
    }

    /// Persists this configuration's history under `data_dir`.
    #[must_use]
    pub fn with_data_dir(mut self, data_dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(data_dir.into());
        self
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self::new(SocketAddr::from(([127, 0, 0, 1], 7878)))
    }
}

#[cfg(test)]
mod test;

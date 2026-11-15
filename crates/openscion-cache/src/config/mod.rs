//! The cache's configuration, read from the command line and environment.
//!
//! Every option has a flag and an `OPENSCION_CACHE_*` environment variable, so
//! the same binary runs unchanged from a shell, a container, or a service
//! manager. Unlike the daemon, the cache has no data directory and no snapshot:
//! it holds nothing durable and starts empty. Its upstream is required — a cache
//! with nothing above it has nothing to cache.

use std::net::SocketAddr;

use clap::Parser;

use openscion_tier::Endpoint;

/// The endpoint the cache listens on when none is configured.
///
/// Loopback only: exposing the cache beyond the host is a deliberate `--listen`
/// choice, never a default. A regional cache is reached over TCP; the port sits
/// just above the daemon's default so the two can run side by side in
/// development.
pub const DEFAULT_LISTEN: &str = "tcp:127.0.0.1:7989";

/// The resident byte budget when none is configured: one gibibyte.
pub const DEFAULT_BUDGET_BYTES: u64 = 1 << 30;

/// The maximum concurrent upstream fetches when none is configured.
///
/// A conservative bound, well under how many connections the cache serves at
/// once, so a burst of misses protects the tier above rather than mirroring the
/// region's load onto it; past it, fetches shed with `Unavailable`.
pub const DEFAULT_UPSTREAM_CONCURRENCY: usize = 32;

/// How the regional cache is configured.
#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(
    name = "openscion-cache",
    version,
    about = "Serve a regional OpenScion object cache"
)]
#[non_exhaustive]
pub struct CacheConfig {
    /// Endpoint to accept connections on: `tcp:<addr>` or `unix:<path>`.
    #[arg(long, env = "OPENSCION_CACHE_LISTEN", default_value = DEFAULT_LISTEN)]
    pub listen: Endpoint,

    /// Tier above to read through for objects not held locally. Required.
    #[arg(long, env = "OPENSCION_CACHE_UPSTREAM")]
    pub upstream: Endpoint,

    /// Resident byte budget; least-recently-used objects evict past it.
    #[arg(long, env = "OPENSCION_CACHE_BUDGET_BYTES", default_value_t = DEFAULT_BUDGET_BYTES)]
    pub budget_bytes: u64,

    /// Maximum concurrent fetches to the tier above; past it, fetches shed with
    /// `Unavailable` rather than queueing.
    #[arg(
        long,
        env = "OPENSCION_CACHE_UPSTREAM_CONCURRENCY",
        default_value_t = DEFAULT_UPSTREAM_CONCURRENCY
    )]
    pub upstream_concurrency: usize,
}

impl CacheConfig {
    /// A configuration serving the default loopback endpoint, reading through
    /// `upstream`, with the default budget and upstream bound.
    ///
    /// # Examples
    ///
    /// ```
    /// # use openscion_cache::{CacheConfig, DEFAULT_LISTEN};
    /// let config = CacheConfig::new("tcp:10.0.0.1:7979".parse().unwrap());
    /// assert_eq!(config.listen, DEFAULT_LISTEN.parse().unwrap());
    /// assert_eq!(config.upstream, "tcp:10.0.0.1:7979".parse().unwrap());
    /// ```
    #[must_use]
    pub fn new(upstream: Endpoint) -> Self {
        Self {
            listen: Endpoint::Tcp(SocketAddr::from(([127, 0, 0, 1], 7989))),
            upstream,
            budget_bytes: DEFAULT_BUDGET_BYTES,
            upstream_concurrency: DEFAULT_UPSTREAM_CONCURRENCY,
        }
    }

    /// Accepts connections on `listen` instead of the default.
    #[must_use]
    pub fn with_listen(mut self, listen: Endpoint) -> Self {
        self.listen = listen;
        self
    }

    /// Holds at most `budget_bytes` of objects resident.
    #[must_use]
    pub fn with_budget_bytes(mut self, budget_bytes: u64) -> Self {
        self.budget_bytes = budget_bytes;
        self
    }

    /// Caps concurrent fetches to the tier above at `limit`.
    #[must_use]
    pub fn with_upstream_concurrency(mut self, limit: usize) -> Self {
        self.upstream_concurrency = limit;
        self
    }
}

#[cfg(test)]
mod test;

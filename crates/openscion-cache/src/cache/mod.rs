//! Assembling the running cache: dialing the tier above, opening the in-memory
//! store, binding the listener, and serving.
//!
//! This is wiring over the library: a [`RegionalCache`] dials the configured
//! `--upstream` into a [`SocketSource`], bounds its concurrency with a
//! [`BoundedSource`], opens a [`CacheStore`] over a fresh [`MemoryBacking`], and
//! binds its listener with [`Endpoint::bind`](openscion_tier::Endpoint::bind).
//! Unlike the daemon it persists nothing and warms from nothing: the cache
//! starts empty, and a restart loses only warmth.

use std::sync::Arc;

use openscion_tier::{BoundListener, BoundedSource, Forwarding, SocketSource, Store};

use crate::backing::MemoryBacking;
use crate::config::CacheConfig;
use crate::{Error, Result};

/// The cache's object store: the shared read-through
/// [`Store`](openscion_tier::Store) over the in-memory [`MemoryBacking`],
/// reading through a bounded dial of the tier above. Named for its role, as the
/// daemon names its disk store `NodeStore`.
pub type CacheStore<S> = Store<S, MemoryBacking>;

/// A bound regional cache ready to [`serve`](RegionalCache::serve).
#[derive(Debug)]
pub struct RegionalCache {
    store: Arc<CacheStore<BoundedSource<SocketSource>>>,
    upstream: SocketSource,
    listener: BoundListener,
    listening: String,
}

impl RegionalCache {
    /// Dials the configured tier above, opens the in-memory store over it with
    /// the configured budget and upstream bound, and binds the listener.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Transport`] when the listener cannot bind.
    pub fn bind(config: &CacheConfig) -> Result<Self> {
        // Bound the fetches to the tier above: past the limit they shed with
        // `Unavailable` rather than queueing, so a burst of misses cannot
        // stampede the tier above.
        let upstream = BoundedSource::new(config.upstream.dial(), config.upstream_concurrency);
        let store = Arc::new(Store::with_backing(
            upstream,
            config.budget_bytes,
            MemoryBacking::new(),
        ));
        let listener = config.listen.bind().map_err(Error::Transport)?;
        let listening = listener.listening().to_owned();
        Ok(Self {
            store,
            // A second dial of the same tier above, for forwarding non-object
            // RPCs; the store's own dial is bounded and object-only.
            upstream: config.upstream.dial(),
            listener,
            listening,
        })
    }

    /// Where the cache is listening, as a `tcp:<addr>`/`unix:<path>` string —
    /// for `tcp:…:0`, the resolved port.
    #[must_use]
    pub fn listening(&self) -> &str {
        &self.listening
    }

    /// Serves connections, each on its own thread, until a shutdown signal
    /// (`SIGINT`/`SIGTERM`), then exits — there is nothing to persist.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Transport`] when the signal handler cannot be installed.
    #[cfg(unix)]
    pub fn serve(self) -> Result<()> {
        use signal_hook::consts::{SIGINT, SIGTERM};
        use signal_hook::iterator::Signals;

        let Self {
            store,
            upstream,
            listener,
            ..
        } = self;

        // The accept loop blocks forever, so it runs on its own thread and
        // shutdown is driven by a signal on this one. The cache keeps no state,
        // so there is nothing to flush when the signal arrives. It serves
        // objects from its store and forwards non-object RPCs to the tier above.
        let serving = Arc::new(Forwarding::new(store, Some(upstream)));
        std::thread::spawn(move || {
            let _ = listener.serve(&serving);
        });

        let mut signals = Signals::new([SIGINT, SIGTERM]).map_err(Error::Transport)?;
        let _ = signals.forever().next();
        Ok(())
    }

    /// Serves connections, each on its own thread, until the accept loop ends.
    ///
    /// Signal-driven shutdown is Unix-only; on other platforms the cache serves
    /// until terminated.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Transport`] when accepting a connection fails.
    #[cfg(not(unix))]
    pub fn serve(self) -> Result<()> {
        let Self {
            store,
            upstream,
            listener,
            ..
        } = self;
        let serving = Arc::new(Forwarding::new(store, Some(upstream)));
        listener.serve(&serving)
    }
}

#[cfg(test)]
mod test;

//! Assembling the running daemon: the tier above, the bound listener, warm boot
//! from a snapshot, serving, and persisting the snapshot on a clean shutdown.
//!
//! This is wiring over the library: a [`Daemon`] opens a
//! [`NodeStore`](crate::NodeStore) on a data directory against an [`Upstream`]
//! tier above, warms it from the snapshot file if one is configured, binds the
//! [`Endpoint`] listener, and serves each connection on a thread. On a Unix
//! shutdown signal it persists the current warm set back to the snapshot file.

use std::path::PathBuf;
use std::sync::Arc;

use openscion_core::ObjectId;

use openscion_tier::{
    BoundListener, BoundedSource, Forwarding, ObjectSource, SocketSource, Warmed,
};

use crate::config::{DaemonConfig, Endpoint};
use crate::snapshot;
use crate::{Error, NodeStore, Result};

/// The tier above the node store: a daemon dialed over a socket, or nothing.
///
/// A concrete type rather than a trait object so the whole daemon is one
/// monomorphized [`NodeStore`]; a detached daemon is a leaf whose every miss is
/// absent.
#[derive(Debug, Clone)]
pub(crate) enum Upstream {
    /// Read through a daemon on the tier above.
    Socket(SocketSource),
    /// A leaf: hold only local objects, and report every miss as absent.
    Detached,
}

impl Upstream {
    /// Builds the tier above from the configured `--upstream` endpoint.
    fn from_endpoint(endpoint: Option<&Endpoint>) -> Self {
        match endpoint {
            Some(endpoint) => Self::Socket(endpoint.dial()),
            None => Self::Detached,
        }
    }
}

impl ObjectSource for Upstream {
    fn fetch(&self, id: ObjectId) -> Result<Option<Arc<[u8]>>> {
        match self {
            Self::Socket(source) => source.fetch(id),
            Self::Detached => Ok(None),
        }
    }
}

/// A bound, warmed node daemon ready to [`serve`](Daemon::serve).
#[derive(Debug)]
pub struct Daemon {
    store: Arc<NodeStore<BoundedSource<Upstream>>>,
    upstream: Option<SocketSource>,
    listener: BoundListener,
    listening: String,
    snapshot_path: Option<PathBuf>,
    warmed: Warmed,
}

impl Daemon {
    /// Opens the store on the configured data directory against the configured
    /// tier above, warms it from the snapshot file if one is configured, and
    /// binds the listener.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the data directory or snapshot file cannot be
    /// used, [`Error::Transport`] when the listener cannot bind, and the
    /// upstream error ([`Error::Transport`]/[`Error::Upstream`]) when warming
    /// cannot reach the tier above.
    pub fn bind(config: &DaemonConfig) -> Result<Self> {
        // Bound the fetches to the tier above: past the limit they shed with
        // `Unavailable` rather than queueing, so a burst of misses cannot
        // stampede the tier above.
        let upstream = BoundedSource::new(
            Upstream::from_endpoint(config.upstream.as_ref()),
            config.upstream_concurrency,
        );
        let store = Arc::new(NodeStore::open(
            &config.data_dir,
            upstream,
            config.budget_bytes,
        )?);
        let warmed = match &config.snapshot {
            Some(path) => store.warm(&snapshot::read_ids(path)?)?,
            None => store.warm(&[])?,
        };
        let listener = config.listen.bind().map_err(Error::Transport)?;
        let listening = listener.listening().to_owned();
        Ok(Self {
            store,
            // A dial of the tier above for forwarding non-object RPCs, or `None`
            // when detached — a leaf daemon forwards nowhere and declines them.
            upstream: config.upstream.as_ref().map(Endpoint::dial),
            listener,
            listening,
            snapshot_path: config.snapshot.clone(),
            warmed,
        })
    }

    /// Where the daemon is listening, as a `tcp:<addr>`/`unix:<path>` string —
    /// for `tcp:…:0`, the resolved port.
    #[must_use]
    pub fn listening(&self) -> &str {
        &self.listening
    }

    /// What warming the store from the snapshot fetched.
    #[must_use]
    pub fn warmed(&self) -> Warmed {
        self.warmed
    }

    /// Serves connections, each on its own thread, until a shutdown signal
    /// (`SIGINT`/`SIGTERM`), then persists the snapshot if one is configured.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Transport`] when the signal handler cannot be installed,
    /// and [`Error::Io`] when the snapshot cannot be written on shutdown.
    #[cfg(unix)]
    pub fn serve(self) -> Result<()> {
        use signal_hook::consts::{SIGINT, SIGTERM};
        use signal_hook::iterator::Signals;

        let Self {
            store,
            upstream,
            listener,
            snapshot_path,
            ..
        } = self;

        // The accept loop blocks forever, so it runs on its own thread and
        // shutdown is driven by a signal on this one. It serves objects from the
        // node store and forwards non-object RPCs to the tier above; the store
        // is kept here to persist the snapshot on shutdown.
        let serving = Arc::new(Forwarding::new(Arc::clone(&store), upstream));
        std::thread::spawn(move || {
            let _ = listener.serve(&serving);
        });

        let mut signals = Signals::new([SIGINT, SIGTERM]).map_err(Error::Transport)?;
        let _ = signals.forever().next();

        if let Some(path) = snapshot_path {
            snapshot::write_ids(&path, &store.snapshot())?;
        }
        Ok(())
    }

    /// Serves connections, each on its own thread, until the accept loop ends.
    ///
    /// Signal-driven shutdown and snapshot persistence are Unix-only; on other
    /// platforms the daemon serves until terminated.
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
        let serving = Arc::new(Forwarding::new(store, upstream));
        listener.serve(&serving)
    }
}

#[cfg(test)]
mod test;

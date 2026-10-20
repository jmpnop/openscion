//! Assembling the running daemon: the tier above, the bound listener, warm boot
//! from a snapshot, serving, and persisting the snapshot on a clean shutdown.
//!
//! This is wiring over the library: a [`Daemon`] opens a
//! [`NodeStore`](crate::NodeStore) on a data directory against an [`Upstream`]
//! tier above, warms it from the snapshot file if one is configured, binds the
//! [`Endpoint`] listener, and serves each connection on a thread. On a Unix
//! shutdown signal it persists the current warm set back to the snapshot file.

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(unix)]
use std::os::unix::net::UnixListener;

use openscion_core::ObjectId;

use crate::config::{DaemonConfig, Endpoint};
use crate::snapshot;
use crate::store::{NodeStore, ObjectSource, Warmed};
#[cfg(unix)]
use crate::transport::serve_unix;
use crate::transport::{SocketSource, serve_tcp};
use crate::{Error, Result};

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
            Some(Endpoint::Tcp(addr)) => Self::Socket(SocketSource::tcp(*addr)),
            #[cfg(unix)]
            Some(Endpoint::Unix(path)) => Self::Socket(SocketSource::unix(path.clone())),
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

/// The daemon's bound listener, over whichever transport `--listen` named.
#[derive(Debug)]
enum Listener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(UnixListener),
}

/// A bound, warmed node daemon ready to [`serve`](Daemon::serve).
#[derive(Debug)]
pub struct Daemon {
    store: Arc<NodeStore<Upstream>>,
    listener: Listener,
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
        let upstream = Upstream::from_endpoint(config.upstream.as_ref());
        let store = Arc::new(NodeStore::open(
            &config.data_dir,
            upstream,
            config.budget_bytes,
        )?);
        let warmed = match &config.snapshot {
            Some(path) => store.warm(&snapshot::read_ids(path)?)?,
            None => store.warm(&[])?,
        };
        let (listener, listening) = bind_listener(&config.listen)?;
        Ok(Self {
            store,
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
            listener,
            snapshot_path,
            ..
        } = self;

        // The accept loop blocks forever, so it runs on its own thread and
        // shutdown is driven by a signal on this one.
        let serving = Arc::clone(&store);
        std::thread::spawn(move || match listener {
            Listener::Tcp(tcp) => {
                let _ = serve_tcp(&tcp, &serving);
            }
            Listener::Unix(unix) => {
                let _ = serve_unix(&unix, &serving);
            }
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
            store, listener, ..
        } = self;
        match listener {
            Listener::Tcp(tcp) => serve_tcp(&tcp, &store),
        }
    }
}

/// Binds the listener named by `endpoint`, returning it with the resolved
/// `tcp:<addr>`/`unix:<path>` string to report.
fn bind_listener(endpoint: &Endpoint) -> Result<(Listener, String)> {
    match endpoint {
        Endpoint::Tcp(addr) => {
            let listener = TcpListener::bind(addr).map_err(Error::Transport)?;
            let resolved = listener.local_addr().map_err(Error::Transport)?;
            Ok((Listener::Tcp(listener), format!("tcp:{resolved}")))
        }
        #[cfg(unix)]
        Endpoint::Unix(path) => {
            let listener = UnixListener::bind(path).map_err(Error::Transport)?;
            Ok((Listener::Unix(listener), format!("unix:{}", path.display())))
        }
    }
}

#[cfg(test)]
mod test;

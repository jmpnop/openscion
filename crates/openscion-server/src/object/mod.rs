//! The framed object endpoint: serving the authoritative repository as a tier.
//!
//! The daemon and the regional cache serve the framed object RPCs
//! ([ADR 0004](../../../../docs/adr/0004-daemon-transport.md)) from a
//! read-through `Store`; the server is the origin of that read-through chain and
//! serves them from its authoritative [`Repository`](openscion_core::Repository)
//! instead. [`RepositoryObjects`] adapts the shared repository to the tier's
//! [`ObjectService`], and [`serve_objects`] runs the tier transport's accept
//! loop over a bound [`Endpoint`] on its own thread, alongside the async HTTP
//! server.
//!
//! The endpoint is read-only: it answers object gets and nothing else, as the
//! wire does at every tier. As the origin it has no tier above and sheds
//! nothing — a get is found or absent.

use std::net::TcpListener;
use std::sync::Arc;

#[cfg(unix)]
use std::os::unix::net::UnixListener;

use openscion_core::ObjectId;
#[cfg(unix)]
use openscion_tier::serve_unix;
use openscion_tier::{Endpoint, ObjectService, serve_tcp};

use crate::{Error, Result, SharedRepository};

/// The server's authoritative objects as a tier [`ObjectService`].
///
/// It serves object bytes straight from the shared
/// [`Repository`](openscion_core::Repository), under the same read lock the HTTP
/// read handlers take, so a framed get and an HTTP request see one repository
/// under one lock discipline. As the origin of the read-through chain it has no
/// tier above and sheds nothing: a get is found or absent.
pub(crate) struct RepositoryObjects {
    repository: SharedRepository,
}

impl RepositoryObjects {
    /// Serves objects from `repository`.
    pub(crate) fn new(repository: SharedRepository) -> Self {
        Self { repository }
    }
}

impl ObjectService for RepositoryObjects {
    fn get(&self, id: ObjectId) -> openscion_tier::Result<Arc<[u8]>> {
        // Take the read lock only across the lookup: the guard drops at the end
        // of this statement, before the bytes are mapped onto the wire result.
        let found = crate::api::read(&self.repository).object(id);
        map_object(found, id)
    }
}

/// Maps an engine object lookup onto the tier's serving result: found bytes are
/// served, the engine's absent case becomes the tier's `NotFound` (which the
/// transport answers as *absent*), and any other engine failure is relayed as an
/// upstream error. The origin never sheds, so `Unavailable` never arises here.
fn map_object(
    found: openscion_core::Result<Arc<[u8]>>,
    id: ObjectId,
) -> openscion_tier::Result<Arc<[u8]>> {
    match found {
        Ok(bytes) => Ok(bytes),
        Err(openscion_core::Error::NotFound(_)) => Err(openscion_tier::Error::NotFound(id)),
        Err(error) => Err(openscion_tier::Error::Upstream(error.to_string())),
    }
}

/// The server's bound framed listener, over whichever transport the endpoint
/// named.
#[derive(Debug)]
pub(crate) enum Listener {
    /// A TCP listener, for a `tcp:<addr>` endpoint.
    Tcp(TcpListener),
    /// A Unix-domain listener, for a `unix:<path>` endpoint.
    #[cfg(unix)]
    Unix(UnixListener),
}

/// Binds the framed object listener named by `endpoint`, returning it with the
/// resolved `tcp:<addr>`/`unix:<path>` string to report.
///
/// # Errors
///
/// Returns [`Error::ObjectListen`] when the endpoint cannot be bound.
pub(crate) fn bind_object_listener(endpoint: &Endpoint) -> Result<(Listener, String)> {
    match endpoint {
        Endpoint::Tcp(addr) => {
            let listener = TcpListener::bind(addr).map_err(listen_error(endpoint))?;
            let resolved = listener.local_addr().map_err(listen_error(endpoint))?;
            Ok((Listener::Tcp(listener), format!("tcp:{resolved}")))
        }
        #[cfg(unix)]
        Endpoint::Unix(path) => {
            let listener = UnixListener::bind(path).map_err(listen_error(endpoint))?;
            Ok((Listener::Unix(listener), format!("unix:{}", path.display())))
        }
    }
}

/// Maps a bind failure to [`Error::ObjectListen`], naming the endpoint.
fn listen_error(endpoint: &Endpoint) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::ObjectListen {
        endpoint: endpoint.clone(),
        source,
    }
}

/// Serves the framed object RPCs from `service` on `listener`, each connection
/// on its own thread, on a dedicated thread so it runs alongside the async HTTP
/// server.
///
/// The accept loop lives for the process and is torn down with it; because
/// objects are immutable and each framed connection is stateless, a connection
/// dropped at shutdown costs a caller only a retry, so the endpoint needs no
/// drain of its own.
pub(crate) fn serve_objects(listener: Listener, service: Arc<RepositoryObjects>) {
    std::thread::spawn(move || match listener {
        Listener::Tcp(tcp) => {
            let _ = serve_tcp(&tcp, &service);
        }
        #[cfg(unix)]
        Listener::Unix(unix) => {
            let _ = serve_unix(&unix, &service);
        }
    });
}

#[cfg(test)]
mod test;

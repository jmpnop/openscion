//! The object-read surface the framed transport serves.
//!
//! [`ObjectService`] is what [`serve_connection`](crate::serve_connection) and
//! its listeners answer requests from: an id resolves to bytes, is absent, is
//! shed, or fails. The read-through [`Store`] implements it, so the daemon and
//! the cache serve their store directly. An origin that holds authoritative
//! objects — the server — implements it over its own object store instead, with
//! no tier above and nothing to shed, and the *same* transport serves either.
//! This is the seam [ADR 0004](../../../../docs/adr/0004-daemon-transport.md)
//! intends when it says the daemon, the cache, and the server speak the same
//! frames.

use std::sync::Arc;

use openscion_core::rpc::{Request, Response};
use openscion_core::{ErrorBody, ObjectId};

use crate::Result;
use crate::backing::ObjectBacking;
use crate::source::ObjectSource;
use crate::store::Store;

/// A tier's object-read surface, as the framed transport serves it.
///
/// The transport maps a [`get`](Self::get) outcome onto the wire: `Ok` bytes are
/// a *found* response, [`NotFound`](crate::Error::NotFound) is *absent*,
/// [`Unavailable`](crate::Error::Unavailable) is a *shed*, and any other error is
/// a relayed protocol error. An implementation therefore chooses its wire answer
/// by choosing its `Result`.
pub trait ObjectService: Send + Sync {
    /// The bytes of object `id`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotFound`](crate::Error::NotFound) when no tier holds
    /// `id`, [`Error::Unavailable`](crate::Error::Unavailable) when the service
    /// sheds the request rather than serving it, and any other
    /// [`Error`](crate::Error) for a failure to relay to the caller.
    fn get(&self, id: ObjectId) -> Result<Arc<[u8]>>;

    /// Answers a forwarded non-object RPC ([`Request`]) with its [`Response`].
    ///
    /// Only the server answers these from its engine; the default declines, for
    /// a tier that serves objects but does not itself answer mutable queries
    /// (the daemon and cache forward them to the tier above instead). The reply
    /// is a value, not a transport fault, so declining keeps the connection
    /// usable.
    ///
    /// # Errors
    ///
    /// Returns an error only when the connection should be dropped; a refusal or
    /// an engine failure is carried in [`Response::Error`], returned as `Ok`.
    fn rpc(&self, _request: Request) -> Result<Response> {
        Ok(Response::Error(ErrorBody::new(
            "Unsupported",
            "this tier does not serve non-object RPCs",
        )))
    }
}

/// The read-through store is an object service: it serves a local copy or reads
/// through to the tier above, exactly as its [`get`](Store::get) already does.
/// It declines forwarded RPCs by the trait default; a tier that forwards wraps
/// it in a [`Forwarding`].
impl<S: ObjectSource, B: ObjectBacking> ObjectService for Store<S, B> {
    fn get(&self, id: ObjectId) -> Result<Arc<[u8]>> {
        Store::get(self, id)
    }
}

/// An object service that serves objects from a local [`Store`] and forwards
/// non-object RPCs to the tier above.
///
/// This is how the daemon and the regional cache serve mutable queries: they
/// answer the object RPCs from their store and relay everything else upstream
/// unchanged, as the architecture's strict tiering prescribes. A tier with no
/// tier above (a detached daemon) forwards nowhere and declines the RPC.
#[derive(Debug)]
pub struct Forwarding<S: ObjectSource, B: ObjectBacking> {
    store: Arc<Store<S, B>>,
    upstream: Option<crate::transport::SocketSource>,
}

impl<S: ObjectSource, B: ObjectBacking> Forwarding<S, B> {
    /// Serves objects from `store` and forwards RPCs to `upstream`, or declines
    /// them when there is no tier above.
    #[must_use]
    pub fn new(store: Arc<Store<S, B>>, upstream: Option<crate::transport::SocketSource>) -> Self {
        Self { store, upstream }
    }
}

impl<S: ObjectSource, B: ObjectBacking> ObjectService for Forwarding<S, B> {
    fn get(&self, id: ObjectId) -> Result<Arc<[u8]>> {
        self.store.get(id)
    }

    fn rpc(&self, request: Request) -> Result<Response> {
        match &self.upstream {
            Some(upstream) => upstream.rpc(&request),
            None => Ok(Response::Error(ErrorBody::new(
                "Unavailable",
                "this tier has no tier above to forward the request to",
            ))),
        }
    }
}

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

use openscion_core::ObjectId;

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
}

/// The read-through store is an object service: it serves a local copy or reads
/// through to the tier above, exactly as its [`get`](Store::get) already does.
impl<S: ObjectSource, B: ObjectBacking> ObjectService for Store<S, B> {
    fn get(&self, id: ObjectId) -> Result<Arc<[u8]>> {
        Store::get(self, id)
    }
}

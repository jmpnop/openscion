//! The framed object endpoint: serving the authoritative repository as a tier.
//!
//! The daemon and the regional cache serve the framed object RPCs
//! ([ADR 0004](../../../../docs/adr/0004-daemon-transport.md)) from a
//! read-through `Store`; the server is the origin of that read-through chain and
//! serves them from its authoritative [`Repository`](openscion_core::Repository)
//! instead. [`RepositoryObjects`] adapts the shared repository to the tier's
//! [`ObjectService`], which an [`Endpoint::bind`](openscion_tier::Endpoint::bind)
//! listener serves beside the async HTTP server.
//!
//! The endpoint is read-only: it answers object gets and nothing else, as the
//! wire does at every tier. As the origin it has no tier above and sheds
//! nothing — a get is found or absent.

use std::sync::Arc;

use openscion_core::ObjectId;
use openscion_tier::ObjectService;

use crate::SharedRepository;

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

#[cfg(test)]
mod test;

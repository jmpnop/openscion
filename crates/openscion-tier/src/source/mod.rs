//! The port for the tier above: where a store fetches objects it does not hold.

use std::sync::Arc;

use openscion_core::ObjectId;

use crate::Result;

/// The tier above a store — the regional cache, or the server when there is no
/// cache. A store asks it for objects it does not hold locally.
///
/// The store does not know whether the source is a network client, another
/// store, or a test fake; only that an id either resolves to bytes or exists
/// nowhere above.
pub trait ObjectSource: Send + Sync {
    /// The bytes of object `id` from the tier above, or `None` when no tier
    /// above holds it.
    ///
    /// # Errors
    ///
    /// Returns an error only when the upstream could not be reached or answered
    /// improperly — never for a plain absence, which is `Ok(None)`.
    fn fetch(&self, id: ObjectId) -> Result<Option<Arc<[u8]>>>;
}

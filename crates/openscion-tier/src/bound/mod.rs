//! Bounding a tier's concurrent upstream fetches, shedding the overflow.
//!
//! [`BoundedSource`] wraps the [`ObjectSource`] a store reads through and caps
//! how many fetches run against it at once. While `limit` fetches are in flight,
//! a further fetch does not queue or block: it is shed, returning
//! [`Error::Unavailable`] at once. A fetch that returns — with bytes, with an
//! absence, or with an error — frees its slot for the next.
//!
//! This is the architecture's upstream-concurrency bound: strict tiering means
//! each tier bounds its calls to the tier above and sheds when saturated, so a
//! slow or failing upstream collects a bounded fan-in and a lower tier backs off
//! instead of amplifying one tier's trouble into a stampede on the next. The
//! shed crosses the wire as a distinct response (see [`transport`]), so a store
//! reading through a saturated server receives `Unavailable` and re-sheds.
//!
//! The slot count is a single lock-free [`AtomicUsize`]: at most `limit` fetches
//! are ever inside `inner.fetch` at once, a shed never takes a slot, and a slot
//! is released when the fetch returns whatever the outcome, because the
//! [`Permit`] that holds it releases on drop.
//!
//! [`transport`]: crate::serve_connection

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use openscion_core::ObjectId;

use crate::source::ObjectSource;
use crate::{Error, Result};

/// An [`ObjectSource`] that caps concurrent fetches against an inner source and
/// sheds the overflow with [`Error::Unavailable`].
#[derive(Debug)]
pub struct BoundedSource<S> {
    inner: S,
    in_flight: AtomicUsize,
    limit: usize,
}

impl<S> BoundedSource<S> {
    /// Wraps `inner`, allowing at most `limit` concurrent fetches through it.
    ///
    /// A `limit` of 0 sheds every fetch — a deliberate "serve only what is
    /// local" bound; the daemon never configures 0.
    #[must_use]
    pub fn new(inner: S, limit: usize) -> Self {
        Self {
            inner,
            in_flight: AtomicUsize::new(0),
            limit,
        }
    }

    /// The maximum number of concurrent fetches allowed through.
    #[must_use]
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Takes a slot if fewer than `limit` are in use, else `None`. The compare
    /// and increment are one atomic step, so no two callers can both take the
    /// last slot.
    fn acquire(&self) -> Option<Permit<'_>> {
        self.in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |in_flight| {
                (in_flight < self.limit).then_some(in_flight + 1)
            })
            .ok()
            .map(|_| Permit {
                in_flight: &self.in_flight,
            })
    }
}

impl<S: ObjectSource> ObjectSource for BoundedSource<S> {
    fn fetch(&self, id: ObjectId) -> Result<Option<Arc<[u8]>>> {
        let _permit = self.acquire().ok_or_else(|| {
            Error::Unavailable(format!(
                "upstream fetch concurrency limit of {} reached",
                self.limit
            ))
        })?;
        self.inner.fetch(id)
    }
}

/// A held fetch slot. Dropping it — on return, early error, or unwind — frees
/// the slot, so a slot is never leaked.
struct Permit<'a> {
    in_flight: &'a AtomicUsize,
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod test;

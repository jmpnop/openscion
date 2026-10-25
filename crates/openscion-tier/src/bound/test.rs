//! Unit tests for the bounded upstream source.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Condvar, Mutex};

use openscion_core::ObjectId;

use crate::ObjectSource;
use crate::test_support::MemorySource;
use crate::{Error, Result};

use super::BoundedSource;

fn id(byte: u8) -> ObjectId {
    ObjectId::from_bytes([byte; 32])
}

/// An inner source whose every fetch fails, to prove the slot is freed after an
/// error just as after a success.
struct ErrSource;

impl ObjectSource for ErrSource {
    fn fetch(&self, _: ObjectId) -> Result<Option<Arc<[u8]>>> {
        Err(Error::Upstream("boom".to_owned()))
    }
}

/// An inner source that blocks each fetch inside itself until the test releases
/// it, tracking how many fetches are inside at once so a test can hold slots
/// open and observe the peak concurrency the bound allowed.
#[derive(Clone)]
struct GateSource {
    gate: Arc<Gate>,
}

struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

#[derive(Default)]
struct GateState {
    inside: usize,
    peak: usize,
    released: bool,
}

impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(GateState::default()),
            changed: Condvar::new(),
        })
    }

    /// Blocks until at least `n` fetches are inside the source at once.
    fn wait_until_inside(&self, n: usize) {
        let mut state = self.state.lock().unwrap();
        while state.inside < n {
            state = self.changed.wait(state).unwrap();
        }
    }

    /// Releases every blocked fetch and every later one.
    fn release(&self) {
        let mut state = self.state.lock().unwrap();
        state.released = true;
        self.changed.notify_all();
    }

    /// The most fetches that were inside at once.
    fn peak(&self) -> usize {
        self.state.lock().unwrap().peak
    }
}

impl ObjectSource for GateSource {
    fn fetch(&self, _: ObjectId) -> Result<Option<Arc<[u8]>>> {
        let mut state = self.gate.state.lock().unwrap();
        state.inside += 1;
        state.peak = state.peak.max(state.inside);
        self.gate.changed.notify_all();
        while !state.released {
            state = self.gate.changed.wait(state).unwrap();
        }
        state.inside -= 1;
        self.gate.changed.notify_all();
        Ok(Some(Arc::from(&b"gated"[..])))
    }
}

#[test]
fn within_the_limit_it_passes_bytes_and_absence_through() {
    let inner = MemorySource::new();
    let present = inner.add(b"hello");
    let source = BoundedSource::new(inner, 1);

    assert_eq!(
        source.fetch(present).unwrap().as_deref(),
        Some(&b"hello"[..])
    );
    // A second fetch proves the first freed its slot; an unknown id is a plain
    // absence, not a shed.
    assert!(source.fetch(id(9)).unwrap().is_none());
}

#[test]
fn an_inner_error_passes_through_and_frees_the_slot() {
    let source = BoundedSource::new(ErrSource, 1);
    // Twice: the second only reaches the inner error if the first freed its
    // slot rather than leaving the source shedding.
    assert!(matches!(source.fetch(id(1)), Err(Error::Upstream(_))));
    assert!(matches!(source.fetch(id(2)), Err(Error::Upstream(_))));
}

#[test]
fn at_the_limit_a_fetch_is_shed_without_touching_inner() {
    let gate = Gate::new();
    let source = Arc::new(BoundedSource::new(GateSource { gate: gate.clone() }, 1));

    let held = {
        let source = Arc::clone(&source);
        std::thread::spawn(move || source.fetch(id(1)))
    };
    gate.wait_until_inside(1);

    // The one slot is held; this fetch sheds at once instead of queueing.
    assert!(matches!(source.fetch(id(2)), Err(Error::Unavailable(_))));
    // The shed never entered the inner source.
    assert_eq!(gate.peak(), 1);

    gate.release();
    assert!(held.join().unwrap().unwrap().is_some());

    // A released slot lets the next fetch through.
    assert!(source.fetch(id(3)).unwrap().is_some());
}

#[test]
fn it_allows_at_most_the_limit_of_concurrent_fetches() {
    const LIMIT: usize = 3;
    let gate = Gate::new();
    let source = Arc::new(BoundedSource::new(GateSource { gate: gate.clone() }, LIMIT));

    let held: Vec<_> = (0..LIMIT)
        .map(|n| {
            let source = Arc::clone(&source);
            std::thread::spawn(move || source.fetch(id(u8::try_from(n).unwrap())))
        })
        .collect();
    gate.wait_until_inside(LIMIT);

    // All slots are held; the next fetch sheds.
    assert!(matches!(source.fetch(id(200)), Err(Error::Unavailable(_))));

    gate.release();
    for handle in held {
        assert!(handle.join().unwrap().unwrap().is_some());
    }
    // The bound never let more than the limit in at once.
    assert_eq!(gate.peak(), LIMIT);
}

#[test]
fn a_limit_of_zero_sheds_every_fetch_and_names_the_limit() {
    let inner = MemorySource::new();
    let present = inner.add(b"unreachable");
    let source = BoundedSource::new(inner, 0);

    match source.fetch(present) {
        Err(Error::Unavailable(message)) => {
            assert!(message.contains('0'), "message names the limit: {message}");
            assert!(message.contains("concurrency limit"));
        }
        other => panic!("expected a shed, got {other:?}"),
    }
}

#[test]
fn it_reports_its_limit() {
    assert_eq!(BoundedSource::new(ErrSource, 7).limit(), 7);
}

#[test]
fn the_shed_error_displays_its_reason() {
    assert_eq!(
        Error::Unavailable("slots full".to_owned()).to_string(),
        "unavailable: slots full"
    );
}

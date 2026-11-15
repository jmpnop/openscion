//! Unit tests for the cache wiring: the in-memory read-through store over the
//! wire, load shedding, and binding.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;

use openscion_core::ObjectId;
use openscion_tier::{BoundedSource, SocketSource, Store};

use super::{CacheStore, RegionalCache};
use crate::backing::MemoryBacking;
use crate::config::CacheConfig;
use crate::test_support::{MemorySource, scratch};
use crate::{Endpoint, serve_tcp};

/// Stands up an in-process server over TCP holding `objects` in memory — a stand
/// -in for the tier above the cache — and returns its address and the ids it
/// holds.
fn serve_upstream(objects: &[&[u8]]) -> (SocketAddr, Vec<ObjectId>) {
    let memory = MemorySource::new();
    let ids = objects.iter().map(|bytes| memory.add(bytes)).collect();
    let store: Arc<CacheStore<MemorySource>> =
        Arc::new(Store::with_backing(memory, 1 << 20, MemoryBacking::new()));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let _ = serve_tcp(&listener, &store);
    });
    (addr, ids)
}

#[test]
fn the_store_reads_through_the_wire_then_serves_locally() {
    let (addr, ids) = serve_upstream(&[b"shared object"]);
    let store: CacheStore<SocketSource> =
        Store::with_backing(SocketSource::tcp(addr), 1 << 20, MemoryBacking::new());

    // A cold miss reads through to the tier above over the framed wire.
    assert_eq!(&*store.get(ids[0]).unwrap(), b"shared object");
    // Cached in memory now: it reports the id in its snapshot and serves it
    // without the tier above.
    assert_eq!(store.snapshot(), ids);
}

#[test]
fn a_bounded_store_sheds_with_unavailable() {
    let (addr, ids) = serve_upstream(&[b"would be served"]);
    // The tier above holds the object, but the upstream is bounded at zero, so a
    // cold miss sheds before reaching it.
    let store: CacheStore<BoundedSource<SocketSource>> = Store::with_backing(
        BoundedSource::new(SocketSource::tcp(addr), 0),
        1 << 20,
        MemoryBacking::new(),
    );
    assert!(matches!(
        store.get(ids[0]),
        Err(crate::Error::Unavailable(_))
    ));
}

#[test]
fn bind_reports_its_resolved_tcp_endpoint() {
    let (upstream, _) = serve_upstream(&[]);
    let config = CacheConfig::new(Endpoint::Tcp(upstream))
        .with_listen(Endpoint::Tcp(([127, 0, 0, 1], 0).into()));
    let cache = RegionalCache::bind(&config).unwrap();
    assert!(cache.listening().starts_with("tcp:127.0.0.1:"));
}

#[test]
fn a_bind_on_a_taken_tcp_address_is_a_transport_error() {
    let (upstream, _) = serve_upstream(&[]);
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = taken.local_addr().unwrap();
    let config = CacheConfig::new(Endpoint::Tcp(upstream)).with_listen(Endpoint::Tcp(addr));
    assert!(matches!(
        RegionalCache::bind(&config),
        Err(crate::Error::Transport(_))
    ));
}

#[cfg(unix)]
#[test]
fn bind_on_a_unix_socket_reports_its_path() {
    let (upstream, _) = serve_upstream(&[]);
    let path = scratch("unix").join("cache.sock");
    let config =
        CacheConfig::new(Endpoint::Tcp(upstream)).with_listen(Endpoint::Unix(path.clone()));
    let cache = RegionalCache::bind(&config).unwrap();
    assert_eq!(cache.listening(), format!("unix:{}", path.display()));
}

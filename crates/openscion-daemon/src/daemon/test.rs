//! Unit tests for the daemon wiring: the tier above, warm boot, and binding.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;

use openscion_core::ObjectId;

use super::{Daemon, Upstream};
use crate::config::{DaemonConfig, Endpoint};
use crate::store::NodeStore;
use crate::test_support::{MemorySource, scratch};
use crate::transport::serve_tcp;
use crate::{ObjectSource, snapshot};

/// Stands up an in-process daemon over TCP serving `objects` from memory, and
/// returns its address and the ids it holds.
fn serve_memory(objects: &[&[u8]]) -> (SocketAddr, Vec<ObjectId>) {
    let memory = MemorySource::new();
    let ids = objects.iter().map(|bytes| memory.add(bytes)).collect();
    let store = Arc::new(NodeStore::open(&scratch("upstream"), memory, 1 << 20).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let _ = serve_tcp(&listener, &store);
    });
    (addr, ids)
}

#[test]
fn a_detached_upstream_holds_nothing() {
    let source = Upstream::from_endpoint(None);
    assert!(matches!(source, Upstream::Detached));
    assert!(
        source
            .fetch(ObjectId::from_bytes([1; 32]))
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_socket_upstream_reads_through_to_the_tier_above() {
    let (addr, ids) = serve_memory(&[b"hello"]);
    let source = Upstream::from_endpoint(Some(&Endpoint::Tcp(addr)));
    assert!(matches!(source, Upstream::Socket(_)));
    let bytes = source.fetch(ids[0]).unwrap().unwrap();
    assert_eq!(&*bytes, b"hello");
}

#[test]
fn bind_warms_from_the_snapshot_file() {
    let (addr, ids) = serve_memory(&[b"one", b"two"]);
    let snapshot_file = scratch("node").join("warm.ids");
    snapshot::write_ids(&snapshot_file, &ids).unwrap();

    let config = DaemonConfig::new(scratch("node-data"))
        .with_listen(Endpoint::Tcp(([127, 0, 0, 1], 0).into()))
        .with_upstream(Endpoint::Tcp(addr))
        .with_snapshot(&snapshot_file);
    let daemon = Daemon::bind(&config).unwrap();

    assert_eq!(daemon.warmed().fetched, 2);
    assert_eq!(daemon.warmed().missing, 0);
    assert!(daemon.listening().starts_with("tcp:127.0.0.1:"));
}

#[test]
fn a_leaf_warms_nothing_even_with_a_snapshot() {
    let (_addr, ids) = serve_memory(&[b"one", b"two"]);
    let snapshot_file = scratch("leaf").join("warm.ids");
    snapshot::write_ids(&snapshot_file, &ids).unwrap();

    let config = DaemonConfig::new(scratch("leaf-data"))
        .with_listen(Endpoint::Tcp(([127, 0, 0, 1], 0).into()))
        .with_snapshot(&snapshot_file);
    let daemon = Daemon::bind(&config).unwrap();

    assert_eq!(daemon.warmed().fetched, 0);
    assert_eq!(daemon.warmed().missing, 2);
}

#[test]
fn bind_with_no_snapshot_warms_nothing() {
    let config = DaemonConfig::new(scratch("cold-data"))
        .with_listen(Endpoint::Tcp(([127, 0, 0, 1], 0).into()));
    let daemon = Daemon::bind(&config).unwrap();
    assert_eq!(daemon.warmed().fetched, 0);
    assert_eq!(daemon.warmed().missing, 0);
}

#[cfg(unix)]
#[test]
fn bind_on_a_unix_socket_reports_its_path() {
    let path = scratch("unix").join("daemon.sock");
    let config = DaemonConfig::new(scratch("unix-data")).with_listen(Endpoint::Unix(path.clone()));
    let daemon = Daemon::bind(&config).unwrap();
    assert_eq!(daemon.listening(), format!("unix:{}", path.display()));
}

#[test]
fn a_bind_on_a_taken_tcp_address_is_a_transport_error() {
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = taken.local_addr().unwrap();
    let config = DaemonConfig::new(scratch("conflict-data")).with_listen(Endpoint::Tcp(addr));
    assert!(matches!(
        Daemon::bind(&config),
        Err(crate::Error::Transport(_))
    ));
}

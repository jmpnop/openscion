//! Unit tests for endpoint parsing, display, and dialing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::TcpListener;
use std::sync::Arc;

use super::Endpoint;
use crate::ObjectSource;
use crate::backing::ObjectCache;
use crate::store::Store;
use crate::test_support::{MemorySource, blob_id, scratch};

#[test]
fn a_tcp_endpoint_parses_and_displays() {
    let endpoint: Endpoint = "tcp:127.0.0.1:7979".parse().unwrap();
    assert_eq!(endpoint, Endpoint::Tcp(([127, 0, 0, 1], 7979).into()));
    assert_eq!(endpoint.to_string(), "tcp:127.0.0.1:7979");
}

#[test]
fn a_tcp_endpoint_accepts_an_ipv6_address() {
    let endpoint: Endpoint = "tcp:[::1]:7979".parse().unwrap();
    assert_eq!(endpoint.to_string(), "tcp:[::1]:7979");
}

#[cfg(unix)]
#[test]
fn a_unix_endpoint_parses_and_displays() {
    use std::path::PathBuf;

    let endpoint: Endpoint = "unix:/run/openscion/d.sock".parse().unwrap();
    assert_eq!(
        endpoint,
        Endpoint::Unix(PathBuf::from("/run/openscion/d.sock"))
    );
    assert_eq!(endpoint.to_string(), "unix:/run/openscion/d.sock");
}

#[cfg(unix)]
#[test]
fn a_unix_endpoint_needs_a_path() {
    let error = "unix:".parse::<Endpoint>().unwrap_err();
    assert_eq!(error.to_string(), "a unix endpoint needs a path");
}

#[test]
fn a_malformed_endpoint_is_an_error() {
    assert!("".parse::<Endpoint>().is_err());
    assert!("http:x".parse::<Endpoint>().is_err());
    assert!("tcp".parse::<Endpoint>().is_err());
    let error = "tcp:not-an-addr".parse::<Endpoint>().unwrap_err();
    assert!(error.to_string().contains("not a TCP socket address"));
}

/// A disk-backed store over `source`, rooted at a fresh cache directory.
fn store(name: &str, source: MemorySource) -> Arc<Store<MemorySource, ObjectCache>> {
    Arc::new(Store::open(&scratch(name), source, 1 << 20).unwrap())
}

#[test]
fn a_tcp_endpoint_dials_the_server_it_names() {
    // A server holds an object; the endpoint naming its address dials a source
    // that reads it over the wire.
    let source = MemorySource::new();
    let id = source.add(b"dialed over tcp");
    let served = store("dial-tcp", source);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let _ = crate::transport::serve_tcp(&listener, &served);
    });

    let dialed = Endpoint::Tcp(addr).dial();
    assert_eq!(&*dialed.fetch(id).unwrap().unwrap(), b"dialed over tcp");
    assert_eq!(dialed.fetch(blob_id(b"absent")).unwrap(), None);
}

#[cfg(unix)]
#[test]
fn a_unix_endpoint_dials_the_server_it_names() {
    use std::os::unix::net::UnixListener;

    let source = MemorySource::new();
    let id = source.add(b"dialed over unix");
    let served = store("dial-unix", source);

    let dir = scratch("dial-unix-socket");
    let path = dir.join("tier.sock");
    let listener = UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        let _ = crate::transport::serve_unix(&listener, &served);
    });

    let dialed = Endpoint::Unix(path).dial();
    assert_eq!(&*dialed.fetch(id).unwrap().unwrap(), b"dialed over unix");
}

#[test]
fn binding_a_tcp_endpoint_reports_the_resolved_address() {
    let bound = Endpoint::Tcp("127.0.0.1:0".parse().unwrap())
        .bind()
        .unwrap();
    // Port 0 is resolved to the port the OS chose.
    assert!(bound.listening().starts_with("tcp:127.0.0.1:"));
    assert!(!bound.listening().ends_with(":0"));
}

#[test]
fn binding_a_taken_tcp_address_is_an_io_error() {
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = taken.local_addr().unwrap();
    assert!(Endpoint::Tcp(addr).bind().is_err());
}

#[test]
fn a_bound_tcp_listener_serves_an_object_service() {
    let source = MemorySource::new();
    let id = source.add(b"served via bind");
    let served = store("bind-tcp", source);

    let bound = Endpoint::Tcp("127.0.0.1:0".parse().unwrap())
        .bind()
        .unwrap();
    let addr: std::net::SocketAddr = bound
        .listening()
        .trim_start_matches("tcp:")
        .parse()
        .unwrap();
    std::thread::spawn(move || {
        let _ = bound.serve(&served);
    });

    let client = Endpoint::Tcp(addr).dial();
    assert_eq!(&*client.fetch(id).unwrap().unwrap(), b"served via bind");
    assert_eq!(client.fetch(blob_id(b"absent")).unwrap(), None);
}

#[cfg(unix)]
#[test]
fn binding_a_unix_endpoint_reports_the_path_and_serves() {
    let source = MemorySource::new();
    let id = source.add(b"served via unix bind");
    let served = store("bind-unix", source);

    let dir = scratch("bind-unix-socket");
    let path = dir.join("tier.sock");
    let endpoint = Endpoint::Unix(path.clone());
    let bound = endpoint.bind().unwrap();
    assert_eq!(bound.listening(), format!("unix:{}", path.display()));
    std::thread::spawn(move || {
        let _ = bound.serve(&served);
    });

    let client = Endpoint::Unix(path).dial();
    assert_eq!(
        &*client.fetch(id).unwrap().unwrap(),
        b"served via unix bind"
    );
    assert_eq!(client.fetch(blob_id(b"absent")).unwrap(), None);
}

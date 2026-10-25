//! Unit tests for the framed transport.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::TcpListener;
use std::sync::Arc;

use super::{SocketSource, read_frame, serve_tcp, write_frame};
use crate::ObjectSource;
use crate::backing::ObjectCache;
use crate::store::Store;
use crate::test_support::{MemorySource, blob_id, scratch};

/// A disk-backed store over `source`, rooted at a fresh cache directory.
fn store(name: &str, source: MemorySource) -> Arc<Store<MemorySource, ObjectCache>> {
    Arc::new(Store::open(&scratch(name), source, 1 << 20).unwrap())
}

/// Binds a TCP server for `store` on a loopback port and returns its address.
fn serve(store: Arc<Store<MemorySource, ObjectCache>>) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let _ = serve_tcp(&listener, &store);
    });
    addr
}

#[test]
fn frames_round_trip_and_end_cleanly() {
    let mut buffer = Vec::new();
    write_frame(&mut buffer, 1, b"a request").unwrap();
    write_frame(&mut buffer, 2, &[]).unwrap();

    let mut reader = &buffer[..];
    assert_eq!(
        read_frame(&mut reader).unwrap(),
        Some((1, b"a request".to_vec()))
    );
    assert_eq!(read_frame(&mut reader).unwrap(), Some((2, Vec::new())));
    // A clean end of stream is None, not an error.
    assert_eq!(read_frame(&mut reader).unwrap(), None);
}

#[test]
fn a_truncated_frame_is_a_protocol_error() {
    // A tag and a partial length: the stream ends mid-frame.
    let partial = [7u8, 0, 0, 0];
    let mut reader = &partial[..];
    assert!(matches!(
        read_frame(&mut reader),
        Err(crate::Error::Protocol(_))
    ));
}

#[test]
fn an_oversized_frame_is_rejected() {
    // A length past the maximum must be refused before allocating it.
    let mut header = vec![9u8];
    header.extend_from_slice(&u64::MAX.to_be_bytes());
    let mut reader = &header[..];
    assert!(matches!(
        read_frame(&mut reader),
        Err(crate::Error::Protocol(_))
    ));
}

#[test]
fn serves_found_and_absent_objects_over_tcp() {
    let source = MemorySource::new();
    let id = source.add(b"the object");
    let addr = serve(store("tcp", source));

    let client = SocketSource::tcp(addr);
    assert_eq!(&*client.fetch(id).unwrap().unwrap(), b"the object");
    // An id no tier holds comes back absent, not an error.
    assert_eq!(client.fetch(blob_id(b"absent")).unwrap(), None);
}

#[test]
fn a_store_can_front_a_server_over_the_wire() {
    // The tier above is a server; a near store reads through to it over TCP.
    let far = MemorySource::new();
    let id = far.add(b"shared object");
    let addr = serve(store("far", far));

    let near = Store::open(&scratch("near"), SocketSource::tcp(addr), 1 << 20).unwrap();
    assert_eq!(&*near.get(id).unwrap(), b"shared object");
    // Cached now: it serves locally and reports the id in its snapshot.
    assert_eq!(near.snapshot(), vec![id]);
}

#[test]
fn an_upstream_error_is_relayed_to_the_client() {
    let source = MemorySource::new();
    // The tier above serves bytes that do not match the id, so the store's get
    // fails with a corruption error, which the server relays over the wire.
    let id = source.add_corrupt(b"claimed", b"tampered");
    let addr = serve(store("err", source));

    let client = SocketSource::tcp(addr);
    assert!(matches!(client.fetch(id), Err(crate::Error::Upstream(_))));
}

#[test]
fn a_shed_is_relayed_as_unavailable_not_a_generic_error() {
    use crate::BoundedSource;

    // The tier above holds the object, but the store's upstream is bounded at
    // zero, so a cold miss sheds before reaching it.
    let upstream = MemorySource::new();
    let id = upstream.add(b"would be served");
    let node =
        Arc::new(Store::open(&scratch("shed"), BoundedSource::new(upstream, 0), 1 << 20).unwrap());

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let _ = serve_tcp(&listener, &node);
    });

    // The shed crosses the wire as Unavailable, distinct from a generic upstream
    // error, so a lower tier can tell a shed from a failure.
    let client = SocketSource::tcp(addr);
    assert!(matches!(
        client.fetch(id),
        Err(crate::Error::Unavailable(_))
    ));
}

#[test]
fn an_unexpected_response_tag_is_a_protocol_error() {
    // A rogue server that replies with a tag the client does not expect.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            // Read the request frame (tag + 8-byte len + 32-byte id), then reply
            // with an unknown tag.
            let mut discard = [0u8; 1 + 8 + 32];
            let _ = std::io::Read::read_exact(&mut stream, &mut discard);
            let _ = write_frame(&mut stream, 99, b"junk");
        }
    });

    let client = SocketSource::tcp(addr);
    assert!(matches!(
        client.fetch(blob_id(b"x")),
        Err(crate::Error::Protocol(_))
    ));
}

#[test]
fn a_connection_refused_is_a_transport_error() {
    // Bind then drop a listener so nothing accepts at the address.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let client = SocketSource::tcp(addr);
    assert!(matches!(
        client.fetch(blob_id(b"x")),
        Err(crate::Error::Transport(_))
    ));
}

#[test]
fn the_peer_closing_before_replying_is_a_protocol_error() {
    // A server that reads the whole request, then closes without replying, so
    // the client's write succeeds and its read sees a clean end of stream.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request = [0u8; 1 + 8 + 32];
            let _ = std::io::Read::read_exact(&mut stream, &mut request);
            drop(stream);
        }
    });
    let client = SocketSource::tcp(addr);
    assert!(matches!(
        client.fetch(blob_id(b"x")),
        Err(crate::Error::Protocol(_))
    ));
}

#[cfg(unix)]
#[test]
fn serves_objects_over_a_unix_socket() {
    use std::os::unix::net::UnixListener;

    use super::serve_unix;

    let source = MemorySource::new();
    let id = source.add(b"unix object");
    let node = store("unix", source);

    let dir = scratch("unix-socket");
    let path = dir.join("daemon.sock");
    let listener = UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        let _ = serve_unix(&listener, &node);
    });

    let client = SocketSource::unix(&path);
    assert_eq!(&*client.fetch(id).unwrap().unwrap(), b"unix object");
    assert_eq!(client.fetch(blob_id(b"absent")).unwrap(), None);
}

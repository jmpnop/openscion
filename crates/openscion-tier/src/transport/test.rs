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
fn a_tier_forwards_a_non_object_rpc_to_the_tier_above() {
    use openscion_core::AncestorResult;
    use openscion_core::rpc::{Request, Response};

    use crate::{Forwarding, ObjectService};

    // The origin — the server's role — answers RPCs with a fixed typed response.
    struct Origin;
    impl ObjectService for Origin {
        fn get(&self, id: openscion_core::ObjectId) -> crate::Result<Arc<[u8]>> {
            Err(crate::Error::NotFound(id))
        }
        fn rpc(&self, _request: Request) -> crate::Result<Response> {
            Ok(Response::Ancestor(AncestorResult::new(true)))
        }
    }
    let origin = Arc::new(Origin);
    let origin_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin_addr = origin_listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let _ = serve_tcp(&origin_listener, &origin);
    });

    // A forwarding tier serves objects from a store and relays RPCs to origin.
    let forwarding = Arc::new(Forwarding::new(
        store("forwarding", MemorySource::new()),
        Some(SocketSource::tcp(origin_addr)),
    ));
    let tier_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let tier_addr = tier_listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let _ = serve_tcp(&tier_listener, &forwarding);
    });

    // A client talks only to the forwarding tier; its RPC reaches the origin and
    // the answer is relayed back.
    let client = SocketSource::tcp(tier_addr);
    assert_eq!(
        client.rpc(&Request::GetRef("x".to_owned())).unwrap(),
        Response::Ancestor(AncestorResult::new(true))
    );
}

#[test]
fn a_forwarding_tier_with_no_upstream_declines() {
    use openscion_core::rpc::{Request, Response};

    use crate::{Forwarding, ObjectService};

    let forwarding = Forwarding::new(store("forwarding-detached", MemorySource::new()), None);
    match forwarding.rpc(Request::GetRef("x".to_owned())).unwrap() {
        Response::Error(body) => assert_eq!(body.code, "Unavailable"),
        other => panic!("expected an unavailable error, got {other:?}"),
    }
}

#[test]
fn an_rpc_to_a_tier_that_declines_returns_an_unsupported_error() {
    use openscion_core::rpc::{Request, Response};

    // A plain Store serves objects but declines forwarded RPCs by default.
    let addr = serve(store("rpc-decline", MemorySource::new()));
    let client = SocketSource::tcp(addr);
    match client
        .rpc(&Request::GetRef("refs/heads/main".to_owned()))
        .unwrap()
    {
        Response::Error(body) => assert_eq!(body.code, "Unsupported"),
        other => panic!("expected an error response, got {other:?}"),
    }
}

#[test]
fn an_rpc_round_trips_a_typed_response_over_the_wire() {
    use openscion_core::AncestorResult;
    use openscion_core::rpc::{Request, Response};

    use crate::ObjectService;

    // A service that answers any RPC with a fixed typed response, to prove the
    // envelope round-trips over the framed wire.
    struct Yes;
    impl ObjectService for Yes {
        fn get(&self, id: openscion_core::ObjectId) -> crate::Result<Arc<[u8]>> {
            Err(crate::Error::NotFound(id))
        }
        fn rpc(&self, _request: Request) -> crate::Result<Response> {
            Ok(Response::Ancestor(AncestorResult::new(true)))
        }
    }

    let service = Arc::new(Yes);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let _ = serve_tcp(&listener, &service);
    });

    let client = SocketSource::tcp(addr);
    let response = client.rpc(&Request::GetRef("anything".to_owned())).unwrap();
    assert_eq!(response, Response::Ancestor(AncestorResult::new(true)));
}

#[test]
fn fetches_a_batch_of_found_and_absent_objects() {
    let mem = MemorySource::new();
    let alpha = mem.add(b"alpha");
    let gamma = mem.add(b"gamma");
    let absent = blob_id(b"missing");
    let addr = serve(store("batch", mem));

    let client = SocketSource::tcp(addr);
    // One request, three replies in order: found, absent, found.
    let results = client.fetch_many(&[alpha, absent, gamma]).unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].as_deref(), Some(&b"alpha"[..]));
    assert_eq!(results[1], None);
    assert_eq!(results[2].as_deref(), Some(&b"gamma"[..]));
    // An empty batch needs no round-trip.
    assert_eq!(client.fetch_many(&[]).unwrap().len(), 0);
}

#[test]
fn ids_from_requires_a_whole_number_of_ids() {
    use super::ids_from;

    assert_eq!(ids_from(&[]).unwrap().len(), 0);
    assert_eq!(ids_from(&[7u8; 64]).unwrap().len(), 2);
    // 40 bytes is neither one id nor two: a protocol error.
    assert!(matches!(
        ids_from(&[0u8; 40]),
        Err(crate::Error::Protocol(_))
    ));
}

#[test]
fn a_batch_reuses_a_pooled_connection() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::serve_connection;

    let mem = MemorySource::new();
    let id = mem.add(b"batched");
    let served = store("batch-reuse", mem);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let accepts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&accepts);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            counter.fetch_add(1, Ordering::SeqCst);
            let served = Arc::clone(&served);
            std::thread::spawn(move || {
                let _ = serve_connection(stream, &*served);
            });
        }
    });

    let client = SocketSource::tcp(addr);
    assert_eq!(
        client.fetch_many(&[id]).unwrap()[0].as_deref(),
        Some(&b"batched"[..])
    );
    assert_eq!(
        client.fetch_many(&[id]).unwrap()[0].as_deref(),
        Some(&b"batched"[..])
    );
    // Two batches reused one connection.
    assert_eq!(accepts.load(Ordering::SeqCst), 1);
}

#[test]
fn reuses_a_pooled_connection_across_fetches() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::serve_connection;

    let mem = MemorySource::new();
    let id = mem.add(b"pooled object");
    let served = store("pool-reuse", mem);

    // A server that counts accepted connections and serves each one fully.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let accepts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&accepts);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            counter.fetch_add(1, Ordering::SeqCst);
            let served = Arc::clone(&served);
            std::thread::spawn(move || {
                let _ = serve_connection(stream, &*served);
            });
        }
    });

    let client = SocketSource::tcp(addr);
    for _ in 0..3 {
        assert_eq!(&*client.fetch(id).unwrap().unwrap(), b"pooled object");
    }
    // Three fetches reused one pooled connection: the server accepted once.
    assert_eq!(accepts.load(Ordering::SeqCst), 1);
}

#[test]
fn a_stale_pooled_connection_is_retried() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{TAG_FOUND, read_frame, write_frame};

    // A server that replies to one request then closes the connection, as a
    // server dropping an idle connection would. It keeps accepting new ones.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let accepts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&accepts);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            counter.fetch_add(1, Ordering::SeqCst);
            if read_frame(&mut stream).ok().flatten().is_some() {
                let _ = write_frame(&mut stream, TAG_FOUND, b"after stale");
            }
            // `stream` drops here: the connection closes after its one reply.
        }
    });

    let client = SocketSource::tcp(addr);
    let id = blob_id(b"anything");
    // The first fetch dials and pools the connection; the server has closed it.
    assert_eq!(&*client.fetch(id).unwrap().unwrap(), b"after stale");
    // The second takes the stale pooled connection, fails, and retries fresh.
    assert_eq!(&*client.fetch(id).unwrap().unwrap(), b"after stale");
    // One accept per fetch: the second did not silently fail on the stale one.
    assert_eq!(accepts.load(Ordering::SeqCst), 2);
}

#[test]
fn concurrent_fetches_through_a_shared_source_all_succeed() {
    let mem = MemorySource::new();
    let id = mem.add(b"shared object");
    let addr = serve(store("pool-concurrent", mem));

    // One source, shared by value across threads; its pool is shared behind an
    // Arc, so concurrent fetches check connections in and out without races.
    let client = SocketSource::tcp(addr);
    let mut handles = Vec::new();
    for _ in 0..8 {
        let client = client.clone();
        handles.push(std::thread::spawn(move || {
            client.fetch(id).unwrap().unwrap()
        }));
    }
    for handle in handles {
        assert_eq!(&*handle.join().unwrap(), b"shared object");
    }
}

#[test]
fn serves_a_bare_object_service_not_only_a_store() {
    // The transport serves any ObjectService, not only a cache Store: an origin
    // that holds authoritative objects — the server — serves the same wire from
    // its own store, with no tier above and nothing to shed.
    use openscion_core::ObjectId;

    use crate::{Error, ObjectService};

    struct OneObject {
        id: ObjectId,
        bytes: Arc<[u8]>,
    }
    impl ObjectService for OneObject {
        fn get(&self, id: ObjectId) -> crate::Result<Arc<[u8]>> {
            if id == self.id {
                Ok(Arc::clone(&self.bytes))
            } else {
                Err(Error::NotFound(id))
            }
        }
    }

    let id = blob_id(b"authoritative");
    let service = Arc::new(OneObject {
        id,
        bytes: Arc::from(&b"authoritative"[..]),
    });

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let _ = serve_tcp(&listener, &service);
    });

    let client = SocketSource::tcp(addr);
    assert_eq!(&*client.fetch(id).unwrap().unwrap(), b"authoritative");
    // A miss is absent, mapped from the service's NotFound.
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

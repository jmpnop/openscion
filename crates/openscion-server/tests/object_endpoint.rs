//! End-to-end test of the server's framed object endpoint.
//!
//! An object uploaded over the HTTP API is read back over the framed wire with a
//! tier client, proving a regional cache or node daemon can read through the
//! server as the origin of the read-through chain.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use openscion_core::{FrameWriter, ObjectKind, object_id};
use openscion_server::{Server, ServerConfig};
use openscion_tier::{Endpoint, ObjectSource};
use tokio::sync::oneshot;

#[tokio::test]
async fn an_object_uploaded_over_http_is_served_over_the_framed_wire() {
    // A server with both its HTTP API and a framed object endpoint, in memory.
    let config = ServerConfig::new("127.0.0.1:0".parse().unwrap())
        .with_object_listen(Endpoint::Tcp("127.0.0.1:0".parse().unwrap()));
    let server = Server::bind(&config).await.unwrap();
    let base = format!("http://{}", server.local_addr());
    let object = server.object_addr().unwrap().to_owned();

    let (stop, stopped) = oneshot::channel::<()>();
    let serving = tokio::spawn(async move {
        server
            .serve(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });

    // Upload a blob over the HTTP API, exactly as the client does: a frame
    // stream of (id, bytes) posted to /v1/blobs.
    let payload = b"content-addressed payload";
    let id = object_id(ObjectKind::Blob, payload);
    let mut frames = FrameWriter::new();
    frames.push(id, payload);
    let body = frames.finish();
    let upload = base.clone();
    tokio::task::spawn_blocking(move || {
        ureq::post(format!("{upload}/v1/blobs"))
            .header("content-type", "application/octet-stream")
            .send(&body[..])
            .unwrap();
    })
    .await
    .unwrap();

    // Read it back over the framed wire with a tier client, as a cache would.
    let endpoint: Endpoint = object.parse().unwrap();
    let (found, absent) = tokio::task::spawn_blocking(move || {
        let source = endpoint.dial();
        let found = source.fetch(id).unwrap();
        let absent = source
            .fetch(object_id(ObjectKind::Blob, b"never uploaded"))
            .unwrap();
        (found, absent)
    })
    .await
    .unwrap();

    // The framed client receives the exact uploaded bytes, and absent for a miss.
    assert_eq!(found.as_deref(), Some(&payload[..]));
    assert!(absent.is_none());

    stop.send(()).unwrap();
    serving.await.unwrap();
}

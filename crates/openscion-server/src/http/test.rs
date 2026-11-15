//! Tests for the HTTP surface, over a real socket.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use openscion_core::{PROTOCOL_VERSION, ServerInfo};
use openscion_tier::Endpoint;
use tokio::sync::oneshot;

use super::Server;
use crate::{Error, ServerConfig};

async fn start() -> (String, oneshot::Sender<()>, tokio::task::JoinHandle<()>) {
    let server = Server::bind(&ServerConfig::new("127.0.0.1:0".parse().unwrap()))
        .await
        .unwrap();
    let base = format!("http://{}", server.local_addr());
    let (stop, stopped) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        server
            .serve(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    (base, stop, task)
}

async fn get(url: String) -> (u16, String) {
    tokio::task::spawn_blocking(move || {
        let mut response = ureq::get(&url).call().unwrap();
        let status = response.status().as_u16();
        (status, response.body_mut().read_to_string().unwrap())
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn healthz_reports_ok() {
    let (base, stop, task) = start().await;
    assert_eq!(get(format!("{base}/healthz")).await, (200, "ok".to_owned()));
    stop.send(()).unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn version_reports_the_package_and_protocol_versions() {
    let (base, stop, task) = start().await;
    let (status, body) = get(format!("{base}/v1/version")).await;
    assert_eq!(status, 200);
    assert_eq!(
        serde_json::from_str::<ServerInfo>(&body).unwrap(),
        ServerInfo::new(env!("CARGO_PKG_VERSION"), PROTOCOL_VERSION)
    );
    stop.send(()).unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn binding_an_address_in_use_fails_with_bind() {
    let first = Server::bind(&ServerConfig::new("127.0.0.1:0".parse().unwrap()))
        .await
        .unwrap();
    let taken = ServerConfig::new(first.local_addr());
    let error = Server::bind(&taken).await.unwrap_err();
    assert!(matches!(error, Error::Bind { addr, .. } if addr == first.local_addr()));
}

#[tokio::test]
async fn a_configured_object_endpoint_is_bound_and_reported() {
    let config = ServerConfig::new("127.0.0.1:0".parse().unwrap())
        .with_object_listen(Endpoint::Tcp("127.0.0.1:0".parse().unwrap()));
    let server = Server::bind(&config).await.unwrap();
    let object = server.object_addr().unwrap();
    assert!(object.starts_with("tcp:127.0.0.1:"));
    // A resolved, non-zero port: the endpoint is actually bound.
    assert!(!object.ends_with(":0"));
}

#[tokio::test]
async fn without_an_object_endpoint_none_is_reported() {
    let server = Server::bind(&ServerConfig::new("127.0.0.1:0".parse().unwrap()))
        .await
        .unwrap();
    assert!(server.object_addr().is_none());
}

#[tokio::test]
async fn binding_a_taken_object_endpoint_fails_with_object_listen() {
    // Hold a port, then ask the server to serve its framed endpoint there.
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = taken.local_addr().unwrap();
    let config =
        ServerConfig::new("127.0.0.1:0".parse().unwrap()).with_object_listen(Endpoint::Tcp(addr));
    let error = Server::bind(&config).await.unwrap_err();
    assert!(
        matches!(error, Error::ObjectListen { endpoint, .. } if endpoint == Endpoint::Tcp(addr))
    );
}

#[tokio::test]
async fn binding_with_an_unusable_data_dir_fails_with_store() {
    // A data directory that is actually a file: the object store cannot create
    // its subdirectory there, so opening the stores fails before binding.
    let file = std::env::temp_dir().join(format!("openscion-notdir-{}", std::process::id()));
    std::fs::write(&file, b"i am a file").unwrap();
    let config = ServerConfig::new("127.0.0.1:0".parse().unwrap()).with_data_dir(&file);
    let error = Server::bind(&config).await.unwrap_err();
    assert!(matches!(error, Error::Store(_)));
    let _ = std::fs::remove_file(&file);
}

//! Unit tests for server configuration parsing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use clap::Parser;
use openscion_tier::Endpoint;

use super::{DEFAULT_LISTEN, ServerConfig};

#[test]
fn the_default_listens_on_loopback() {
    assert_eq!(
        ServerConfig::default().listen,
        DEFAULT_LISTEN.parse().unwrap()
    );
}

#[test]
fn listen_is_read_from_the_command_line() {
    let config =
        ServerConfig::try_parse_from(["openscion-server", "--listen", "0.0.0.0:9000"]).unwrap();
    assert_eq!(config.listen, "0.0.0.0:9000".parse().unwrap());
}

#[test]
fn an_invalid_listen_address_is_rejected() {
    let parsed = ServerConfig::try_parse_from(["openscion-server", "--listen", "not-an-address"]);
    assert!(parsed.is_err());
}

#[test]
fn the_data_directory_defaults_to_none() {
    assert!(ServerConfig::default().data_dir.is_none());
}

#[test]
fn the_data_directory_is_read_from_the_command_line() {
    let config =
        ServerConfig::try_parse_from(["openscion-server", "--data-dir", "/srv/openscion"]).unwrap();
    assert_eq!(
        config.data_dir.as_deref(),
        Some(std::path::Path::new("/srv/openscion"))
    );
}

#[test]
fn with_data_dir_sets_the_directory() {
    let config = ServerConfig::default().with_data_dir("/var/data");
    assert_eq!(
        config.data_dir.as_deref(),
        Some(std::path::Path::new("/var/data"))
    );
}

#[test]
fn the_object_endpoint_defaults_to_none() {
    assert!(ServerConfig::default().object_listen.is_none());
}

#[test]
fn a_tcp_object_endpoint_is_read_from_the_command_line() {
    let config =
        ServerConfig::try_parse_from(["openscion-server", "--object-listen", "tcp:127.0.0.1:7879"])
            .unwrap();
    assert_eq!(
        config.object_listen,
        Some(Endpoint::Tcp("127.0.0.1:7879".parse().unwrap()))
    );
}

#[cfg(unix)]
#[test]
fn a_unix_object_endpoint_is_read_from_the_command_line() {
    let config = ServerConfig::try_parse_from([
        "openscion-server",
        "--object-listen",
        "unix:/run/openscion/objects.sock",
    ])
    .unwrap();
    assert_eq!(
        config.object_listen,
        Some(Endpoint::Unix("/run/openscion/objects.sock".into()))
    );
}

#[test]
fn an_invalid_object_endpoint_is_rejected() {
    let parsed =
        ServerConfig::try_parse_from(["openscion-server", "--object-listen", "not-an-endpoint"]);
    assert!(parsed.is_err());
}

#[test]
fn with_object_listen_sets_the_endpoint() {
    let endpoint = Endpoint::Tcp("127.0.0.1:0".parse().unwrap());
    let config = ServerConfig::default().with_object_listen(endpoint.clone());
    assert_eq!(config.object_listen, Some(endpoint));
}

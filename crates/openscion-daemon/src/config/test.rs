//! Unit tests for endpoint parsing and the command-line configuration.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use clap::Parser;

use super::{DEFAULT_BUDGET_BYTES, DaemonConfig, Endpoint};

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

#[test]
fn only_a_data_dir_takes_every_default() {
    let config = DaemonConfig::try_parse_from(["openscion-daemon", "--data-dir", "/data"]).unwrap();
    assert_eq!(config.listen, super::DEFAULT_LISTEN.parse().unwrap());
    assert_eq!(config.data_dir, PathBuf::from("/data"));
    assert_eq!(config.budget_bytes, DEFAULT_BUDGET_BYTES);
    assert!(config.upstream.is_none());
    assert!(config.snapshot.is_none());
}

#[test]
fn every_flag_is_read() {
    let config = DaemonConfig::try_parse_from([
        "openscion-daemon",
        "--listen",
        "tcp:127.0.0.1:0",
        "--data-dir",
        "/data",
        "--budget-bytes",
        "4096",
        "--upstream",
        "tcp:10.0.0.1:7979",
        "--snapshot",
        "/data/warm.ids",
    ])
    .unwrap();
    assert_eq!(config.listen, Endpoint::Tcp(([127, 0, 0, 1], 0).into()));
    assert_eq!(config.budget_bytes, 4096);
    assert_eq!(
        config.upstream,
        Some(Endpoint::Tcp(([10, 0, 0, 1], 7979).into()))
    );
    assert_eq!(config.snapshot, Some(PathBuf::from("/data/warm.ids")));
}

#[test]
fn a_missing_data_dir_is_a_usage_error() {
    assert!(DaemonConfig::try_parse_from(["openscion-daemon"]).is_err());
}

#[test]
fn a_bogus_listen_is_a_usage_error() {
    assert!(
        DaemonConfig::try_parse_from([
            "openscion-daemon",
            "--data-dir",
            "/data",
            "--listen",
            "ftp:x"
        ])
        .is_err()
    );
}

#[test]
fn the_builders_set_each_field() {
    let config = DaemonConfig::new("/data")
        .with_listen(Endpoint::Tcp(([127, 0, 0, 1], 0).into()))
        .with_upstream("tcp:10.0.0.1:7979".parse().unwrap())
        .with_snapshot("/data/warm.ids");
    assert_eq!(config.listen, Endpoint::Tcp(([127, 0, 0, 1], 0).into()));
    assert_eq!(
        config.upstream,
        Some(Endpoint::Tcp(([10, 0, 0, 1], 7979).into()))
    );
    assert_eq!(config.snapshot, Some(PathBuf::from("/data/warm.ids")));
}

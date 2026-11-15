//! Unit tests for the command-line configuration.
//!
//! The endpoint type's own parse/display tests live with it in
//! `openscion-tier`; these exercise `DaemonConfig`'s flags, defaults, and
//! builders, including that a bogus endpoint is a usage error through clap.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use clap::Parser;

use super::{DEFAULT_BUDGET_BYTES, DEFAULT_UPSTREAM_CONCURRENCY, DaemonConfig, Endpoint};

#[test]
fn only_a_data_dir_takes_every_default() {
    let config = DaemonConfig::try_parse_from(["openscion-daemon", "--data-dir", "/data"]).unwrap();
    assert_eq!(config.listen, super::DEFAULT_LISTEN.parse().unwrap());
    assert_eq!(config.data_dir, PathBuf::from("/data"));
    assert_eq!(config.budget_bytes, DEFAULT_BUDGET_BYTES);
    assert!(config.upstream.is_none());
    assert_eq!(config.upstream_concurrency, DEFAULT_UPSTREAM_CONCURRENCY);
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
        "--upstream-concurrency",
        "8",
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
    assert_eq!(config.upstream_concurrency, 8);
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
        .with_upstream_concurrency(8)
        .with_snapshot("/data/warm.ids");
    assert_eq!(config.listen, Endpoint::Tcp(([127, 0, 0, 1], 0).into()));
    assert_eq!(
        config.upstream,
        Some(Endpoint::Tcp(([10, 0, 0, 1], 7979).into()))
    );
    assert_eq!(config.upstream_concurrency, 8);
    assert_eq!(config.snapshot, Some(PathBuf::from("/data/warm.ids")));
}

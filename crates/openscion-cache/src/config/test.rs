//! Unit tests for the command-line configuration.
//!
//! The endpoint type's own parse/display tests live with it in
//! `openscion-tier`; these exercise `CacheConfig`'s flags, defaults, and
//! builders, including that a missing or bogus endpoint is a usage error.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use clap::Parser;

use openscion_tier::Endpoint;

use super::{CacheConfig, DEFAULT_BUDGET_BYTES, DEFAULT_UPSTREAM_CONCURRENCY};

#[test]
fn only_the_upstream_takes_every_default() {
    let config =
        CacheConfig::try_parse_from(["openscion-cache", "--upstream", "tcp:10.0.0.1:7979"])
            .unwrap();
    assert_eq!(config.listen, super::DEFAULT_LISTEN.parse().unwrap());
    assert_eq!(config.upstream, Endpoint::Tcp(([10, 0, 0, 1], 7979).into()));
    assert_eq!(config.budget_bytes, DEFAULT_BUDGET_BYTES);
    assert_eq!(config.upstream_concurrency, DEFAULT_UPSTREAM_CONCURRENCY);
}

#[test]
fn every_flag_is_read() {
    let config = CacheConfig::try_parse_from([
        "openscion-cache",
        "--listen",
        "tcp:127.0.0.1:0",
        "--upstream",
        "tcp:10.0.0.1:7979",
        "--budget-bytes",
        "4096",
        "--upstream-concurrency",
        "8",
    ])
    .unwrap();
    assert_eq!(config.listen, Endpoint::Tcp(([127, 0, 0, 1], 0).into()));
    assert_eq!(config.upstream, Endpoint::Tcp(([10, 0, 0, 1], 7979).into()));
    assert_eq!(config.budget_bytes, 4096);
    assert_eq!(config.upstream_concurrency, 8);
}

#[test]
fn a_missing_upstream_is_a_usage_error() {
    assert!(CacheConfig::try_parse_from(["openscion-cache"]).is_err());
}

#[test]
fn a_bogus_listen_is_a_usage_error() {
    assert!(
        CacheConfig::try_parse_from([
            "openscion-cache",
            "--upstream",
            "tcp:10.0.0.1:7979",
            "--listen",
            "ftp:x",
        ])
        .is_err()
    );
}

#[test]
fn a_bogus_upstream_is_a_usage_error() {
    assert!(CacheConfig::try_parse_from(["openscion-cache", "--upstream", "ftp:x"]).is_err());
}

#[test]
fn the_builders_set_each_field() {
    let config = CacheConfig::new("tcp:10.0.0.1:7979".parse().unwrap())
        .with_listen(Endpoint::Tcp(([127, 0, 0, 1], 0).into()))
        .with_budget_bytes(4096)
        .with_upstream_concurrency(8);
    assert_eq!(config.listen, Endpoint::Tcp(([127, 0, 0, 1], 0).into()));
    assert_eq!(config.upstream, Endpoint::Tcp(([10, 0, 0, 1], 7979).into()));
    assert_eq!(config.budget_bytes, 4096);
    assert_eq!(config.upstream_concurrency, 8);
}

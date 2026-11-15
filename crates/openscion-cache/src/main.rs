//! The `openscion-cache` binary.
//!
//! Parses [`CacheConfig`] from the command line and environment, dials the tier
//! above, opens the in-memory store, binds the listener, and serves until a
//! shutdown signal. It persists nothing: the cache starts empty each time.

use std::process::ExitCode;

use clap::Parser;
use openscion_cache::{CacheConfig, RegionalCache, Result};

fn main() -> ExitCode {
    match run(&CacheConfig::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("openscion-cache: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(config: &CacheConfig) -> Result<()> {
    let cache = RegionalCache::bind(config)?;
    println!("openscion-cache listening on {}", cache.listening());
    cache.serve()
}

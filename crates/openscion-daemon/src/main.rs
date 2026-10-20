//! The `openscion-daemon` binary.
//!
//! Parses [`DaemonConfig`] from the command line and environment, opens the node
//! store, warms it from a snapshot, binds the listener, and serves until a
//! shutdown signal — then persists the snapshot.

use std::process::ExitCode;

use clap::Parser;
use openscion_daemon::{Daemon, DaemonConfig, Result};

fn main() -> ExitCode {
    match run(&DaemonConfig::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("openscion-daemon: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(config: &DaemonConfig) -> Result<()> {
    let daemon = Daemon::bind(config)?;
    println!("openscion-daemon listening on {}", daemon.listening());
    daemon.serve()
}

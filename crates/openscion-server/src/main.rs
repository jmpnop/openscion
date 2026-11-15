//! The `openscion-server` binary.
//!
//! Parses [`ServerConfig`] from the command line and environment, binds, and
//! serves until Ctrl-C or SIGTERM.

use std::process::ExitCode;

use clap::Parser;
use openscion_server::{Result, Server, ServerConfig, shutdown_signal};

#[tokio::main]
async fn main() -> ExitCode {
    match run(ServerConfig::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("openscion-server: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(config: ServerConfig) -> Result<()> {
    let server = Server::bind(&config).await?;
    println!(
        "openscion-server listening on http://{}",
        server.local_addr()
    );
    if let Some(object) = server.object_addr() {
        println!("openscion-server serving framed objects on {object}");
    }
    server.serve(shutdown_signal()).await
}

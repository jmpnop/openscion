//! The OpenScion server: the authoritative tier of the system.
//!
//! In the architecture (`docs/specs/openscion-architecture.md`) the server owns
//! the changeset log, the refs, and the derived-data indexes, and is the only
//! component that creates changesets or moves refs. It serves the
//! object, changeset, ref, query, and landing operations over an in-memory
//! [`openscion_core::Repository`]; a restart starts from an empty repository.
//!
//! # Endpoints
//!
//! | Method | Path | Response |
//! | --- | --- | --- |
//! | `GET` | `/healthz` | `200 ok` while the process is serving |
//! | `GET` | `/v1/version` | [`ServerInfo`] as JSON |
//! | `POST` | `/v1/blobs` | upload a batch of blob frames |
//! | `POST` | `/v1/objects` | fetch a batch of trees and blobs as frames |
//! | `POST` | `/v1/changesets` | create a changeset |
//! | `GET` | `/v1/changesets/{id}` | read a changeset |
//! | `GET` | `/v1/changesets/{id}/stat`, `/tree`, `/file` | query a snapshot |
//! | `POST` | `/v1/diff`, `/v1/log`, `/v1/ancestor`, `/v1/merge-base` | query history |
//! | `GET`, `POST` | `/v1/refs/{name}` | read a ref, or move it by compare-and-swap |
//! | `POST` | `/v1/rebase`, `/v1/revert`, `/v1/land` | move changes onto a new base |
//!
//! The `api` module documents each body and status code.
//!
//! # Entry points
//!
//! - [`ServerConfig`] — the command-line and environment configuration.
//! - [`Server::bind`] then [`Server::serve`] — listen and serve until a
//!   shutdown future resolves.
//! - [`router`] — the HTTP routes, for embedding or testing.
//!
//! # Example
//!
//! ```no_run
//! use openscion_server::{Server, ServerConfig};
//!
//! # async fn run() -> openscion_server::Result<()> {
//! let config = ServerConfig::default();
//! let server = Server::bind(&config).await?;
//! println!("listening on {}", server.local_addr());
//! server.serve(std::future::pending()).await
//! # }
//! ```
//!
//! # What is deliberately not here
//!
//! No source control logic. Hashing, validation, the log, and refs live in the
//! `openscion-core` engine; this crate translates HTTP to engine calls and back.

mod api;
mod config;
mod error;
mod http;

pub use api::{ApiError, MAX_BODY_BYTES, SharedRepository};
pub use config::{DEFAULT_LISTEN, ServerConfig};
pub use error::{Error, Result};
pub use http::{Server, router, shutdown_signal};
pub use openscion_core::ServerInfo;

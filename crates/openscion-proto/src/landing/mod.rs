//! The landing payloads: what a rebase, revert, or land carries.
//!
//! These are the JSON bodies of the server's write operations that move a
//! changeset onto a new base. A rebase or revert answers with the
//! [`RebaseResult`] it produced; a land answers with the [`LandResult`] naming
//! the ref's new target and version (`docs/specs/openscion-api.md`). A conflict
//! is not carried here — it comes back as an error body listing the paths.

mod types;

pub use types::{LandRequest, LandResult, RebaseRequest, RebaseResult, RevertRequest};

#[cfg(test)]
mod test;

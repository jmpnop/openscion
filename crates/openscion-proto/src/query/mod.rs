//! The query payloads: what a snapshot read or a history question carries.
//!
//! These are the JSON bodies of the server's read operations. Snapshot reads
//! name a changeset in the URL and answer with [`StatResult`] or
//! [`TreeListing`]; history questions name their changesets in the body and
//! answer with [`DiffResult`], [`LogPage`], [`AncestorResult`], or
//! [`MergeBaseResult`] (`docs/specs/openscion-api.md`). The bytes of a single
//! file do not travel here: they stream as a raw body, like batched objects.

mod types;

pub use types::{
    AncestorQuery, AncestorResult, DiffRequest, DiffResult, Entry, LogPage, LogRequest,
    MergeBaseQuery, MergeBaseResult, StatResult, TreeListing,
};

#[cfg(test)]
mod test;

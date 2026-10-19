//! The `opnscn` commands over the server.
//!
//! Each command talks to the server through a [`ServerClient`]; the
//! working-copy commands also touch the disk. They fall into three groups:
//!
//! The **working-copy** commands move whole snapshots to and from a directory:
//!
//! - **import** records a whole directory as the repository's first
//!   changeset and creates the ref.
//! - **commit** records what changed since the base changeset and moves the
//!   ref from the base to the new changeset by compare-and-swap. If someone
//!   else moved it first the swap fails; the changeset stays in the log
//!   unreferenced and the error is reported — `land` resolves such a race.
//! - **clone** reads the ref, fetches the trees breadth first — one request
//!   per level — and then the blobs in batches, verifying every id. With a
//!   path it fetches only the trees on the way to that directory and the
//!   files under it.
//! - **status** ([`status`]) reports what a commit would record — the added,
//!   modified, and deleted files — from local state alone, without touching
//!   the server. It and `commit` classify a file the same way, so they never
//!   disagree about what changed.
//! - **sparse** ([`sparse_show`], [`sparse_add`], [`sparse_remove`]) shows and
//!   edits which paths the working copy materializes: `add` fetches more of the
//!   tree from the server, `remove` deletes paths locally. `add` reuses the
//!   same fetch as `clone`.
//!
//! The **query** commands ([`query`]) read what the server already knows —
//! `stat`, `ls`, `cat`, `diff`, `log`, `ancestor`, and `merge-base` — and the
//! **landing** commands ([`landing`]) move changes onto a new base — `rebase`,
//! `revert`, and `land`. Both groups name their revisions by object id or ref
//! name, resolved through [`resolve_rev`].
//!
//! [`ServerClient`]: crate::ServerClient

mod clone;
mod commit;
mod import;
mod landing;
mod query;
mod sparse;
mod status;

pub(crate) use clone::clone;
pub(crate) use commit::commit;
pub(crate) use import::import;
pub(crate) use landing::{land, rebase, revert};
pub(crate) use query::{DEFAULT_LOG_LIMIT, ancestor, cat, diff, log, ls, merge_base, stat};
pub(crate) use sparse::{add as sparse_add, remove as sparse_remove, show as sparse_show};
pub(crate) use status::{StatusCode, status};

use openscion_core::{FrameWriter, ObjectId};

use crate::{Result, ServerClient};

/// Resolves a revision — a 64-hex object id, or a ref name the server resolves
/// to its tip — into a changeset id.
///
/// # Errors
///
/// Returns the client error when a ref lookup fails or the server is
/// unreachable.
pub(crate) fn resolve_rev(client: &ServerClient, rev: &str) -> Result<ObjectId> {
    match rev.parse::<ObjectId>() {
        Ok(id) => Ok(id),
        Err(_) => Ok(client.get_ref(rev)?.target),
    }
}

/// Upload batches are cut at this many bytes of blob data.
const UPLOAD_BATCH_BYTES: usize = 64 << 20;
/// The bytes a frame adds before its body: the id and a `u64` length.
const FRAME_HEADER: usize = ObjectId::LEN + 8;
/// Fetch requests name at most this many objects.
const FETCH_BATCH: usize = 4096;

/// Uploads blobs in batches of at most [`UPLOAD_BATCH_BYTES`].
fn upload(client: &ServerClient, blobs: &[(ObjectId, Vec<u8>)]) -> Result<usize> {
    upload_in_batches(client, blobs, UPLOAD_BATCH_BYTES)
}

/// Uploads blobs, starting a new request whenever the next blob would push
/// the current one past `batch_bytes`. Returns the number of requests sent.
fn upload_in_batches(
    client: &ServerClient,
    blobs: &[(ObjectId, Vec<u8>)],
    batch_bytes: usize,
) -> Result<usize> {
    let mut requests = 0;
    let mut frames = FrameWriter::new();
    for (id, bytes) in blobs {
        let frame = FRAME_HEADER + bytes.len();
        if !frames.is_empty() && frames.len() + frame > batch_bytes {
            requests += 1;
            client.upload_blobs(&std::mem::take(&mut frames).finish())?;
        }
        frames.push(*id, bytes);
    }
    if !frames.is_empty() {
        requests += 1;
        client.upload_blobs(&frames.finish())?;
    }
    Ok(requests)
}

#[cfg(test)]
mod test;

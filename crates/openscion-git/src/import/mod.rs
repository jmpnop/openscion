//! Walking a git repository's first-parent spine and importing it into an
//! OpenScion [`Repository`].

use std::collections::BTreeMap;
use std::path::Path;

use gix::ObjectId as GitOid;
use gix::bstr::{BStr, ByteSlice};
use openscion_core::ObjectId as OscId;
use openscion_core::{Change, CreateChangeset, Repository, UpdateRef};

use crate::convert::{self, Entry, Mapped, classify};
use crate::{Error, Result};

/// What an import produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Imported {
    /// The number of changesets created — one per first-parent spine commit.
    pub changesets: usize,
    /// The tip changeset the ref now points at.
    pub head: OscId,
    /// Submodule gitlink entries skipped across all imported commits.
    pub skipped_submodules: usize,
}

/// Imports the first-parent history of the git repository at `repo`, up to the
/// commit named by `start`, into `into`, pointing `ref_name` at the tip.
///
/// One OpenScion changeset is created per commit on the first-parent spine, in
/// order, each recording the tree changes from the previous spine commit. Each
/// carries the git author as `Name <email>` and the commit message, with the
/// git commit id, identities, and timestamps in its metadata.
///
/// # Errors
///
/// Returns [`Error::Git`] when the repository cannot be opened, `start` cannot
/// be resolved, or a git object cannot be read; [`Error::NonUtf8Path`] for a
/// git path that is not UTF-8; and [`Error::Core`] when the engine rejects a
/// path, object, or changeset.
pub fn import_first_parent(
    repo: &Path,
    into: &mut Repository,
    start: &str,
    ref_name: &str,
) -> Result<Imported> {
    let git = gix::open(repo).map_err(git_err)?;
    let tip = git
        .rev_parse_single(BStr::new(start))
        .map_err(git_err)?
        .detach();

    // The first-parent spine, tip to root, then reversed to import oldest first.
    let mut spine = vec![tip];
    let mut cursor = tip;
    loop {
        let commit = git.find_commit(cursor).map_err(git_err)?;
        let Some(parent) = commit.parent_ids().next() else {
            break;
        };
        cursor = parent.detach();
        spine.push(cursor);
    }
    spine.reverse();

    let mut previous_tree: BTreeMap<String, Entry> = BTreeMap::new();
    let mut parent_changeset: Option<OscId> = None;
    let mut skipped = 0;
    let mut head = OscId::from_bytes([0; 32]);
    for commit_id in spine.iter().copied() {
        let commit = git.find_commit(commit_id).map_err(git_err)?;
        let mut tree = BTreeMap::new();
        flatten(
            &git,
            &commit.tree().map_err(git_err)?,
            "",
            &mut tree,
            &mut skipped,
        )?;

        let diff = convert::diff(&previous_tree, &tree)?;
        let mut changes = Vec::with_capacity(diff.upserts.len() + diff.deletes.len());
        for upsert in diff.upserts {
            let bytes = git.find_object(upsert.oid).map_err(git_err)?.detach().data;
            let blob = into.put_blob(&bytes)?;
            changes.push(Change::Upsert {
                path: upsert.path,
                kind: upsert.kind,
                blob,
            });
        }
        for path in diff.deletes {
            changes.push(Change::Delete { path });
        }

        let request = request_for(&commit, commit_id, parent_changeset, changes)?;
        let created = into.create_changeset(request)?;
        head = created.id;
        parent_changeset = Some(created.id);
        previous_tree = tree;
    }

    let expected = into.get_ref(ref_name).ok().map(|value| value.target);
    into.update_ref(ref_name, UpdateRef::new(expected, head))?;

    Ok(Imported {
        changesets: spine.len(),
        head,
        skipped_submodules: skipped,
    })
}

/// Flattens a git tree into `out`, keyed by `/`-separated path, descending into
/// subtrees under `prefix`, recording files/executables/symlinks and counting
/// skipped submodule gitlinks.
fn flatten(
    git: &gix::Repository,
    tree: &gix::Tree<'_>,
    prefix: &str,
    out: &mut BTreeMap<String, Entry>,
    skipped: &mut usize,
) -> Result<()> {
    for entry in tree.iter() {
        let entry = entry.map_err(git_err)?;
        let inner = &entry.inner;
        let name = inner
            .filename
            .to_str()
            .map_err(|_| Error::NonUtf8Path(inner.filename.to_vec()))?;
        let oid: GitOid = inner.oid.to_owned();
        match classify(inner.mode.kind()) {
            Mapped::Tree => {
                let subtree = git.find_tree(oid).map_err(git_err)?;
                flatten(git, &subtree, &format!("{prefix}{name}/"), out, skipped)?;
            }
            Mapped::File(kind) => {
                out.insert(format!("{prefix}{name}"), Entry { kind, oid });
            }
            Mapped::Submodule => *skipped += 1,
        }
    }
    Ok(())
}

/// Builds the changeset request for a git `commit`: its first parent, author,
/// message, and git provenance metadata.
fn request_for(
    commit: &gix::Commit<'_>,
    commit_id: GitOid,
    parent: Option<OscId>,
    changes: Vec<Change>,
) -> Result<CreateChangeset> {
    let author = commit.author().map_err(git_err)?;
    let committer = commit.committer().map_err(git_err)?;
    let who = |signature: &gix::actor::SignatureRef<'_>| {
        format!(
            "{} <{}>",
            signature.name.to_str_lossy().trim(),
            signature.email.to_str_lossy().trim()
        )
    };
    let message = commit.message_raw_sloppy().to_str_lossy().into_owned();
    let mut request =
        CreateChangeset::new(parent.into_iter().collect(), changes, who(&author), message);
    request
        .metadata
        .insert("git.commit".to_owned(), commit_id.to_string());
    request
        .metadata
        .insert("git.author".to_owned(), who(&author));
    request
        .metadata
        .insert("git.committer".to_owned(), who(&committer));
    request
        .metadata
        .insert("git.author_time".to_owned(), author.time.to_owned());
    request
        .metadata
        .insert("git.commit_time".to_owned(), committer.time.to_owned());
    Ok(request)
}

/// Maps a `gix` failure to [`Error::Git`].
fn git_err(error: impl std::fmt::Display) -> Error {
    Error::Git(error.to_string())
}

#[cfg(test)]
mod test;

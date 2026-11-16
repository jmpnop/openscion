//! The framed object endpoint: serving the authoritative repository as a tier.
//!
//! The daemon and the regional cache serve the framed object RPCs
//! ([ADR 0004](../../../../docs/adr/0004-daemon-transport.md)) from a
//! read-through `Store`; the server is the origin of that read-through chain and
//! serves them from its authoritative [`Repository`](openscion_core::Repository)
//! instead. [`RepositoryObjects`] adapts the shared repository to the tier's
//! [`ObjectService`], which an [`Endpoint::bind`](openscion_tier::Endpoint::bind)
//! listener serves beside the async HTTP server.
//!
//! The endpoint is read-only: it answers object gets and nothing else, as the
//! wire does at every tier. As the origin it has no tier above and sheds
//! nothing — a get is found or absent.

use std::sync::Arc;

use openscion_core::rpc::{Request, Response};
use openscion_core::{
    AncestorResult, DiffRequest, DiffResult, Entry, ErrorBody, LandRequest, LandResult, LogPage,
    LogRequest, MergeBaseResult, ObjectId, RebaseResult, RepoPath, RevertRequest, StatResult,
    TreeListing,
};
use openscion_tier::ObjectService;

use crate::SharedRepository;
use crate::api::{read, write};

/// The server's authoritative objects as a tier [`ObjectService`].
///
/// It serves object bytes straight from the shared
/// [`Repository`](openscion_core::Repository), under the same read lock the HTTP
/// read handlers take, so a framed get and an HTTP request see one repository
/// under one lock discipline. As the origin of the read-through chain it has no
/// tier above and sheds nothing: a get is found or absent.
pub(crate) struct RepositoryObjects {
    repository: SharedRepository,
}

impl RepositoryObjects {
    /// Serves objects from `repository`.
    pub(crate) fn new(repository: SharedRepository) -> Self {
        Self { repository }
    }
}

impl ObjectService for RepositoryObjects {
    fn get(&self, id: ObjectId) -> openscion_tier::Result<Arc<[u8]>> {
        // Take the read lock only across the lookup: the guard drops at the end
        // of this statement, before the bytes are mapped onto the wire result.
        let found = read(&self.repository).object(id);
        map_object(found, id)
    }

    fn rpc(&self, request: Request) -> openscion_tier::Result<Response> {
        // The origin answers every forwarded mutable query from its engine. An
        // engine failure is a `Response::Error`, not a transport fault, so the
        // call succeeds at the tier even when the operation did not.
        Ok(dispatch(&self.repository, request))
    }
}

/// Answers a forwarded [`Request`] from the repository, turning an engine
/// failure into [`Response::Error`] with the same stable code and message the
/// HTTP API returns, so a caller reconstructs the error without a second
/// taxonomy.
fn dispatch(repository: &SharedRepository, request: Request) -> Response {
    match run(repository, request) {
        Ok(response) => response,
        Err(error) => Response::Error(ErrorBody::new(error.code(), error.to_string())),
    }
}

/// Runs one forwarded operation against the repository, mirroring the HTTP
/// handler of the same name — one engine call, its result shaped into the
/// matching [`Response`] — and letting an engine error propagate for
/// [`dispatch`] to render.
fn run(repository: &SharedRepository, request: Request) -> openscion_core::Result<Response> {
    Ok(match request {
        Request::CreateChangeset(request) => {
            Response::Changeset(write(repository).create_changeset(request)?)
        }
        Request::GetChangeset(id) => Response::ChangesetRecord(read(repository).changeset(id)?),
        Request::Stat { changeset, path } => stat(repository, changeset, &path)?,
        Request::ListTree {
            changeset,
            path,
            recursive,
        } => list_tree(repository, changeset, path.as_deref(), recursive)?,
        Request::ReadFile { changeset, path } => read_file(repository, changeset, &path)?,
        Request::Diff(request) => diff(repository, &request)?,
        Request::Log(request) => log(repository, &request)?,
        Request::Ancestor(query) => Response::Ancestor(AncestorResult::new(
            read(repository).is_ancestor(query.ancestor, query.descendant)?,
        )),
        Request::MergeBase(query) => Response::MergeBase(MergeBaseResult::new(
            read(repository).merge_base(query.a, query.b)?,
        )),
        Request::GetRef(name) => Response::Ref(read(repository).get_ref(&name)?),
        Request::UpdateRef { name, update } => {
            Response::Ref(write(repository).update_ref(&name, update)?)
        }
        Request::Rebase(request) => Response::Rebase(RebaseResult::new(
            write(repository).rebase(request.changeset, request.onto)?,
        )),
        Request::Revert(request) => revert(repository, &request)?,
        Request::Land(request) => land(repository, &request)?,
    })
}

/// The entry at `path` in a changeset's snapshot.
fn stat(
    repository: &SharedRepository,
    changeset: ObjectId,
    path: &str,
) -> openscion_core::Result<Response> {
    let path = RepoPath::new(path)?;
    let entry = read(repository)
        .stat(changeset, &path)?
        .map(|(kind, blob)| Entry::new(path.as_str(), kind, blob));
    Ok(Response::Stat(StatResult::new(entry)))
}

/// The entries under `path` (or the root) in a changeset's snapshot.
fn list_tree(
    repository: &SharedRepository,
    changeset: ObjectId,
    path: Option<&str>,
    recursive: bool,
) -> openscion_core::Result<Response> {
    let path = path.map(RepoPath::new).transpose()?;
    let entries = read(repository)
        .list_tree(changeset, path.as_ref(), recursive)?
        .into_iter()
        .map(|(path, kind, blob)| Entry::new(path, kind, blob))
        .collect();
    Ok(Response::Tree(TreeListing::new(entries)))
}

/// A file's bytes in a changeset's snapshot, or `None`.
fn read_file(
    repository: &SharedRepository,
    changeset: ObjectId,
    path: &str,
) -> openscion_core::Result<Response> {
    let path = RepoPath::new(path)?;
    Ok(Response::File(
        read(repository)
            .read_file(changeset, &path)?
            .map(|(_, bytes)| bytes.to_vec()),
    ))
}

/// The changes that turn one snapshot into another.
fn diff(repository: &SharedRepository, request: &DiffRequest) -> openscion_core::Result<Response> {
    let prefixes: Vec<&str> = request.prefixes.iter().map(String::as_str).collect();
    Ok(Response::Diff(DiffResult::new(read(repository).diff(
        request.from,
        request.to,
        &prefixes,
    )?)))
}

/// A page of first-parent history.
fn log(repository: &SharedRepository, request: &LogRequest) -> openscion_core::Result<Response> {
    let limit = usize::try_from(request.limit).unwrap_or(usize::MAX);
    Ok(Response::Log(LogPage::new(read(repository).log(
        request.from,
        request.until,
        request.path.as_ref(),
        limit,
    )?)))
}

/// Undo a changeset on top of another, producing a new changeset.
fn revert(
    repository: &SharedRepository,
    request: &RevertRequest,
) -> openscion_core::Result<Response> {
    Ok(Response::Rebase(RebaseResult::new(
        write(repository).revert(
            request.changeset,
            request.onto,
            &request.author,
            &request.message,
        )?,
    )))
}

/// Move a ref to include a changeset by compare-and-swap, with retries.
fn land(repository: &SharedRepository, request: &LandRequest) -> openscion_core::Result<Response> {
    let (changeset, version) =
        write(repository).land(request.changeset, &request.ref_name, request.max_attempts)?;
    Ok(Response::Land(LandResult::new(changeset, version)))
}

/// Maps an engine object lookup onto the tier's serving result: found bytes are
/// served, the engine's absent case becomes the tier's `NotFound` (which the
/// transport answers as *absent*), and any other engine failure is relayed as an
/// upstream error. The origin never sheds, so `Unavailable` never arises here.
fn map_object(
    found: openscion_core::Result<Arc<[u8]>>,
    id: ObjectId,
) -> openscion_tier::Result<Arc<[u8]>> {
    match found {
        Ok(bytes) => Ok(bytes),
        Err(openscion_core::Error::NotFound(_)) => Err(openscion_tier::Error::NotFound(id)),
        Err(error) => Err(openscion_tier::Error::Upstream(error.to_string())),
    }
}

#[cfg(test)]
mod test;

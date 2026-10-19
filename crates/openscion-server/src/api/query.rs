//! The query routes: snapshot reads and history questions over the engine.
//!
//! Snapshot reads name their changeset in the URL and a path in the query
//! string; history questions name their changesets in a JSON body. Each
//! handler makes one engine call and shapes the result into its payload. A
//! single file's bytes travel as a raw `application/octet-stream` body, the
//! way batched objects do, rather than base64 inside JSON.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use openscion_core::{
    AncestorQuery, AncestorResult, DiffRequest, DiffResult, Entry, LogPage, LogRequest,
    MergeBaseQuery, MergeBaseResult, ObjectId, RepoPath, StatResult, TreeListing,
};
use serde::Deserialize;

use super::{ApiError, SharedRepository, read};

/// A query string naming one path.
#[derive(Debug, Deserialize)]
pub(super) struct PathParam {
    /// The path to resolve, relative to the repository root.
    path: String,
}

/// A query string for a tree listing: an optional path and a recursion flag.
#[derive(Debug, Deserialize)]
pub(super) struct TreeParams {
    /// The directory to list, or the root when absent.
    #[serde(default)]
    path: Option<String>,
    /// List every entry beneath, not just the direct children.
    #[serde(default)]
    recursive: bool,
}

/// `GET /v1/changesets/{id}/stat?path=` — the entry at a path, or `null`.
pub(super) async fn stat(
    State(repository): State<SharedRepository>,
    Path(id): Path<String>,
    Query(params): Query<PathParam>,
) -> Result<Json<StatResult>, ApiError> {
    let id: ObjectId = id.parse()?;
    let path = RepoPath::new(&params.path)?;
    let entry = read(&repository)
        .stat(id, &path)?
        .map(|(kind, blob)| Entry::new(path.as_str(), kind, blob));
    Ok(Json(StatResult::new(entry)))
}

/// `GET /v1/changesets/{id}/tree?path=&recursive=` — the entries under a path.
pub(super) async fn list_tree(
    State(repository): State<SharedRepository>,
    Path(id): Path<String>,
    Query(params): Query<TreeParams>,
) -> Result<Json<TreeListing>, ApiError> {
    let id: ObjectId = id.parse()?;
    let path = params.path.as_deref().map(RepoPath::new).transpose()?;
    let entries = read(&repository)
        .list_tree(id, path.as_ref(), params.recursive)?
        .into_iter()
        .map(|(path, kind, blob)| Entry::new(path, kind, blob))
        .collect();
    Ok(Json(TreeListing::new(entries)))
}

/// `GET /v1/changesets/{id}/file?path=` — a file's bytes, or `404` when the
/// path names nothing or a directory.
pub(super) async fn read_file(
    State(repository): State<SharedRepository>,
    Path(id): Path<String>,
    Query(params): Query<PathParam>,
) -> Result<impl IntoResponse, ApiError> {
    let id: ObjectId = id.parse()?;
    let path = RepoPath::new(&params.path)?;
    match read(&repository).read_file(id, &path)? {
        Some((_, bytes)) => Ok((
            [(header::CONTENT_TYPE, "application/octet-stream")],
            Bytes::copy_from_slice(&bytes),
        )),
        None => Err(ApiError::NoFile(params.path)),
    }
}

/// `POST /v1/diff` — the changes that turn one snapshot into another.
pub(super) async fn diff(
    State(repository): State<SharedRepository>,
    Json(request): Json<DiffRequest>,
) -> Result<Json<DiffResult>, ApiError> {
    let prefixes: Vec<&str> = request.prefixes.iter().map(String::as_str).collect();
    let changes = read(&repository).diff(request.from, request.to, &prefixes)?;
    Ok(Json(DiffResult::new(changes)))
}

/// `POST /v1/log` — a page of first-parent history, newest first.
pub(super) async fn log(
    State(repository): State<SharedRepository>,
    Json(request): Json<LogRequest>,
) -> Result<Json<LogPage>, ApiError> {
    let limit = usize::try_from(request.limit).unwrap_or(usize::MAX);
    let changesets =
        read(&repository).log(request.from, request.until, request.path.as_ref(), limit)?;
    Ok(Json(LogPage::new(changesets)))
}

/// `POST /v1/ancestor` — whether one changeset is an ancestor of another.
pub(super) async fn is_ancestor(
    State(repository): State<SharedRepository>,
    Json(request): Json<AncestorQuery>,
) -> Result<Json<AncestorResult>, ApiError> {
    let answer = read(&repository).is_ancestor(request.ancestor, request.descendant)?;
    Ok(Json(AncestorResult::new(answer)))
}

/// `POST /v1/merge-base` — a lowest common ancestor of two changesets.
pub(super) async fn merge_base(
    State(repository): State<SharedRepository>,
    Json(request): Json<MergeBaseQuery>,
) -> Result<Json<MergeBaseResult>, ApiError> {
    let base = read(&repository).merge_base(request.a, request.b)?;
    Ok(Json(MergeBaseResult::new(base)))
}

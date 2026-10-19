//! The OpenScion API routes: objects, changesets, and refs over the engine.
//!
//! Each handler takes the shared [`Repository`], makes one engine call, and
//! translates the result to HTTP. JSON carries every payload except object
//! bytes, which travel as a frame stream (`id ‖ u64 length ‖ bytes`, see
//! `openscion_core::FrameWriter`) so a batch of thousands of files is one
//! request with no base64 overhead.
//!
//! | Method | Path | Request | Response |
//! | --- | --- | --- | --- |
//! | `POST` | `/v1/blobs` | blob frames | `StoredObjects` |
//! | `POST` | `/v1/objects` | `FetchObjects` | object frames, in request order |
//! | `POST` | `/v1/changesets` | `CreateChangeset` | `ChangesetInfo` |
//! | `GET` | `/v1/changesets/{id}` | — | `ChangesetRecord` |
//! | `GET` | `/v1/changesets/{id}/stat` | `?path=` | `StatResult` |
//! | `GET` | `/v1/changesets/{id}/tree` | `?path=&recursive=` | `TreeListing` |
//! | `GET` | `/v1/changesets/{id}/file` | `?path=` | file bytes, or `404` |
//! | `POST` | `/v1/diff` | `DiffRequest` | `DiffResult` |
//! | `POST` | `/v1/log` | `LogRequest` | `LogPage` |
//! | `POST` | `/v1/ancestor` | `AncestorQuery` | `AncestorResult` |
//! | `POST` | `/v1/merge-base` | `MergeBaseQuery` | `MergeBaseResult` |
//! | `GET` | `/v1/refs/{name}` | — | `RefValue` |
//! | `POST` | `/v1/refs/{name}` | `UpdateRef` | `RefValue`, or `409` on conflict |
//! | `POST` | `/v1/rebase` | `RebaseRequest` | `RebaseResult`, or `409` on conflict |
//! | `POST` | `/v1/revert` | `RevertRequest` | `RebaseResult`, or `409` on conflict |
//! | `POST` | `/v1/land` | `LandRequest` | `LandResult`, or `409` on conflict |
//!
//! Failures answer with an `ErrorBody`; see [`ApiError`] for the status codes.

mod failure;
mod landing;
mod query;

use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use openscion_core::{
    ChangesetInfo, ChangesetRecord, CreateChangeset, FetchObjects, FrameWriter, ObjectId, RefValue,
    Repository, StoredObjects, UpdateRef, decode_frames,
};

pub use failure::ApiError;

/// The repository every request shares.
pub type SharedRepository = Arc<RwLock<Repository>>;

/// The largest request body the API accepts: a batch of blobs.
pub const MAX_BODY_BYTES: usize = 1 << 30;

/// The API routes, with the shared repository as their state.
pub(crate) fn routes() -> Router<SharedRepository> {
    Router::new()
        .route("/v1/blobs", post(put_blobs))
        .route("/v1/objects", post(fetch_objects))
        .route("/v1/changesets", post(create_changeset))
        .route("/v1/changesets/{id}", get(get_changeset))
        .route("/v1/changesets/{id}/stat", get(query::stat))
        .route("/v1/changesets/{id}/tree", get(query::list_tree))
        .route("/v1/changesets/{id}/file", get(query::read_file))
        .route("/v1/diff", post(query::diff))
        .route("/v1/log", post(query::log))
        .route("/v1/ancestor", post(query::is_ancestor))
        .route("/v1/merge-base", post(query::merge_base))
        .route("/v1/refs/{*name}", get(get_ref).post(update_ref))
        .route("/v1/rebase", post(landing::rebase))
        .route("/v1/revert", post(landing::revert))
        .route("/v1/land", post(landing::land))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

// A handler that panicked mid-write cannot leave the repository half-updated:
// every engine call validates before it mutates. So a poisoned lock still
// guards consistent data, and the server keeps serving rather than failing
// every later request.
pub(super) fn read(repository: &SharedRepository) -> RwLockReadGuard<'_, Repository> {
    repository.read().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn write(repository: &SharedRepository) -> RwLockWriteGuard<'_, Repository> {
    repository.write().unwrap_or_else(PoisonError::into_inner)
}

async fn put_blobs(
    State(repository): State<SharedRepository>,
    body: Bytes,
) -> Result<Json<StoredObjects>, ApiError> {
    let frames = decode_frames(&body)?;
    let mut repository = write(&repository);
    let mut ids = Vec::with_capacity(frames.len());
    for (id, bytes) in frames {
        repository.put_blob_verified(id, bytes)?;
        ids.push(id);
    }
    Ok(Json(StoredObjects::new(ids)))
}

async fn fetch_objects(
    State(repository): State<SharedRepository>,
    Json(request): Json<FetchObjects>,
) -> Result<impl IntoResponse, ApiError> {
    let repository = read(&repository);
    let mut frames = FrameWriter::new();
    for id in request.ids {
        frames.push(id, &repository.object(id)?);
    }
    Ok((
        [(header::CONTENT_TYPE, "application/octet-stream")],
        frames.finish(),
    ))
}

async fn create_changeset(
    State(repository): State<SharedRepository>,
    Json(request): Json<CreateChangeset>,
) -> Result<Json<ChangesetInfo>, ApiError> {
    Ok(Json(write(&repository).create_changeset(request)?))
}

async fn get_changeset(
    State(repository): State<SharedRepository>,
    Path(id): Path<String>,
) -> Result<Json<ChangesetRecord>, ApiError> {
    let id: ObjectId = id.parse()?;
    Ok(Json(read(&repository).changeset(id)?))
}

async fn get_ref(
    State(repository): State<SharedRepository>,
    Path(name): Path<String>,
) -> Result<Json<RefValue>, ApiError> {
    Ok(Json(read(&repository).get_ref(&name)?))
}

async fn update_ref(
    State(repository): State<SharedRepository>,
    Path(name): Path<String>,
    Json(update): Json<UpdateRef>,
) -> Result<Json<RefValue>, ApiError> {
    Ok(Json(write(&repository).update_ref(&name, update)?))
}

#[cfg(test)]
mod test;

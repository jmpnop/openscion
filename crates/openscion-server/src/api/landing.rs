//! The landing routes: rebase, revert, and land over the engine.
//!
//! Each handler takes a JSON request, makes one engine call under the write
//! lock, and answers with the changeset it produced. A conflict comes back as
//! a `409` with the `MergeConflict` code and the conflicting paths in its
//! message; contention that outlasts a land's attempts is a `409 RefConflict`.

use axum::Json;
use axum::extract::State;
use openscion_core::{LandRequest, LandResult, RebaseRequest, RebaseResult, RevertRequest};

use super::{ApiError, SharedRepository, write};

/// `POST /v1/rebase` — move a changeset's changes onto another changeset.
pub(super) async fn rebase(
    State(repository): State<SharedRepository>,
    Json(request): Json<RebaseRequest>,
) -> Result<Json<RebaseResult>, ApiError> {
    let changeset = write(&repository).rebase(request.changeset, request.onto)?;
    Ok(Json(RebaseResult::new(changeset)))
}

/// `POST /v1/revert` — undo a changeset on top of another.
pub(super) async fn revert(
    State(repository): State<SharedRepository>,
    Json(request): Json<RevertRequest>,
) -> Result<Json<RebaseResult>, ApiError> {
    let changeset = write(&repository).revert(
        request.changeset,
        request.onto,
        &request.author,
        &request.message,
    )?;
    Ok(Json(RebaseResult::new(changeset)))
}

/// `POST /v1/land` — move a ref to include a changeset by compare-and-swap.
pub(super) async fn land(
    State(repository): State<SharedRepository>,
    Json(request): Json<LandRequest>,
) -> Result<Json<LandResult>, ApiError> {
    let (changeset, version) =
        write(&repository).land(request.changeset, &request.ref_name, request.max_attempts)?;
    Ok(Json(LandResult::new(changeset, version)))
}

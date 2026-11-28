//! Google Workspace directory settings and group mirroring.

use axum::{
    Json, Router,
    extract::State,
    http::HeaderMap,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Serialize;

use crate::identity::{
    Identity, IdentityError, Receipt,
    google::{GoogleDirectory, SaveGoogle},
};

#[derive(Clone)]
struct Api {
    identity: Identity,
    directory: GoogleDirectory,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(settings, save, sync, groups))]
pub(super) struct Doc;

pub(super) fn router(identity: Identity, directory: GoogleDirectory) -> Router {
    Router::new()
        .route("/api/identity/google", get(settings).put(save))
        .route("/api/identity/google/sync", post(sync))
        .route("/api/identity/google/groups", get(groups))
        .with_state(Api {
            identity,
            directory,
        })
}

fn no_store<T: Serialize>(value: T) -> impl IntoResponse {
    ([("cache-control", "no-store")], Json(value))
}

/// Administrators and auditors; the service account key is never returned.
#[utoipa::path(get, path = "/api/identity/google", operation_id = "google_directory", responses((status = 200, body = crate::identity::google::GoogleSettings), (status = 403)))]
async fn settings(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok(no_store(api.directory.settings(&actor).await?))
}

/// 400 for an invalid key or address, or enabling without both; 503 when no credential key is
/// configured to seal a new key.
#[utoipa::path(put, path = "/api/identity/google", operation_id = "save_google_directory", request_body = SaveGoogle, responses((status = 200, body = Receipt), (status = 400), (status = 403), (status = 503)))]
async fn save(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<SaveGoogle>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok(no_store(api.directory.save(&actor, &input).await?))
}

/// Mirrors every linked Google group now; groups Google could not list are named in `problem`.
/// 409 until a key and administrator are saved.
#[utoipa::path(post, path = "/api/identity/google/sync", operation_id = "sync_google_directory", responses((status = 200, body = crate::identity::google::SyncOutcome), (status = 403), (status = 409)))]
async fn sync(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok(no_store(api.directory.sync_now(&actor).await?))
}

/// The Workspace's groups, for linking; 503 when Google refuses.
#[utoipa::path(get, path = "/api/identity/google/groups", operation_id = "google_workspace_groups", responses((status = 200, body = Vec<crate::identity::google::WorkspaceGroup>), (status = 403), (status = 409), (status = 503)))]
async fn groups(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok(no_store(api.directory.workspace_groups(&actor).await?))
}

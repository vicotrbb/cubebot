use crate::{
    identity::{Identity, IdentityError},
    work::Work,
};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, put},
};

#[derive(Clone)]
struct Api {
    identity: Identity,
    work: Work,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(activity, read_activity))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/activity", get(activity))
        .route("/api/activity/read", put(read_activity))
        .with_state(Api {
            identity,
            work: Work::new(pool),
        })
}

/// What needs the caller, newest first, at most 100 items.
#[utoipa::path(get, path = "/api/activity", responses((status = 200, body = Vec<crate::work::ActivityItem>)))]
async fn activity(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.work.activity(&actor).await?),
    ))
}

/// Marks the caller's inbox as seen, on every device; its items stay until they are dealt with.
#[utoipa::path(put, path = "/api/activity/read", operation_id = "read_activity", responses((status = 204)))]
async fn read_activity(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.work.read_activity(&actor).await?;
    Ok(StatusCode::NO_CONTENT)
}

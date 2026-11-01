use crate::{
    identity::{Identity, IdentityError},
    work::Work,
};
use axum::{Json, Router, extract::State, http::HeaderMap, response::IntoResponse, routing::get};

#[derive(Clone)]
struct Api {
    identity: Identity,
    work: Work,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(activity))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/activity", get(activity))
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

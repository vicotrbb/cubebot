use super::Installation;
use crate::{
    identity::{Identity, IdentityError},
    watchdog::{Dismiss, Watchdog},
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, put},
};
use uuid::Uuid;

#[derive(Clone)]
struct Api {
    identity: Identity,
    watchdog: Watchdog,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(overview, dismiss, unmute))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/watchdog", get(overview))
        .route("/api/watchdog/findings/{finding}/dismiss", put(dismiss))
        .route("/api/watchdog/mutes/{mute}", delete(unmute))
        .with_state(Api {
            identity,
            watchdog: Watchdog::new(pool),
        })
}

/// Open and recently dismissed findings, mutes and posture checks; administrators only.
#[utoipa::path(get, path = "/api/watchdog", operation_id = "watchdog", responses((status = 200, body = crate::watchdog::Overview)))]
async fn overview(
    State(api): State<Api>,
    headers: HeaderMap,
    flags: Option<Extension<Installation>>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let flags = flags.map_or_else(Installation::default, |Extension(flags)| flags);
    Ok((
        [("cache-control", "no-store")],
        Json(api.watchdog.overview(&actor, &flags).await?),
    ))
}

#[utoipa::path(put, path = "/api/watchdog/findings/{finding}/dismiss", operation_id = "dismiss_finding", params(("finding" = Uuid, Path)), request_body = Dismiss, responses((status = 204), (status = 404)))]
async fn dismiss(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(finding): Path<Uuid>,
    Json(input): Json<Dismiss>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.watchdog.dismiss(&actor, finding, &input).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/watchdog/mutes/{mute}", operation_id = "unmute_rule", params(("mute" = Uuid, Path)), responses((status = 204), (status = 404)))]
async fn unmute(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(mute): Path<Uuid>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.watchdog.unmute(&actor, mute).await?;
    Ok(StatusCode::NO_CONTENT)
}

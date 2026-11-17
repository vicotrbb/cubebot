use crate::{
    apps::{App, Apps},
    identity::{Identity, IdentityError},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{any, delete, get, post},
};
use cubebot_protocol::apps::Desired;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
struct Api {
    identity: Identity,
    apps: Apps,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(list_apps, set_app_state, remove_app, view_app))]
pub(super) struct Doc;

pub(super) fn router(apps: Apps, identity: Identity) -> Router {
    Router::new()
        .route("/api/apps", get(list_apps))
        .route("/api/apps/{slug}/state", post(set_app_state))
        .route("/api/apps/{slug}/views", post(view_app))
        .route("/api/apps/{slug}", delete(remove_app))
        .layer(DefaultBodyLimit::max(1024))
        .with_state(Api { apps, identity })
}

pub(super) fn relay(apps: Apps) -> Router {
    Router::new()
        .route("/api/apps/{slug}/v/{token}", any(relay_app))
        .route("/api/apps/{slug}/v/{token}/", any(relay_app))
        .route("/api/apps/{slug}/v/{token}/{*rest}", any(relay_app))
        .with_state(apps)
}

async fn relay_app(
    State(apps): State<Apps>,
    request: axum::extract::Request,
) -> axum::response::Response {
    apps.relay(request).await
}

#[derive(Serialize, utoipa::ToSchema)]
struct ViewPath {
    path: String,
}

#[utoipa::path(post, path = "/api/apps/{slug}/views", params(("slug" = String, Path)), responses((status = 200, body = ViewPath), (status = 404)))]
async fn view_app(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let token = api.apps.view(&actor, &slug).await?;
    let path = format!("/api/apps/{slug}/v/{token}/");
    Ok(([("cache-control", "no-store")], Json(ViewPath { path })))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
struct SetState {
    desired: Desired,
}

#[utoipa::path(get, path = "/api/apps", responses((status = 200, body = Vec<App>)))]
async fn list_apps(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.apps.list(&actor).await?),
    ))
}

#[utoipa::path(post, path = "/api/apps/{slug}/state", params(("slug" = String, Path)), request_body = SetState, responses((status = 200, body = App), (status = 404)))]
async fn set_app_state(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<SetState>,
) -> Result<Json<App>, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok(Json(
        api.apps.set_desired(&actor, &slug, input.desired).await?,
    ))
}

#[utoipa::path(delete, path = "/api/apps/{slug}", params(("slug" = String, Path)), responses((status = 204), (status = 403), (status = 404)))]
async fn remove_app(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.apps.remove(&actor, &slug).await?;
    Ok(StatusCode::NO_CONTENT)
}

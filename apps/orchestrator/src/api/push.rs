use crate::{
    identity::{Identity, IdentityError},
    push::{Push, PushEndpoint, PushSettings, PushSubscription, PushTest},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post, put},
};

#[derive(Clone)]
struct Api {
    identity: Identity,
    push: Push,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(settings, subscribe, unsubscribe, test))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/push", get(settings))
        .route(
            "/api/push/subscriptions",
            put(subscribe).delete(unsubscribe),
        )
        .route("/api/push/test", post(test))
        .layer(DefaultBodyLimit::max(8 * 1024))
        .with_state(Api {
            identity,
            push: Push::new(pool),
        })
}

#[utoipa::path(get, path = "/api/push", operation_id = "push_settings", responses((status = 200, body = PushSettings)))]
async fn settings(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<Json<PushSettings>, IdentityError> {
    super::identity::actor(&api.identity, &headers).await?;
    Ok(Json(api.push.settings().await?))
}

#[utoipa::path(put, path = "/api/push/subscriptions", operation_id = "push_subscribe", request_body = PushSubscription, responses((status = 204)))]
async fn subscribe(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(device): Json<PushSubscription>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.push.subscribe(&actor, device).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/push/subscriptions", operation_id = "push_unsubscribe", request_body = PushEndpoint, responses((status = 204)))]
async fn unsubscribe(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(device): Json<PushEndpoint>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.push.unsubscribe(&actor, &device.endpoint).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/push/test", operation_id = "push_test", responses((status = 200, body = PushTest)))]
async fn test(State(api): State<Api>, headers: HeaderMap) -> Result<Json<PushTest>, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok(Json(api.push.test(&actor).await?))
}

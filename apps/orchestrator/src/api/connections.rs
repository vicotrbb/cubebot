//! Human administration never returns recoverable provider credentials.
use crate::{
    connections::{ConfigureProvider, Connections},
    identity::{Identity, IdentityError},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::get,
};

#[derive(Clone)]
struct Api {
    identity: Identity,
    connections: Connections,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(inspect, configure))]
pub(super) struct Doc;

pub(super) fn router(identity: Identity, connections: Connections) -> Router {
    Router::new()
        .route("/api/provider", get(inspect).post(configure))
        .layer(DefaultBodyLimit::max(8192))
        .with_state(Api {
            identity,
            connections,
        })
}

#[utoipa::path(get, path = "/api/provider", responses((status = 200, body = Option<crate::connections::ProviderSummary>)))]
async fn inspect(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.connections.inspect(&actor).await?),
    ))
}

#[utoipa::path(post, path = "/api/provider", request_body = ConfigureProvider, responses((status = 200, body = crate::connections::ProviderSummary)))]
async fn configure(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<ConfigureProvider>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.connections.configure(&actor, &input).await?),
    ))
}

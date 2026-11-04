//! Human administration never returns recoverable provider credentials.
use crate::{
    connections::{ConfigureProvider, Connections, ListModels, ModelsError},
    identity::{Identity, IdentityError, audit::Event},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde_json::json;

#[derive(Clone)]
struct Api {
    identity: Identity,
    connections: Connections,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(inspect, configure, models, saved, forget))]
pub(super) struct Doc;

pub(super) fn router(identity: Identity, connections: Connections) -> Router {
    Router::new()
        .route("/api/provider", get(inspect).post(configure))
        .route("/api/provider/models", post(models))
        .route("/api/provider/saved", get(saved))
        .route("/api/provider/saved/{provider}", delete(forget))
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
    let summary = api.connections.configure(&actor, &input).await?;
    let detail = json!({
        "provider": summary.provider,
        "model": summary.model,
        "key_changed": input.api_key.is_some(),
        "revision": summary.revision,
    });
    let event = Event::by(actor.id, "provider.configure").detail(detail);
    api.identity.audit(event).await;
    Ok(([("cache-control", "no-store")], Json(summary)))
}

#[utoipa::path(get, path = "/api/provider/saved", responses((status = 200, body = Vec<crate::connections::SavedProvider>)))]
async fn saved(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.connections.saved(&actor).await?),
    ))
}

/// Forgets the key saved for a provider that is not in use; 404 when none is saved.
#[utoipa::path(delete, path = "/api/provider/saved/{provider}", params(("provider" = cubebot_protocol::runtime::ProviderKind, Path)), responses((status = 204), (status = 404)))]
async fn forget(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(provider): Path<cubebot_protocol::runtime::ProviderKind>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.connections.forget(&actor, provider).await?;
    let event = Event::by(actor.id, "provider.forget").detail(json!({"provider": provider}));
    api.identity.audit(event).await;
    Ok(StatusCode::NO_CONTENT)
}

/// The models a key can use, newest first: 422 when the provider rejects the key, 502 when it
/// cannot be reached, 404 without a key when none is saved for that provider.
#[utoipa::path(post, path = "/api/provider/models", request_body = ListModels, responses((status = 200, body = Vec<crate::connections::ModelChoice>), (status = 422), (status = 502)))]
async fn models(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<ListModels>,
) -> Result<Response, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    match api.connections.models(&actor, &input).await {
        Ok(models) => Ok(([("cache-control", "no-store")], Json(models)).into_response()),
        Err(ModelsError::Identity(error)) => Err(error),
        Err(ModelsError::Provider(error)) => {
            let status = match error {
                cubebot_providers::Error::Rejected => StatusCode::UNPROCESSABLE_ENTITY,
                _ => StatusCode::BAD_GATEWAY,
            };
            Ok((status, Json(json!({"error": error.code()}))).into_response())
        }
    }
}

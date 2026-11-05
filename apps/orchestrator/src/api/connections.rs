//! Human administration never returns recoverable provider credentials.
use crate::{
    connections::{ConfigureProvider, Connections, ListModels, ModelsError, SignInError},
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
#[openapi(paths(
    inspect,
    configure,
    models,
    saved,
    forget,
    subscriptions,
    start_chatgpt,
    finish_chatgpt,
    start_supergrok,
    poll_supergrok,
    sign_out_chatgpt,
    sign_out_supergrok
))]
pub(super) struct Doc;

pub(super) fn router(identity: Identity, connections: Connections) -> Router {
    Router::new()
        .route("/api/provider", get(inspect).post(configure))
        .route("/api/provider/models", post(models))
        .route("/api/provider/saved", get(saved))
        .route("/api/provider/saved/{provider}", delete(forget))
        .route("/api/provider/subscriptions", get(subscriptions))
        // Each plan's path carries its own sign-out: a `{provider}` route would lose to these.
        .route(
            "/api/provider/subscriptions/chatgpt",
            post(start_chatgpt).delete(sign_out_chatgpt),
        )
        .route(
            "/api/provider/subscriptions/chatgpt/{sign_in}",
            post(finish_chatgpt),
        )
        .route(
            "/api/provider/subscriptions/supergrok",
            post(start_supergrok).delete(sign_out_supergrok),
        )
        .route(
            "/api/provider/subscriptions/supergrok/{sign_in}",
            post(poll_supergrok),
        )
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

/// Signed-in plans, without their tokens.
#[utoipa::path(get, path = "/api/provider/subscriptions", responses((status = 200, body = Vec<crate::connections::SubscriptionSummary>)))]
async fn subscriptions(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.connections.subscriptions(&actor).await?),
    ))
}

/// Starts Sign in with ChatGPT; the browser ends on a `127.0.0.1` address the person pastes back.
#[utoipa::path(post, path = "/api/provider/subscriptions/chatgpt", responses((status = 200, body = crate::connections::ChatgptSignIn)))]
async fn start_chatgpt(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.connections.start_chatgpt(&actor).await?),
    ))
}

#[derive(serde::Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Landed {
    /// The `http://127.0.0.1:1456/auth/callback?...` address the browser landed on.
    pub url: String,
}

/// 422 with `sign_in_expired`, `sign_in_denied`, `sign_in_refused`, `sign_in_mismatch` or
/// `plan_not_included`; 400 when the address is not this sign-in's; 409 while someone else's plan is signed in.
#[utoipa::path(post, path = "/api/provider/subscriptions/chatgpt/{sign_in}", params(("sign_in" = uuid::Uuid, Path)), request_body = Landed, responses((status = 200, body = crate::connections::SubscriptionSummary), (status = 400), (status = 409), (status = 422), (status = 502)))]
async fn finish_chatgpt(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(sign_in): Path<uuid::Uuid>,
    Json(landed): Json<Landed>,
) -> Result<Response, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let finished = api
        .connections
        .finish_chatgpt(&actor, sign_in, &landed.url)
        .await;
    signed_in(&api, &actor, finished.map(Some)).await
}

/// Starts SuperGrok device sign-in, which uses the Grok CLI's client id.
#[utoipa::path(post, path = "/api/provider/subscriptions/supergrok", responses((status = 200, body = crate::connections::SupergrokSignIn), (status = 502)))]
async fn start_supergrok(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<Response, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    match api.connections.start_supergrok(&actor).await {
        Ok(started) => Ok(([("cache-control", "no-store")], Json(started)).into_response()),
        Err(error) => refused(error),
    }
}

/// 202 until the person approves on xAI's page; then the signed-in plan.
#[utoipa::path(post, path = "/api/provider/subscriptions/supergrok/{sign_in}", params(("sign_in" = uuid::Uuid, Path)), responses((status = 200, body = crate::connections::SubscriptionSummary), (status = 202), (status = 409), (status = 422), (status = 502)))]
async fn poll_supergrok(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(sign_in): Path<uuid::Uuid>,
) -> Result<Response, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let polled = api.connections.poll_supergrok(&actor, sign_in).await;
    signed_in(&api, &actor, polled).await
}

async fn signed_in(
    api: &Api,
    actor: &crate::identity::Actor,
    result: Result<Option<crate::connections::SubscriptionSummary>, SignInError>,
) -> Result<Response, IdentityError> {
    match result {
        Ok(Some(summary)) => {
            let detail = json!({"provider": summary.provider});
            api.identity
                .audit(Event::by(actor.id, "provider.sign_in").detail(detail))
                .await;
            Ok(([("cache-control", "no-store")], Json(summary)).into_response())
        }
        Ok(None) => Ok(StatusCode::ACCEPTED.into_response()),
        Err(error) => refused(error),
    }
}

fn refused(error: SignInError) -> Result<Response, IdentityError> {
    match error {
        SignInError::Identity(error) => Err(error),
        SignInError::Refused(code) => Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": code})),
        )
            .into_response()),
        SignInError::Unreachable(source) => {
            tracing::warn!(error = ?source.map(|source| source.to_string()), "subscription vendor unreachable");
            Ok((
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": "sign_in_unreachable"})),
            )
                .into_response())
        }
    }
}

/// Forgets the ChatGPT plan, stops agents using it and asks OpenAI to revoke it; `revoked` is
/// false when OpenAI did not confirm, and the person can disconnect the app in ChatGPT's settings.
#[utoipa::path(delete, path = "/api/provider/subscriptions/chatgpt", responses((status = 200, body = SignedOut), (status = 404)))]
async fn sign_out_chatgpt(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<Json<SignedOut>, IdentityError> {
    sign_out(
        &api,
        &headers,
        cubebot_protocol::runtime::ProviderKind::Chatgpt,
    )
    .await
}

/// Forgets the Grok plan, stops agents using it and asks xAI to revoke it.
#[utoipa::path(delete, path = "/api/provider/subscriptions/supergrok", responses((status = 200, body = SignedOut), (status = 404)))]
async fn sign_out_supergrok(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<Json<SignedOut>, IdentityError> {
    sign_out(
        &api,
        &headers,
        cubebot_protocol::runtime::ProviderKind::Supergrok,
    )
    .await
}

async fn sign_out(
    api: &Api,
    headers: &HeaderMap,
    provider: cubebot_protocol::runtime::ProviderKind,
) -> Result<Json<SignedOut>, IdentityError> {
    let actor = super::identity::actor(&api.identity, headers).await?;
    let revoked = api.connections.sign_out(&actor, provider).await?;
    let detail = json!({"provider": provider, "revoked": revoked});
    api.identity
        .audit(Event::by(actor.id, "provider.sign_out").detail(detail))
        .await;
    Ok(Json(SignedOut { revoked }))
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct SignedOut {
    pub revoked: bool,
}

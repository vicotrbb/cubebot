//! Native connections in the console. No response carries a secret or token.
use crate::{
    agents::AgentId,
    connections::{
        AgentConnection, Catalog, ConnectError, ConnectionActivity, ConnectionSummary, Connections,
        FinishSignIn, SaveConnection, SignInStarted, StartSignIn, UpdateConnection,
    },
    identity::{
        Actor, Identity, IdentityError,
        access::{Capability, Scope},
        audit::Event,
    },
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

#[derive(Clone)]
struct Api {
    identity: Identity,
    connections: Connections,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ConnectionGrant {
    /// Whether the agent may use the connection.
    enabled: bool,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(
    catalog,
    list,
    create,
    update,
    delete,
    check,
    activity,
    start,
    finish,
    agent_connections,
    grant
))]
pub(super) struct Doc;

pub(super) fn router(identity: Identity, connections: Connections) -> Router {
    Router::new()
        .route("/api/connectors", get(catalog))
        .route("/api/connections", get(list).post(create))
        .route("/api/connections/callback", get(callback))
        .route("/api/connections/sign-ins", post(start))
        .route("/api/connections/sign-ins/{sign_in}", post(finish))
        .route("/api/connections/{id}", put(update).delete(delete))
        .route("/api/connections/{id}/check", post(check))
        .route("/api/connections/{id}/activity", get(activity))
        .route("/api/agents/{id}/connections", get(agent_connections))
        .route("/api/agents/{id}/connections/{connection}", put(grant))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(Api {
            identity,
            connections,
        })
}

async fn administrator(api: &Api, headers: &HeaderMap) -> Result<Actor, IdentityError> {
    let actor = super::identity::actor(&api.identity, headers).await?;
    if !actor.administrator {
        return Err(IdentityError::Forbidden);
    }
    Ok(actor)
}

impl IntoResponse for ConnectError {
    fn into_response(self) -> Response {
        let (status, error, message) = match self {
            Self::Identity(error) => return error.into_response(),
            Self::Invalid(message) => (StatusCode::BAD_REQUEST, "invalid_input", message),
            Self::Refused(message) => (StatusCode::UNPROCESSABLE_ENTITY, "refused", message),
            Self::Unavailable(message) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "refused",
                format!("unavailable, nothing was done: {message}"),
            ),
            Self::SignInAgain => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "refused",
                "The account refused the credential.".into(),
            ),
        };
        let body = Json(json!({"error": error, "message": message}));
        (status, [("cache-control", "no-store")], body).into_response()
    }
}

/// Names only: settings can hold an address, and secrets never leave the connection.
async fn audit(api: &Api, actor: &Actor, kind: &'static str, connection: &ConnectionSummary) {
    let detail = json!({"connector": connection.connector, "slug": connection.slug, "level": connection.level});
    let event = Event::by(actor.id, kind).on(connection.id).detail(detail);
    api.identity.audit(event).await;
}

/// Every connector, and the redirect addresses an OAuth client must allow.
#[utoipa::path(get, path = "/api/connectors", responses((status = 200, body = Catalog)))]
async fn catalog(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<Json<Catalog>, IdentityError> {
    administrator(&api, &headers).await?;
    let origin = api.identity.browser_origin.as_deref();
    Ok(Json(api.connections.catalog(origin)))
}

#[utoipa::path(get, path = "/api/connections", operation_id = "list_connections", responses((status = 200, body = Vec<ConnectionSummary>)))]
async fn list(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<Json<Vec<ConnectionSummary>>, IdentityError> {
    administrator(&api, &headers).await?;
    Ok(Json(api.connections.connections().await?))
}

/// Saves a connection signed in with pasted secrets or another connection, once its check
/// passes; 422 with the vendor's reason when it does not, 409 when the slug is taken.
#[utoipa::path(post, path = "/api/connections", operation_id = "create_connection", request_body = SaveConnection, responses((status = 200, body = ConnectionSummary), (status = 400), (status = 409), (status = 422)))]
async fn create(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<SaveConnection>,
) -> Result<Json<ConnectionSummary>, ConnectError> {
    let actor = administrator(&api, &headers).await?;
    let connection = api.connections.create_connection(actor.id, input).await?;
    audit(&api, &actor, "connection.create", &connection).await;
    Ok(Json(connection))
}

/// Changed settings or secrets are checked again; the result is in the status.
#[utoipa::path(put, path = "/api/connections/{id}", operation_id = "update_connection", params(("id" = Uuid, Path)), request_body = UpdateConnection, responses((status = 200, body = ConnectionSummary), (status = 400), (status = 409)))]
async fn update(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateConnection>,
) -> Result<Json<ConnectionSummary>, ConnectError> {
    let actor = administrator(&api, &headers).await?;
    let connection = api.connections.update_connection(id, input).await?;
    audit(&api, &actor, "connection.update", &connection).await;
    Ok(Json(connection))
}

/// Forgets the connection and asks the vendor to revoke an OAuth sign-in.
#[utoipa::path(delete, path = "/api/connections/{id}", operation_id = "delete_connection", params(("id" = Uuid, Path)), responses((status = 204), (status = 404)))]
async fn delete(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, IdentityError> {
    let actor = administrator(&api, &headers).await?;
    let slug = api.connections.delete_connection(id).await?;
    let event = Event::by(actor.id, "connection.delete").on(id);
    api.identity
        .audit(event.detail(json!({"slug": slug})))
        .await;
    Ok(StatusCode::NO_CONTENT)
}

/// Tests the connection: renews its credential when due and runs the connector's check.
#[utoipa::path(post, path = "/api/connections/{id}/check", operation_id = "check_connection", params(("id" = Uuid, Path)), responses((status = 200, body = ConnectionSummary)))]
async fn check(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<ConnectionSummary>, IdentityError> {
    administrator(&api, &headers).await?;
    Ok(Json(api.connections.check_connection(id).await?))
}

#[utoipa::path(get, path = "/api/connections/{id}/activity", operation_id = "connection_activity", params(("id" = Uuid, Path)), responses((status = 200, body = ConnectionActivity)))]
async fn activity(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<ConnectionActivity>, IdentityError> {
    administrator(&api, &headers).await?;
    Ok(Json(api.connections.connection_activity(id).await?))
}

/// Starts an OAuth, device-code or GitHub App sign-in with the person's own app.
#[utoipa::path(post, path = "/api/connections/sign-ins", operation_id = "start_connection_sign_in", request_body = StartSignIn, responses((status = 200, body = SignInStarted), (status = 400), (status = 422)))]
async fn start(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<StartSignIn>,
) -> Result<Response, ConnectError> {
    let actor = administrator(&api, &headers).await?;
    let origin = api.identity.browser_origin.as_deref();
    let started = api
        .connections
        .start_connection_sign_in(actor.id, input, origin)
        .await?;
    Ok(([("cache-control", "no-store")], Json(started)).into_response())
}

/// Finishes with the pasted address the browser landed on, or polls a device sign-in: 202 until
/// the person approves, then the saved connection.
#[utoipa::path(post, path = "/api/connections/sign-ins/{sign_in}", operation_id = "finish_connection_sign_in", params(("sign_in" = Uuid, Path)), request_body = FinishSignIn, responses((status = 200, body = ConnectionSummary), (status = 202), (status = 400), (status = 422)))]
async fn finish(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(sign_in): Path<Uuid>,
    Json(input): Json<FinishSignIn>,
) -> Result<Response, ConnectError> {
    let actor = administrator(&api, &headers).await?;
    let finished = api
        .connections
        .finish_connection_sign_in(actor.id, sign_in, input)
        .await?;
    let Some(connection) = finished else {
        return Ok(StatusCode::ACCEPTED.into_response());
    };
    audit(&api, &actor, "connection.sign_in", &connection).await;
    Ok(Json(connection).into_response())
}

/// The vendor sends the browser here when the installation has a public address; it finishes the
/// sign-in that sent the `state` and returns to the console.
async fn callback(
    State(api): State<Api>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Redirect {
    let back = |query: String| Redirect::to(&format!("/platform/connections?{query}"));
    let (Some(origin), Some(query)) = (api.identity.browser_origin.clone(), query) else {
        return back("sign_in_error=unexpected".into());
    };
    let Ok(actor) = administrator(&api, &headers).await else {
        return back("sign_in_error=signed_out".into());
    };
    let landed = format!("{}?{query}", crate::connections::callback_address(&origin));
    let state = url_state(&landed);
    let Some(sign_in) = state.and_then(|state| api.connections.connection_sign_in_for(&state))
    else {
        return back("sign_in_error=expired".into());
    };
    let input = FinishSignIn { url: Some(landed) };
    match api
        .connections
        .finish_connection_sign_in(actor.id, sign_in, input)
        .await
    {
        Ok(Some(connection)) => {
            audit(&api, &actor, "connection.sign_in", &connection).await;
            back(format!("connected={}", connection.id))
        }
        Ok(None) | Err(_) => back("sign_in_error=refused".into()),
    }
}

fn url_state(landed: &str) -> Option<String> {
    let url = reqwest::Url::parse(landed).ok()?;
    url.query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
}

/// Every connection with whether this agent may use it; needs `agent.manage` on the agent.
#[utoipa::path(get, path = "/api/agents/{id}/connections", operation_id = "agent_connections", params(("id" = Uuid, Path)), responses((status = 200, body = Vec<AgentConnection>)))]
async fn agent_connections(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
) -> Result<Json<Vec<AgentConnection>>, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let scope = Scope::Agent { id: agent.0 };
    api.identity
        .authorize(&actor, scope, Capability::AgentManage)
        .await?;
    Ok(Json(api.connections.agent_connections(agent).await?))
}

/// Grants or withdraws the agent's use of a connection with explicit grants, for administrators;
/// 409 for a connection every agent may use, whose access is changed on the connection.
#[utoipa::path(put, path = "/api/agents/{id}/connections/{connection}", operation_id = "grant_connection", params(("id" = Uuid, Path), ("connection" = Uuid, Path)), request_body = ConnectionGrant, responses((status = 204), (status = 409)))]
async fn grant(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, connection)): Path<(AgentId, Uuid)>,
    Json(input): Json<ConnectionGrant>,
) -> Result<StatusCode, IdentityError> {
    let actor = administrator(&api, &headers).await?;
    api.connections
        .grant_connection(connection, agent, input.enabled)
        .await?;
    let detail = json!({"agent": agent.0, "enabled": input.enabled});
    let event = Event::by(actor.id, "connection.grant")
        .on(connection)
        .detail(detail);
    api.identity.audit(event).await;
    Ok(StatusCode::NO_CONTENT)
}

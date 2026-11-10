//! A person settles an agent's requests to create agents, rooms and teams.
use crate::{
    agents::AgentId,
    identity::{
        Identity, IdentityError,
        access::{Capability, Scope},
        audit::Event,
    },
    work::{CreationRequest, DeclineCreation, Work},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

#[derive(Clone)]
struct Api {
    identity: Identity,
    work: Work,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Created {
    pub id: Uuid,
    pub name: String,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(pending, approve, decline))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/agents/{id}/creations", get(pending))
        .route(
            "/api/agents/{id}/creations/{request}/approve",
            post(approve),
        )
        .route(
            "/api/agents/{id}/creations/{request}/decline",
            post(decline),
        )
        .with_state(Api {
            identity,
            work: Work::new(pool),
        })
}

/// The agent's requests waiting for a person, oldest first.
#[utoipa::path(get, path = "/api/agents/{id}/creations", params(("id" = Uuid, Path)), responses((status = 200, body = [CreationRequest])))]
async fn pending(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
) -> Result<Response, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let scope = Scope::Conversation { id: agent.0 };
    api.identity
        .authorize(&actor, scope, Capability::ConversationRead)
        .await?;
    let pending = api.work.pending_creations(agent).await?;
    Ok(([("cache-control", "no-store")], Json(pending)).into_response())
}

/// Creates it as the caller, with the caller's authority; 409 with the reason, told to the agent
/// too, when it cannot be created as asked; 404 when it is no longer pending.
#[utoipa::path(post, path = "/api/agents/{id}/creations/{request}/approve", params(("id" = Uuid, Path), ("request" = Uuid, Path)), responses((status = 201, body = Created), (status = 404), (status = 409)))]
async fn approve(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, request)): Path<(AgentId, Uuid)>,
) -> Result<Response, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let scope = Scope::Conversation { id: agent.0 };
    api.identity
        .authorize(&actor, scope, Capability::ConversationSend)
        .await?;
    let made = api.work.approve_creation(&actor, agent, request).await?;
    let detail = match &made {
        Ok((id, name)) => json!({ "request": request, "created": id, "name": name }),
        Err(reason) => json!({ "request": request, "failed": reason }),
    };
    let event = Event::by(actor.id, "creation.approve").on(agent.0);
    api.identity.audit(event.detail(detail)).await;
    Ok(match made {
        Ok((id, name)) => (StatusCode::CREATED, Json(Created { id, name })).into_response(),
        Err(reason) => super::refused(&reason),
    })
}

/// Declines it and tells the agent why; 404 when it is no longer pending.
#[utoipa::path(post, path = "/api/agents/{id}/creations/{request}/decline", params(("id" = Uuid, Path), ("request" = Uuid, Path)), request_body = DeclineCreation, responses((status = 204), (status = 404)))]
async fn decline(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, request)): Path<(AgentId, Uuid)>,
    Json(input): Json<DeclineCreation>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let scope = Scope::Conversation { id: agent.0 };
    api.identity
        .authorize(&actor, scope, Capability::ConversationSend)
        .await?;
    let reason = input.reason.trim();
    if reason.len() > 2048 || reason.contains('\0') {
        return Err(IdentityError::InvalidInput);
    }
    let reason = if reason.is_empty() {
        "no reason given"
    } else {
        reason
    };
    let kind = api
        .work
        .decline_creation(&actor, agent, request, reason)
        .await?;
    let event = Event::by(actor.id, "creation.decline").on(agent.0);
    api.identity
        .audit(event.detail(json!({ "request": request, "kind": kind, "reason": reason })))
        .await;
    Ok(StatusCode::NO_CONTENT)
}

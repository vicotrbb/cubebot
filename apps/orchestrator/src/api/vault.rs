//! The vault (docs/adr/identity.md#a-vault-the-desktop-arbiter-fills) for people: no response carries a secret. Installation items are an
//! administrator's; an agent's own items take `browser.control` on it to change, and the items
//! an agent may fill are listed with `browser.view`. A fill types into the desktop for the person,
//! who must hold control of it.
use crate::{
    agents::{AgentId, sandbox::Desktop},
    connections::{Connections, SaveVaultItem, VaultSummary},
    identity::{
        Actor, Identity, IdentityError,
        access::{Capability, Scope},
        audit::Event,
    },
};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use cubebot_protocol::runtime::VaultField;
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
pub(super) struct FillVaultItem {
    field: VaultField,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(list, create, update, delete, agent_items, fill))]
pub(super) struct Doc;

pub(super) fn router(identity: Identity, connections: Connections) -> Router {
    Router::new()
        .route("/api/vault", get(list).post(create))
        .route("/api/vault/{item}", put(update).delete(delete))
        .route("/api/agents/{id}/vault", get(agent_items))
        .route("/api/agents/{id}/vault/{item}/fill", post(fill))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(Api {
            identity,
            connections,
        })
}

/// Installation items for `None`, or the agent's own.
async fn allowed(
    api: &Api,
    actor: &Actor,
    agent: Option<AgentId>,
    capability: Capability,
) -> Result<(), IdentityError> {
    match agent {
        Some(agent) => {
            let scope = Scope::Agent { id: agent.0 };
            api.identity.authorize(actor, scope, capability).await
        }
        None if actor.administrator => Ok(()),
        None => Err(IdentityError::Forbidden),
    }
}

/// The item's agent, `None` for an installation item, once the actor may change it.
async fn item_scope(
    api: &Api,
    actor: &Actor,
    item: Uuid,
) -> Result<Option<AgentId>, IdentityError> {
    let scope = api.connections.vault_scope(item).await?;
    let scope = scope.ok_or(IdentityError::NotFound)?;
    allowed(api, actor, scope, Capability::BrowserControl).await?;
    Ok(scope)
}

/// Installation items, for administrators.
#[utoipa::path(get, path = "/api/vault", operation_id = "list_vault", responses((status = 200, body = Vec<VaultSummary>)))]
async fn list(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<Json<Vec<VaultSummary>>, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    allowed(&api, &actor, None, Capability::BrowserControl).await?;
    Ok(Json(api.connections.installation_vault().await?))
}

#[utoipa::path(post, path = "/api/vault", operation_id = "create_vault_item", request_body = SaveVaultItem, responses((status = 200, body = VaultSummary)))]
async fn create(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<SaveVaultItem>,
) -> Result<Json<VaultSummary>, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let agent = input.agent.map(AgentId);
    allowed(&api, &actor, agent, Capability::BrowserControl).await?;
    let item = api.connections.create_vault_item(actor.id, &input).await?;
    audit(&api, &actor, "vault.create", &item).await;
    Ok(Json(item))
}

async fn audit(api: &Api, actor: &Actor, kind: &'static str, item: &VaultSummary) {
    let detail =
        json!({"name": item.name, "site": item.site, "kind": item.kind, "agent": item.agent});
    let event = Event::by(actor.id, kind).on(item.id).detail(detail);
    api.identity.audit(event).await;
}

#[utoipa::path(put, path = "/api/vault/{item}", operation_id = "update_vault_item", params(("item" = Uuid, Path)), request_body = SaveVaultItem, responses((status = 200, body = VaultSummary)))]
async fn update(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(item): Path<Uuid>,
    Json(input): Json<SaveVaultItem>,
) -> Result<Json<VaultSummary>, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    // An agent's own item has no access to change.
    if item_scope(&api, &actor, item).await?.is_some() && input.access.is_some() {
        return Err(IdentityError::InvalidInput);
    }
    let item = api.connections.update_vault_item(item, &input).await?;
    audit(&api, &actor, "vault.update", &item).await;
    Ok(Json(item))
}

#[utoipa::path(delete, path = "/api/vault/{item}", operation_id = "delete_vault_item", params(("item" = Uuid, Path)), responses((status = 204)))]
async fn delete(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(item): Path<Uuid>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let agent = item_scope(&api, &actor, item).await?;
    let name = api.connections.delete_vault_item(item).await?;
    let detail = json!({"name": name, "agent": agent.map(|agent| agent.0)});
    let event = Event::by(actor.id, "vault.delete").on(item).detail(detail);
    api.identity.audit(event).await;
    Ok(StatusCode::NO_CONTENT)
}

/// The items the agent may fill: its own, then the installation's it has access to.
#[utoipa::path(get, path = "/api/agents/{id}/vault", operation_id = "agent_vault", params(("id" = Uuid, Path)), responses((status = 200, body = Vec<VaultSummary>)))]
async fn agent_items(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
) -> Result<Json<Vec<VaultSummary>>, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    allowed(&api, &actor, Some(agent), Capability::BrowserView).await?;
    Ok(Json(api.connections.agent_vault(agent).await?))
}

/// Types the field at the desktop's focus for the caller, who must hold control; a refusal is
/// 409 with the arbiter's reason.
#[utoipa::path(post, path = "/api/agents/{id}/vault/{item}/fill", operation_id = "fill_vault_item", params(("id" = Uuid, Path), ("item" = Uuid, Path)), request_body = FillVaultItem, responses((status = 204), (status = 409)))]
#[tracing::instrument(skip_all, fields(agent = %agent.0, item = %item))]
async fn fill(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, item)): Path<(AgentId, Uuid)>,
    desktop: Option<Extension<Desktop>>,
    Json(input): Json<FillVaultItem>,
) -> Result<Response, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    allowed(&api, &actor, Some(agent), Capability::BrowserControl).await?;
    let Some(Extension(desktop)) = desktop else {
        return Err(IdentityError::NotFound);
    };
    let text = api
        .connections
        .vault_fill_text(agent, Some(actor.id), item, input.field)
        .await?;
    let text = text.ok_or(IdentityError::NotFound)?;
    match super::desktop::fill(&desktop, agent, item, text, Some(actor.id)).await {
        Ok(Ok(())) => Ok(StatusCode::NO_CONTENT.into_response()),
        Ok(Err(reason)) => Ok(super::refused(&reason)),
        Err(error) => {
            tracing::warn!(?error, "vault fill did not reach the desktop");
            Ok(super::refused("the desktop is unavailable"))
        }
    }
}

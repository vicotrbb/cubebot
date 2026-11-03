//! MCP servers (docs/adr/agents.md#local-and-remote-mcp-server-support) for people: administrators add, change and remove them and
//! choose which agents may use them; no response carries an environment or header value. An
//! agent's Connections tab lists them for anyone who may manage that agent.
use crate::{
    agents::AgentId,
    connections::{AgentMcpServer, Connections, McpServerSummary, SaveMcpServer},
    identity::{
        Actor, Identity, IdentityError,
        access::{Capability, Scope},
    },
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, put},
};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Clone)]
struct Api {
    identity: Identity,
    connections: Connections,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct McpGrant {
    /// Whether the agent may use the server.
    enabled: bool,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(list, create, update, delete, agent_servers, grant))]
pub(super) struct Doc;

pub(super) fn router(identity: Identity, connections: Connections) -> Router {
    Router::new()
        .route("/api/mcp-servers", get(list).post(create))
        .route("/api/mcp-servers/{server}", put(update).delete(delete))
        .route("/api/agents/{id}/mcp-servers", get(agent_servers))
        .route("/api/agents/{id}/mcp-servers/{server}", put(grant))
        .layer(DefaultBodyLimit::max(512 * 1024))
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

#[utoipa::path(get, path = "/api/mcp-servers", operation_id = "list_mcp_servers", responses((status = 200, body = Vec<McpServerSummary>)))]
async fn list(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<Json<Vec<McpServerSummary>>, IdentityError> {
    administrator(&api, &headers).await?;
    Ok(Json(api.connections.mcp_servers().await?))
}

/// 409 when another server has the name.
#[utoipa::path(post, path = "/api/mcp-servers", operation_id = "create_mcp_server", request_body = SaveMcpServer, responses((status = 200, body = McpServerSummary), (status = 409)))]
async fn create(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<SaveMcpServer>,
) -> Result<Json<McpServerSummary>, IdentityError> {
    let actor = administrator(&api, &headers).await?;
    Ok(Json(
        api.connections.create_mcp_server(actor.id, &input).await?,
    ))
}

/// 409 when another server has the name.
#[utoipa::path(put, path = "/api/mcp-servers/{server}", operation_id = "update_mcp_server", params(("server" = Uuid, Path)), request_body = SaveMcpServer, responses((status = 200, body = McpServerSummary), (status = 409)))]
async fn update(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(server): Path<Uuid>,
    Json(input): Json<SaveMcpServer>,
) -> Result<Json<McpServerSummary>, IdentityError> {
    administrator(&api, &headers).await?;
    Ok(Json(
        api.connections.update_mcp_server(server, &input).await?,
    ))
}

#[utoipa::path(delete, path = "/api/mcp-servers/{server}", operation_id = "delete_mcp_server", params(("server" = Uuid, Path)), responses((status = 204)))]
async fn delete(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(server): Path<Uuid>,
) -> Result<StatusCode, IdentityError> {
    administrator(&api, &headers).await?;
    api.connections.delete_mcp_server(server).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Every server with whether this agent may use it; needs `agent.manage` on the agent.
#[utoipa::path(get, path = "/api/agents/{id}/mcp-servers", operation_id = "agent_mcp_servers", params(("id" = Uuid, Path)), responses((status = 200, body = Vec<AgentMcpServer>)))]
async fn agent_servers(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
) -> Result<Json<Vec<AgentMcpServer>>, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let scope = Scope::Agent { id: agent.0 };
    api.identity
        .authorize(&actor, scope, Capability::AgentManage)
        .await?;
    Ok(Json(api.connections.agent_mcp_servers(agent).await?))
}

/// Grants or withdraws the agent's use of a server with explicit grants, for administrators;
/// 409 for a server every agent may use, whose access is changed on the server.
#[utoipa::path(put, path = "/api/agents/{id}/mcp-servers/{server}", operation_id = "grant_mcp_server", params(("id" = Uuid, Path), ("server" = Uuid, Path)), request_body = McpGrant, responses((status = 204), (status = 409)))]
async fn grant(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, server)): Path<(AgentId, Uuid)>,
    Json(input): Json<McpGrant>,
) -> Result<StatusCode, IdentityError> {
    administrator(&api, &headers).await?;
    api.connections
        .grant_mcp_server(server, agent, input.enabled)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

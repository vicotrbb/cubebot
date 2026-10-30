use crate::{
    agents::AgentId,
    identity::{Identity, IdentityError},
    knowledge::{CreateMemory, Knowledge, ReviseMemory},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, put},
};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Clone)]
struct Api {
    identity: Identity,
    knowledge: Knowledge,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(search_memories, create_memory, revise_memory, delete_memory))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route(
            "/api/agents/{agent}/memories",
            get(search_memories).post(create_memory),
        )
        .route(
            "/api/agents/{agent}/memories/{memory}",
            put(revise_memory).delete(delete_memory),
        )
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(Api {
            identity,
            knowledge: Knowledge::new(pool),
        })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchQuery {
    query: Option<String>,
}

#[utoipa::path(get, path = "/api/agents/{agent}/memories", params(("agent" = Uuid, Path), ("query" = Option<String>, Query)), responses((status = 200, body = Vec<crate::knowledge::Memory>)))]
async fn search_memories(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
    Query(input): Query<SearchQuery>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    if input.query.as_ref().is_some_and(|query| query.len() > 1024) {
        return Err(IdentityError::InvalidInput);
    }
    Ok((
        [("cache-control", "no-store")],
        Json(
            api.knowledge
                .search(&actor, agent, input.query.as_deref())
                .await?,
        ),
    ))
}

#[utoipa::path(post, path = "/api/agents/{agent}/memories", params(("agent" = Uuid, Path)), request_body = CreateMemory, responses((status = 201, body = crate::knowledge::Memory)))]
async fn create_memory(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
    Json(input): Json<CreateMemory>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        StatusCode::CREATED,
        [("cache-control", "no-store")],
        Json(api.knowledge.create(&actor, agent, &input).await?),
    ))
}

/// 409 when `expected_revision` is no longer current.
#[utoipa::path(put, path = "/api/agents/{agent}/memories/{memory}", params(("agent" = Uuid, Path), ("memory" = Uuid, Path)), request_body = ReviseMemory, responses((status = 200, body = crate::knowledge::Memory)))]
async fn revise_memory(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, memory)): Path<(AgentId, Uuid)>,
    Json(input): Json<ReviseMemory>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.knowledge.revise(&actor, agent, memory, &input).await?),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    expected_revision: i64,
}

/// Removes the memory from retrieval; its revision history is kept.
#[utoipa::path(delete, path = "/api/agents/{agent}/memories/{memory}", params(("agent" = Uuid, Path), ("memory" = Uuid, Path), ("expected_revision" = i64, Query)), responses((status = 204)))]
async fn delete_memory(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, memory)): Path<(AgentId, Uuid)>,
    Query(input): Query<Expected>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.knowledge
        .delete(&actor, agent, memory, input.expected_revision)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

use crate::{
    agents::{AgentId, Agents, CreateAgent},
    identity::{Identity, IdentityError},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use serde::Deserialize;

#[derive(Clone)]
struct AgentApi {
    identity: Identity,
    agents: Agents,
}

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/agents", get(list).post(create))
        .route("/api/agents/{id}", get(read))
        .with_state(AgentApi {
            identity,
            agents: Agents::new(pool),
        })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    after: Option<AgentId>,
}

async fn list(
    State(api): State<AgentApi>,
    headers: HeaderMap,
    Query(cursor): Query<Cursor>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.agents.list(&actor, cursor.after).await?),
    ))
}

async fn read(
    State(api): State<AgentApi>,
    headers: HeaderMap,
    Path(id): Path<AgentId>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.agents.get(&actor, id).await?),
    ))
}

async fn create(
    State(api): State<AgentApi>,
    headers: HeaderMap,
    Json(input): Json<CreateAgent>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        StatusCode::CREATED,
        [("cache-control", "no-store")],
        Json(api.agents.create(&actor, &input).await?),
    ))
}

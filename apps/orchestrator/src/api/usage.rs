use crate::{
    agents::AgentId,
    identity::{Identity, IdentityError},
    work::{AgentUsage, SetBudget, Situation, Work},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::{get, put},
};

#[derive(Clone)]
struct Api {
    identity: Identity,
    work: Work,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(situation, agent_usage, set_budget))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/situation", get(situation))
        .route("/api/agents/{agent}/usage", get(agent_usage))
        .route("/api/agents/{agent}/budget", put(set_budget))
        .with_state(Api {
            identity,
            work: Work::new(pool),
        })
}

/// The agents the caller may read: their spend, open work and latest events.
#[utoipa::path(get, path = "/api/situation", responses((status = 200, body = Situation)))]
async fn situation(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.work.situation(&actor).await?),
    ))
}

/// Needs `conversation.read` on the agent.
#[utoipa::path(get, path = "/api/agents/{agent}/usage", operation_id = "agent_usage", params(("agent" = uuid::Uuid, Path)), responses((status = 200, body = AgentUsage)))]
async fn agent_usage(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.work.agent_usage(&actor, agent).await?),
    ))
}

/// Needs `agent.manage`. Room to spend requeues the work the old limit paused.
#[utoipa::path(put, path = "/api/agents/{agent}/budget", operation_id = "set_agent_budget", params(("agent" = uuid::Uuid, Path)), request_body = SetBudget, responses((status = 200, body = AgentUsage)))]
async fn set_budget(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
    Json(input): Json<SetBudget>,
) -> Result<Json<AgentUsage>, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok(Json(api.work.set_budget(&actor, agent, &input).await?))
}

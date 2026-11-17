use crate::{
    agents::AgentId,
    identity::{Identity, IdentityError},
    work::{
        Work,
        wakes::{CreateWake, SaveWake, Wake},
    },
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, put},
};
use uuid::Uuid;

#[derive(Clone)]
struct Api {
    identity: Identity,
    work: Work,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(list, create, update, delete))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/agents/{agent}/wakes", get(list).post(create))
        .route(
            "/api/agents/{agent}/wakes/{wake}",
            put(update).delete(delete),
        )
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(Api {
            identity,
            work: Work::new(pool),
        })
}

#[utoipa::path(get, path = "/api/agents/{agent}/wakes", operation_id = "list_wakes", params(("agent" = Uuid, Path)), responses((status = 200, body = Vec<Wake>)))]
async fn list(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let wakes = api.work.wakes(&actor, agent).await?;
    Ok(([("cache-control", "no-store")], Json(wakes)))
}

#[utoipa::path(post, path = "/api/agents/{agent}/wakes", operation_id = "create_wake", params(("agent" = Uuid, Path)), request_body = CreateWake, responses((status = 200, body = Wake)))]
async fn create(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
    Json(input): Json<CreateWake>,
) -> Result<Json<Wake>, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok(Json(api.work.create_wake(&actor, agent, &input).await?))
}

#[utoipa::path(put, path = "/api/agents/{agent}/wakes/{wake}", operation_id = "update_wake", params(("agent" = Uuid, Path), ("wake" = Uuid, Path)), request_body = SaveWake, responses((status = 200, body = Wake)))]
async fn update(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, wake)): Path<(AgentId, Uuid)>,
    Json(input): Json<SaveWake>,
) -> Result<Json<Wake>, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok(Json(
        api.work.update_wake(&actor, agent, wake, &input).await?,
    ))
}

#[utoipa::path(delete, path = "/api/agents/{agent}/wakes/{wake}", operation_id = "delete_wake", params(("agent" = Uuid, Path), ("wake" = Uuid, Path)), responses((status = 204)))]
async fn delete(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, wake)): Path<(AgentId, Uuid)>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.work.delete_wake(&actor, agent, wake).await?;
    Ok(StatusCode::NO_CONTENT)
}

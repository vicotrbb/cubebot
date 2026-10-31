use crate::{
    agents::{AgentId, Agents, CreateAgent, LifecycleCommand, UpdateAgent},
    identity::{Identity, IdentityError},
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post, put},
};
use serde::Deserialize;

#[derive(Clone)]
struct AgentApi {
    identity: Identity,
    agents: Agents,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(list, create, update))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/agents", get(list).post(create))
        .route("/api/agents/{id}", get(read))
        .route("/api/agents/{id}/status", get(status))
        .route("/api/agents/{id}/definition", post(update))
        .route("/api/agents/{id}/lifecycle", post(lifecycle))
        .route("/api/agents/{id}/revisions", get(revisions))
        .route(
            "/api/agents/{id}/wallpaper",
            put(set_wallpaper)
                .delete(reset_wallpaper)
                .layer(DefaultBodyLimit::max(crate::agents::WALLPAPER_BYTES)),
        )
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

#[utoipa::path(get, path = "/api/agents", params(("after" = Option<uuid::Uuid>, Query)), responses((status = 200, body = Vec<crate::agents::Agent>)))]
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

#[utoipa::path(post, path = "/api/agents", request_body = CreateAgent, responses((status = 201, body = crate::identity::Receipt)))]
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevisionCursor {
    #[serde(default)]
    after: i64,
}

#[utoipa::path(post, path = "/api/agents/{id}/definition", params(("id" = uuid::Uuid, Path)), request_body = UpdateAgent, responses((status = 200, body = crate::identity::Receipt)))]
async fn update(
    State(api): State<AgentApi>,
    headers: HeaderMap,
    Path(id): Path<AgentId>,
    Json(input): Json<UpdateAgent>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.agents.update(&actor, id, &input).await?),
    ))
}

async fn lifecycle(
    State(api): State<AgentApi>,
    headers: HeaderMap,
    Path(id): Path<AgentId>,
    Json(input): Json<LifecycleCommand>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        StatusCode::ACCEPTED,
        [("cache-control", "no-store")],
        Json(api.agents.lifecycle(&actor, id, &input).await?),
    ))
}

async fn revisions(
    State(api): State<AgentApi>,
    headers: HeaderMap,
    Path(id): Path<AgentId>,
    Query(cursor): Query<RevisionCursor>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.agents.revisions(&actor, id, cursor.after).await?),
    ))
}

async fn status(
    State(api): State<AgentApi>,
    headers: HeaderMap,
    Path(id): Path<AgentId>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.agents.status(&actor, id).await?),
    ))
}

/// The raw JPEG or PNG; the image type comes from its bytes, never a header.
async fn set_wallpaper(
    State(api): State<AgentApi>,
    headers: HeaderMap,
    Path(id): Path<AgentId>,
    image: Bytes,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.agents.set_wallpaper(&actor, id, Some(&image)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reset_wallpaper(
    State(api): State<AgentApi>,
    headers: HeaderMap,
    Path(id): Path<AgentId>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.agents.set_wallpaper(&actor, id, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

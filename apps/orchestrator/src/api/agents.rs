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
    routing::{get, post},
};
use serde::Deserialize;

#[derive(Clone)]
struct AgentApi {
    identity: Identity,
    agents: Agents,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(
    list,
    create,
    read,
    update,
    lifecycle,
    revisions,
    status,
    get_wallpaper,
    set_wallpaper,
    reset_wallpaper
))]
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
            get(get_wallpaper)
                .put(set_wallpaper)
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

/// The summary everyone with a capability on the agent sees; `instructions` needs `agent_manage`.
#[utoipa::path(get, path = "/api/agents/{id}", params(("id" = uuid::Uuid, Path)), responses((status = 200, body = crate::agents::Agent)))]
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

#[utoipa::path(post, path = "/api/agents/{id}/lifecycle", params(("id" = uuid::Uuid, Path)), request_body = LifecycleCommand, responses((status = 202, body = crate::identity::Receipt)))]
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

#[utoipa::path(get, path = "/api/agents/{id}/revisions", params(("id" = uuid::Uuid, Path), ("after" = Option<i64>, Query)), responses((status = 200, body = Vec<crate::agents::DefinitionRevision>)))]
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

#[utoipa::path(get, path = "/api/agents/{id}/status", params(("id" = uuid::Uuid, Path)), responses((status = 200, body = crate::agents::AgentStatus)))]
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

/// The raw bytes of a JPEG or PNG; a schema only, the handlers take `Bytes`.
#[derive(utoipa::ToSchema)]
#[schema(value_type = String, format = Binary)]
struct Image(#[allow(dead_code)] Vec<u8>);

/// The agent's wallpaper for everyone who may see the agent; 404 while it uses the default.
#[utoipa::path(get, path = "/api/agents/{id}/wallpaper", params(("id" = uuid::Uuid, Path)), responses((status = 200, content((Image = "image/jpeg"), (Image = "image/png"))), (status = 404)))]
async fn get_wallpaper(
    State(api): State<AgentApi>,
    headers: HeaderMap,
    Path(id): Path<AgentId>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let image = api
        .agents
        .wallpaper(&actor, id)
        .await?
        .ok_or(IdentityError::NotFound)?;
    let content_type = crate::agents::content_type(&image).ok_or(IdentityError::NotFound)?;
    Ok((
        [
            ("cache-control", "no-store"),
            ("content-type", content_type),
        ],
        image,
    ))
}

/// The raw JPEG or PNG; the image type comes from its bytes, never a header.
#[utoipa::path(put, path = "/api/agents/{id}/wallpaper", params(("id" = uuid::Uuid, Path)), request_body(content((Image = "image/jpeg"), (Image = "image/png"))), responses((status = 204)))]
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

#[utoipa::path(delete, path = "/api/agents/{id}/wallpaper", params(("id" = uuid::Uuid, Path)), responses((status = 204)))]
async fn reset_wallpaper(
    State(api): State<AgentApi>,
    headers: HeaderMap,
    Path(id): Path<AgentId>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.agents.set_wallpaper(&actor, id, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

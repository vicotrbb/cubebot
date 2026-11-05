//! The webhook authenticates with the routine's own bearer secret, never a person's session.
use crate::{
    agents::AgentId,
    identity::{Identity, IdentityError},
    work::{
        Work,
        routines::{
            CreateRoutine, CreatedRoutine, HOOK_BODY, HookDelivery, Hooked, Occurrence, Ran,
            Routine, RunRoutine, UpdateRoutine, WebhookSecret,
        },
    },
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use uuid::Uuid;

#[derive(Clone)]
struct Api {
    identity: Identity,
    work: Work,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(
    list,
    create,
    update,
    delete,
    rotate,
    remove_secret,
    run,
    occurrences,
    deliveries
))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    let hooks = Router::new()
        .route("/api/hooks/{routine}", post(hook))
        .layer(DefaultBodyLimit::max(HOOK_BODY))
        .with_state(Work::new(pool.clone()));
    Router::new()
        .route("/api/agents/{agent}/routines", get(list).post(create))
        .route(
            "/api/agents/{agent}/routines/{routine}",
            put(update).delete(delete),
        )
        .route(
            "/api/agents/{agent}/routines/{routine}/secret",
            post(rotate).delete(remove_secret),
        )
        .route(
            "/api/agents/{agent}/routines/{routine}/runs",
            get(occurrences).post(run),
        )
        .route(
            "/api/agents/{agent}/routines/{routine}/deliveries",
            get(deliveries),
        )
        .layer(DefaultBodyLimit::max(32 * 1024))
        .with_state(Api {
            identity,
            work: Work::new(pool),
        })
        .merge(hooks)
}

#[utoipa::path(get, path = "/api/agents/{agent}/routines", operation_id = "list_routines", params(("agent" = Uuid, Path)), responses((status = 200, body = Vec<Routine>)))]
async fn list(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
) -> Result<Json<Vec<Routine>>, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok(Json(api.work.routines(&actor, agent).await?))
}

/// With `webhook`, the response carries the secret; it is never shown again.
#[utoipa::path(post, path = "/api/agents/{agent}/routines", operation_id = "create_routine", params(("agent" = Uuid, Path)), request_body = CreateRoutine, responses((status = 200, body = CreatedRoutine)))]
async fn create(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
    Json(input): Json<CreateRoutine>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let created = api.work.create_routine(&actor, agent, &input).await?;
    Ok(([("cache-control", "no-store")], Json(created)))
}

#[utoipa::path(put, path = "/api/agents/{agent}/routines/{routine}", operation_id = "update_routine", params(("agent" = Uuid, Path), ("routine" = Uuid, Path)), request_body = UpdateRoutine, responses((status = 200, body = Routine)))]
async fn update(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, routine)): Path<(AgentId, Uuid)>,
    Json(input): Json<UpdateRoutine>,
) -> Result<Json<Routine>, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok(Json(
        api.work
            .update_routine(&actor, agent, routine, &input)
            .await?,
    ))
}

/// Cancels the routine's queued turns; its history stays in the conversation.
#[utoipa::path(delete, path = "/api/agents/{agent}/routines/{routine}", operation_id = "delete_routine", params(("agent" = Uuid, Path), ("routine" = Uuid, Path)), responses((status = 204)))]
async fn delete(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, routine)): Path<(AgentId, Uuid)>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.work.delete_routine(&actor, agent, routine).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Makes or replaces the webhook secret, shown only in this response.
#[utoipa::path(post, path = "/api/agents/{agent}/routines/{routine}/secret", operation_id = "rotate_routine_secret", params(("agent" = Uuid, Path), ("routine" = Uuid, Path)), responses((status = 200, body = WebhookSecret)))]
async fn rotate(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, routine)): Path<(AgentId, Uuid)>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let secret = api
        .work
        .rotate_routine_secret(&actor, agent, routine)
        .await?;
    Ok(([("cache-control", "no-store")], Json(secret)))
}

/// Stops the webhook; its URL answers 401 and the routine keeps its other triggers.
#[utoipa::path(delete, path = "/api/agents/{agent}/routines/{routine}/secret", operation_id = "remove_routine_secret", params(("agent" = Uuid, Path), ("routine" = Uuid, Path)), responses((status = 204)))]
async fn remove_secret(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, routine)): Path<(AgentId, Uuid)>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.work
        .remove_routine_secret(&actor, agent, routine)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The webhook's authenticated deliveries of the last 30 days, newest first.
#[utoipa::path(get, path = "/api/agents/{agent}/routines/{routine}/deliveries", operation_id = "routine_deliveries", params(("agent" = Uuid, Path), ("routine" = Uuid, Path)), responses((status = 200, body = Vec<HookDelivery>)))]
async fn deliveries(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, routine)): Path<(AgentId, Uuid)>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let deliveries = api.work.routine_deliveries(&actor, agent, routine).await?;
    Ok(([("cache-control", "no-store")], Json(deliveries)))
}

#[utoipa::path(post, path = "/api/agents/{agent}/routines/{routine}/runs", operation_id = "run_routine", params(("agent" = Uuid, Path), ("routine" = Uuid, Path)), request_body = RunRoutine, responses((status = 202, body = Ran)))]
async fn run(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, routine)): Path<(AgentId, Uuid)>,
    Json(input): Json<RunRoutine>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let ran = api.work.run_routine(&actor, agent, routine, &input).await?;
    Ok((StatusCode::ACCEPTED, Json(ran)))
}

#[utoipa::path(get, path = "/api/agents/{agent}/routines/{routine}/runs", operation_id = "routine_occurrences", params(("agent" = Uuid, Path), ("routine" = Uuid, Path)), responses((status = 200, body = Vec<Occurrence>)))]
async fn occurrences(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, routine)): Path<(AgentId, Uuid)>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let occurrences = api.work.routine_occurrences(&actor, agent, routine).await?;
    Ok(([("cache-control", "no-store")], Json(occurrences)))
}

async fn hook(
    State(work): State<Work>,
    Path(routine): Path<Uuid>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, IdentityError> {
    let secret = super::bearer(&headers).ok_or(IdentityError::Unauthorized)?;
    let key = match headers.get("idempotency-key") {
        Some(key) => Some(key.to_str().map_err(|_| IdentityError::InvalidInput)?),
        None => None,
    };
    Ok(match work.hook(routine, secret, key, &body).await? {
        Hooked::Accepted(turn) => (StatusCode::ACCEPTED, Json(Ran { turn })).into_response(),
        Hooked::Replayed(turn) => (StatusCode::OK, Json(Ran { turn })).into_response(),
        Hooked::Limited => IdentityError::Busy.into_response(),
    })
}

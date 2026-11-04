//! Tasks for people: what they asked for, how far it got and the orchestrator's settings. A task's
//! conversation goes through its room's messages and events.
use crate::{
    identity::{Identity, IdentityError, audit::Event},
    work::{
        Work,
        tasks::{CreateTask, Task, TaskSettings},
    },
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use uuid::Uuid;

#[derive(Clone)]
struct Api {
    identity: Identity,
    work: Work,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(list, create, one, stop, settings, save_settings))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/tasks", get(list).post(create))
        .route("/api/tasks/settings", get(settings).put(save_settings))
        .route("/api/tasks/{task}", get(one))
        .route("/api/tasks/{task}/stop", post(stop))
        .with_state(Api {
            identity,
            work: Work::new(pool),
        })
}

/// The caller's tasks, or every task for an administrator, most recently active first.
#[utoipa::path(get, path = "/api/tasks", operation_id = "list_tasks", responses((status = 200, body = Vec<Task>)))]
async fn list(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.work.tasks(&actor).await?),
    ))
}

/// Records what the person wants; the orchestrator picks it up. Repeating `request_id` returns the
/// same task. 409 when the caller has no agent to hand it to.
#[utoipa::path(post, path = "/api/tasks", operation_id = "create_task", request_body = CreateTask, responses((status = 202, body = Task)))]
async fn create(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<CreateTask>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let task = api.work.create_task(&actor, &input).await?;
    let event = Event::by(actor.id, "task.create").on(task.id);
    api.identity.audit(event).await;
    Ok((
        StatusCode::ACCEPTED,
        [("cache-control", "no-store")],
        Json(task),
    ))
}

#[utoipa::path(get, path = "/api/tasks/{task}", operation_id = "get_task", params(("task" = Uuid, Path)), responses((status = 200, body = Task)))]
async fn one(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(task): Path<Uuid>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.work.task(&actor, task).await?),
    ))
}

/// Cancels the task's open turns. Writing in its room starts it again.
#[utoipa::path(post, path = "/api/tasks/{task}/stop", operation_id = "stop_task", params(("task" = Uuid, Path)), responses((status = 200, body = Task)))]
async fn stop(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(task): Path<Uuid>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let stopped = api.work.stop_task(&actor, task).await?;
    let event = Event::by(actor.id, "task.stop").on(task);
    api.identity.audit(event).await;
    Ok(([("cache-control", "no-store")], Json(stopped)))
}

/// Administrators only.
#[utoipa::path(get, path = "/api/tasks/settings", operation_id = "task_settings", responses((status = 200, body = TaskSettings)))]
async fn settings(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.work.task_settings(&actor).await?),
    ))
}

/// Administrators only; 409 when `revision` is not the current one.
#[utoipa::path(put, path = "/api/tasks/settings", operation_id = "save_task_settings", request_body = TaskSettings, responses((status = 200, body = TaskSettings)))]
async fn save_settings(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<TaskSettings>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let saved = api.work.save_task_settings(&actor, &input).await?;
    let event = Event::by(actor.id, "task.settings");
    api.identity.audit(event).await;
    Ok(([("cache-control", "no-store")], Json(saved)))
}

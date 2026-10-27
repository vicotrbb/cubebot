use crate::{
    agents::AgentId,
    identity::{Identity, IdentityError},
    work::{SendMessage, Work},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Clone)]
struct Api {
    identity: Identity,
    work: Work,
}

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/agents/{agent}/conversation", get(conversation))
        .route(
            "/api/conversations/{conversation}/messages",
            get(history).post(send),
        )
        .route(
            "/api/conversations/{conversation}/messages/{turn}/retry",
            post(retry),
        )
        .route(
            "/api/conversations/{conversation}/messages/{turn}/cancel",
            post(cancel),
        )
        .layer(DefaultBodyLimit::max(72 * 1024))
        .with_state(Api {
            identity,
            work: Work::new(pool),
        })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryCursor {
    #[serde(default)]
    after: i64,
}

async fn conversation(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.work.conversation(&actor, agent).await?),
    ))
}
async fn history(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(conversation): Path<Uuid>,
    Query(cursor): Query<HistoryCursor>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.work.history(&actor, conversation, cursor.after).await?),
    ))
}
async fn send(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(conversation): Path<Uuid>,
    Json(input): Json<SendMessage>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        StatusCode::ACCEPTED,
        [("cache-control", "no-store")],
        Json(api.work.send(&actor, conversation, &input).await?),
    ))
}

async fn retry(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((conversation, turn)): Path<(Uuid, Uuid)>,
    Json(input): Json<crate::work::RetryMessage>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.work.retry(&actor, conversation, turn, &input).await?),
    ))
}

async fn cancel(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((conversation, turn)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.work.cancel(&actor, conversation, turn).await?;
    Ok(StatusCode::NO_CONTENT)
}

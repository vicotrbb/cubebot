use crate::{
    agents::AgentId,
    identity::{Identity, IdentityError},
    work::{SendMessage, Work},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse,
        sse::{self, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures_util::{StreamExt, stream};
use serde::Deserialize;
use std::{collections::HashMap, convert::Infallible, time::Duration};
use uuid::Uuid;

#[derive(Clone)]
struct Api {
    identity: Identity,
    work: Work,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(history, send, retry, cancel, steer, events))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/agents/{agent}/conversation", get(conversation))
        .route(
            "/api/conversations/{conversation}/messages",
            get(history).post(send),
        )
        .route("/api/conversations/{conversation}/events", get(events))
        .route(
            "/api/conversations/{conversation}/messages/{turn}/retry",
            post(retry),
        )
        .route(
            "/api/conversations/{conversation}/messages/{turn}/cancel",
            post(cancel),
        )
        .route(
            "/api/conversations/{conversation}/messages/{turn}/steer",
            post(steer),
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
#[utoipa::path(get, path = "/api/conversations/{conversation}/messages", params(("conversation" = Uuid, Path), ("after" = Option<i64>, Query)), responses((status = 200, body = Vec<crate::work::Turn>)))]
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
#[utoipa::path(post, path = "/api/conversations/{conversation}/messages", params(("conversation" = Uuid, Path)), request_body = SendMessage, responses((status = 202, body = crate::work::Turn)))]
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

#[utoipa::path(post, path = "/api/conversations/{conversation}/messages/{turn}/retry", params(("conversation" = Uuid, Path), ("turn" = Uuid, Path)), request_body = crate::work::RetryMessage, responses((status = 200, body = crate::work::Turn)))]
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

/// Cancels the turn and everything delegated under it; each agent's channel delivers the
/// cancellation to the turn it is running.
#[utoipa::path(post, path = "/api/conversations/{conversation}/messages/{turn}/cancel", params(("conversation" = Uuid, Path), ("turn" = Uuid, Path)), responses((status = 204)))]
async fn cancel(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((conversation, turn)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.work.cancel(&actor, conversation, turn).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Queues a correction for this turn, incorporated before its next model request. Retrying
/// the same `request_id` returns the same instruction.
#[utoipa::path(post, path = "/api/conversations/{conversation}/messages/{turn}/steer", params(("conversation" = Uuid, Path), ("turn" = Uuid, Path)), request_body = SendMessage, responses((status = 202, body = crate::work::Steering)))]
async fn steer(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((conversation, turn)): Path<(Uuid, Uuid)>,
    Json(input): Json<SendMessage>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        StatusCode::ACCEPTED,
        [("cache-control", "no-store")],
        Json(api.work.steer(&actor, conversation, turn, &input).await?),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventCursor {
    after: Option<i64>,
}

/// Replays turn events after the cursor, then follows live. `turn_event` frames carry an
/// `Event`; `turn` frames carry a `Turn` snapshot whenever its state may have changed.
#[utoipa::path(get, path = "/api/conversations/{conversation}/events", params(("conversation" = Uuid, Path), ("after" = Option<i64>, Query), ("Last-Event-ID" = Option<i64>, Header)), responses((status = 200, content_type = "text/event-stream", body = crate::work::Event)))]
async fn events(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(conversation): Path<Uuid>,
    Query(cursor): Query<EventCursor>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let resume = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok()?.parse().ok());
    let after = match cursor.after.or(resume) {
        Some(after) => after,
        None => api.work.event_cursor(&actor, conversation).await?,
    };
    let mut follow = Follow {
        api,
        headers,
        conversation,
        after,
        full: false,
        sent: HashMap::new(),
    };
    // The first poll runs before responding so a refusal is an HTTP status, not a closed stream.
    let first = follow.poll().await?;
    let live = stream::unfold(follow, |mut follow| async move {
        if !follow.full {
            // shortcut: polls the ledger twice a second per open stream; use LISTEN/NOTIFY if
            // measured database load requires it.
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        // Ending the stream makes the browser reconnect, which reports a refusal as a status.
        let frames = follow.poll().await.ok()?;
        Some((frames, follow))
    });
    let frames = stream::iter([first])
        .chain(live)
        .flat_map(stream::iter)
        .map(Ok::<_, Infallible>);
    Ok((
        [("cache-control", "no-store")],
        Sse::new(frames).keep_alive(KeepAlive::default()),
    ))
}

struct Follow {
    api: Api,
    headers: HeaderMap,
    conversation: Uuid,
    after: i64,
    /// The last poll hit its page limit, so more events are already waiting.
    full: bool,
    /// Last sent snapshot of each unfinished turn.
    sent: HashMap<Uuid, String>,
}

impl Follow {
    /// Reauthenticates each poll, so signing out or losing access ends the stream.
    async fn poll(&mut self) -> Result<Vec<sse::Event>, IdentityError> {
        let actor = super::identity::actor(&self.api.identity, &self.headers).await?;
        let touched: Vec<Uuid> = self.sent.keys().copied().collect();
        let page = self
            .api
            .work
            .events(&actor, self.conversation, self.after, &touched)
            .await?;
        self.full = page.events.len() == 100;
        let mut frames = Vec::new();
        for event in page.events {
            self.after = event.id;
            let json = serde_json::to_string(&event).unwrap_or_default();
            frames.push(
                sse::Event::default()
                    .id(event.id.to_string())
                    .event("turn_event")
                    .data(json),
            );
        }
        for turn in page.turns {
            let json = serde_json::to_string(&turn).unwrap_or_default();
            if self.sent.get(&turn.id) != Some(&json) {
                frames.push(sse::Event::default().event("turn").data(&json));
            }
            if matches!(turn.status.as_str(), "complete" | "failed" | "cancelled") {
                self.sent.remove(&turn.id);
            } else {
                self.sent.insert(turn.id, json);
            }
        }
        Ok(frames)
    }
}

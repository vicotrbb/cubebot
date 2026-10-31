use crate::{
    identity::{Identity, IdentityError},
    work::{CreateRoom, RoomMembers, SendMessage, Work},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse,
        sse::{self, KeepAlive, Sse},
    },
    routing::{get, put},
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
#[openapi(paths(
    list_rooms,
    create_room,
    set_members,
    room_messages,
    post_room_message,
    room_events
))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/rooms", get(list_rooms).post(create_room))
        .route("/api/rooms/{room}/members", put(set_members))
        .route("/api/rooms/{room}/events", get(room_events))
        .route(
            "/api/rooms/{room}/messages",
            get(room_messages).post(post_room_message),
        )
        .layer(DefaultBodyLimit::max(72 * 1024))
        .with_state(Api {
            identity,
            work: Work::new(pool),
        })
}

#[utoipa::path(get, path = "/api/rooms", responses((status = 200, body = Vec<crate::work::Room>)))]
async fn list_rooms(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.work.rooms(&actor).await?),
    ))
}

/// Administrators create rooms; `request_id` makes a retry return the same room.
#[utoipa::path(post, path = "/api/rooms", request_body = CreateRoom, responses((status = 201, body = crate::work::Room)))]
async fn create_room(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<CreateRoom>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        StatusCode::CREATED,
        [("cache-control", "no-store")],
        Json(api.work.create_room(&actor, &input).await?),
    ))
}

/// Replaces the member set; administrators and `room.manage` holders only.
#[utoipa::path(put, path = "/api/rooms/{room}/members", params(("room" = Uuid, Path)), request_body = RoomMembers, responses((status = 200, body = crate::work::Room)))]
async fn set_members(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(room): Path<Uuid>,
    Json(input): Json<RoomMembers>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.work.set_room_members(&actor, room, &input).await?),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    #[serde(default)]
    after: i64,
}

#[utoipa::path(get, path = "/api/rooms/{room}/messages", params(("room" = Uuid, Path), ("after" = Option<i64>, Query)), responses((status = 200, body = Vec<crate::work::RoomMessage>)))]
async fn room_messages(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(room): Path<Uuid>,
    Query(cursor): Query<Cursor>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.work.room_messages(&actor, room, cursor.after).await?),
    ))
}

/// `@name` mentions of members deliver to each of them; otherwise the coordinator routes it.
#[utoipa::path(post, path = "/api/rooms/{room}/messages", params(("room" = Uuid, Path)), request_body = SendMessage, responses((status = 202, body = crate::work::RoomMessage)))]
async fn post_room_message(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(room): Path<Uuid>,
    Json(input): Json<SendMessage>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        StatusCode::ACCEPTED,
        [("cache-control", "no-store")],
        Json(api.work.post_room_message(&actor, room, &input).await?),
    ))
}

/// Follows the room: `message` frames carry a `RoomMessage` snapshot whenever it is new or its
/// routing or deliveries changed. Starts after `after` or `Last-Event-ID`, the last message
/// sequence seen; a reconnecting client reloads earlier messages itself.
#[utoipa::path(get, path = "/api/rooms/{room}/events", params(("room" = Uuid, Path), ("after" = Option<i64>, Query), ("Last-Event-ID" = Option<i64>, Header)), responses((status = 200, content_type = "text/event-stream", body = crate::work::RoomMessage)))]
async fn room_events(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(room): Path<Uuid>,
    Query(cursor): Query<Cursor>,
) -> Result<impl IntoResponse, IdentityError> {
    let resume = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok()?.parse().ok());
    let mut follow = Follow {
        api,
        headers,
        room,
        after: resume.unwrap_or(cursor.after),
        sent: HashMap::new(),
    };
    // The first poll runs before responding so a refusal is an HTTP status, not a closed stream.
    let first = follow.poll().await?;
    let live = stream::unfold(follow, |mut follow| async move {
        // shortcut: polls the ledger once a second per open room; use LISTEN/NOTIFY if
        // measured database load requires it.
        tokio::time::sleep(Duration::from_secs(1)).await;
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
    room: Uuid,
    after: i64,
    /// Last sent snapshot of each message that can still change.
    sent: HashMap<Uuid, String>,
}

impl Follow {
    /// Reauthenticates each poll, so signing out or losing access ends the stream.
    async fn poll(&mut self) -> Result<Vec<sse::Event>, IdentityError> {
        let actor = super::identity::actor(&self.api.identity, &self.headers).await?;
        let touched: Vec<Uuid> = self.sent.keys().copied().collect();
        let messages = self
            .api
            .work
            .room_feed(&actor, self.room, self.after, &touched)
            .await?;
        let mut frames = Vec::new();
        for message in messages {
            self.after = self.after.max(message.sequence);
            let json = serde_json::to_string(&message).unwrap_or_default();
            if self.sent.get(&message.id) != Some(&json) {
                frames.push(
                    sse::Event::default()
                        .id(self.after.to_string())
                        .event("message")
                        .data(&json),
                );
            }
            if message.settled() {
                self.sent.remove(&message.id);
            } else {
                self.sent.insert(message.id, json);
            }
        }
        Ok(frames)
    }
}

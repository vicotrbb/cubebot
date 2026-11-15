use crate::{
    identity::{Identity, IdentityError},
    meetings::{
        Meetings, PublicMeeting,
        live::{Entry, wire::FRAME_LIMIT},
    },
};
use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, Path, State, ws::WebSocketUpgrade},
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::get,
};
use std::net::SocketAddr;
use std::time::Instant;

type Peer = Option<Extension<ConnectInfo<SocketAddr>>>;

#[derive(Clone)]
struct Api {
    meetings: Meetings,
    console: Option<String>,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(lookup))]
pub(super) struct Doc;

pub(super) fn router(identity: &Identity, meetings: Meetings) -> Router {
    Router::new()
        .route("/api/public/meetings/{token}", get(lookup))
        .route("/api/public/meetings/{token}/live", get(live))
        .with_state(Api {
            meetings,
            console: identity.browser_origin.clone(),
        })
}

pub(super) fn origin_allowed(
    headers: &HeaderMap,
    console: Option<&str>,
    meetings: &Meetings,
    required: bool,
) -> bool {
    let Some(origin) = headers.get("origin") else {
        return !required;
    };
    let origin = origin.to_str().ok();
    origin.is_some() && (origin == console || origin == meetings.origin())
}

fn known<T>(meetings: &Meetings, found: Option<T>) -> Result<T, IdentityError> {
    match found {
        Some(found) => Ok(found),
        None if meetings.limiter().unknown(Instant::now()) => Err(IdentityError::NotFound),
        None => Err(IdentityError::Busy),
    }
}

#[utoipa::path(get, path = "/api/public/meetings/{token}", operation_id = "public_meeting", params(("token" = String, Path)), responses((status = 200, body = PublicMeeting), (status = 404), (status = 429)))]
async fn lookup(
    State(api): State<Api>,
    peer: Peer,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Result<impl IntoResponse, IdentityError> {
    if !origin_allowed(&headers, api.console.as_deref(), &api.meetings, false) {
        return Err(IdentityError::Forbidden);
    }
    let client = api.meetings.client(peer.map(|peer| peer.0.0), &headers);
    if !api.meetings.limiter().lookup(client, Instant::now()) {
        return Err(IdentityError::Busy);
    }
    let found = api.meetings.public_meeting(&token).await?;
    let meeting = known(&api.meetings, found)?;
    Ok(([("cache-control", "no-store")], Json(meeting)))
}

#[tracing::instrument(skip_all)]
async fn live(
    State(api): State<Api>,
    peer: Peer,
    headers: HeaderMap,
    Path(token): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, IdentityError> {
    if !origin_allowed(&headers, api.console.as_deref(), &api.meetings, true) {
        return Err(IdentityError::Forbidden);
    }
    let client = api.meetings.client(peer.map(|peer| peer.0.0), &headers);
    if !api.meetings.limiter().join(client, Instant::now()) {
        return Err(IdentityError::Busy);
    }
    let found = api.meetings.guest_meeting(&token).await?;
    let id = known(&api.meetings, found)?;
    let live = api.meetings.live().clone();
    let entry = Entry::Guest { token };
    Ok(upgrade
        .max_message_size(FRAME_LIMIT)
        .max_frame_size(FRAME_LIMIT)
        .on_upgrade(move |socket| live.attend(socket, id, entry)))
}

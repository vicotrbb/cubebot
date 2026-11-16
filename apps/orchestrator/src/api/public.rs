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

pub(super) fn guarded(router: Router, console: Option<&str>, meetings: Option<&str>) -> Router {
    match (meetings.map(host_key), console.map(host_key)) {
        (Some(meetings), console) if console.as_ref() != Some(&meetings) => {
            router.layer(axum::middleware::from_fn_with_state(meetings, public_only))
        }
        _ => router,
    }
}

async fn public_only(
    State(meetings): State<String>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let hosts = [
        request
            .headers()
            .get(axum::http::header::HOST)
            .and_then(|host| host.to_str().ok()),
        request
            .uri()
            .authority()
            .map(|authority| authority.as_str()),
    ];
    if refused(&meetings, &hosts, request.uri().path()) {
        return IdentityError::NotFound.into_response();
    }
    next.run(request).await
}

fn refused(meetings: &str, hosts: &[Option<&str>], path: &str) -> bool {
    let public = path == "/api/public/meetings" || path.starts_with("/api/public/meetings/");
    !public
        && hosts
            .iter()
            .flatten()
            .any(|host| host_key(host) == meetings)
}

fn host_key(value: &str) -> String {
    let value = value.split_once("://").map_or(value, |(_, rest)| rest);
    let host = value
        .split('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let host = host
        .strip_suffix(":443")
        .or_else(|| host.strip_suffix(":80"))
        .unwrap_or(&host);
    host.trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::{host_key, refused};

    const MEETINGS: &str = "meetings.example.com";

    #[test]
    fn hosts_compare_without_case_default_port_or_trailing_dot() {
        for value in [
            "https://Meetings.Example.com",
            "https://meetings.example.com/",
            "MEETINGS.example.com:443",
            "meetings.example.com:80",
            "meetings.example.com.",
        ] {
            assert_eq!(host_key(value), MEETINGS, "{value}");
        }
        assert_eq!(
            host_key("meetings.example.com:8443"),
            "meetings.example.com:8443"
        );
    }

    #[test]
    fn the_meetings_host_answers_only_the_public_meeting_routes() {
        let on = [Some("Meetings.Example.com:443"), None];
        assert!(!refused(MEETINGS, &on, "/api/public/meetings"));
        assert!(!refused(MEETINGS, &on, "/api/public/meetings/token"));
        assert!(!refused(MEETINGS, &on, "/api/public/meetings/token/live"));
        for path in [
            "/api/identity/me",
            "/api/public/meetingsx",
            "/api/public",
            "/m/token",
            "/",
        ] {
            assert!(refused(MEETINGS, &on, path), "{path}");
        }
        assert!(refused(
            MEETINGS,
            &[None, Some("meetings.example.com")],
            "/api/agents"
        ));
        assert!(refused(
            MEETINGS,
            &[Some("console.example.com"), Some(MEETINGS)],
            "/api/agents"
        ));
    }

    #[test]
    fn other_hosts_are_unaffected() {
        for hosts in [
            [Some("console.example.com"), None],
            [Some("cubebot-orchestrator.cubebot.svc:8443"), None],
            [None, None],
        ] {
            assert!(!refused(MEETINGS, &hosts, "/api/identity/me"));
        }
    }
}

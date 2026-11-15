use crate::{
    agents::runtime::{RuntimeAccess, RuntimeIdentity},
    identity::{Identity, IdentityError, audit::Event},
    meetings::{
        ChangeMeeting, EmailConnection, JoinedMeeting, Meeting, MeetingLink, MeetingSettings,
        MeetingSummary, Meetings, NewInvitations, NewMeeting, SaveMeetingSettings, SetLink,
        live::{Entry, Recheck, wire::FRAME_LIMIT},
    },
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State, ws::WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use cubebot_protocol::runtime::MeetingScreen;
use serde_json::json;
use uuid::Uuid;

#[derive(Clone)]
struct Api {
    identity: Identity,
    meetings: Meetings,
    runtime: RuntimeIdentity,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(
    list,
    create,
    read,
    update,
    cancel,
    invite,
    uninvite,
    link,
    email_connections,
    read_settings,
    save_settings,
    join_link
))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity, meetings: Meetings) -> Router {
    let runtime = RuntimeIdentity::docker(pool);
    Router::new()
        .route("/api/meetings", get(list).post(create))
        .route(
            "/api/meetings/settings",
            get(read_settings).put(save_settings),
        )
        .route("/api/meetings/email-connections", get(email_connections))
        .route("/api/meetings/{id}", get(read).patch(update).delete(cancel))
        .route("/api/meetings/{id}/invitations", post(invite))
        .route(
            "/api/meetings/{id}/invitations/{invitation}",
            delete(uninvite),
        )
        .route("/api/meetings/{id}/link", post(link))
        .route("/api/meetings/{id}/live", get(live))
        .route("/api/meetings/link/{token}/join", post(join_link))
        .route(
            "/api/runtime/turns/{turn}/meeting-screen",
            post(agent_screen),
        )
        .layer(DefaultBodyLimit::max(32 * 1024))
        .with_state(Api {
            identity,
            meetings,
            runtime,
        })
}

#[utoipa::path(get, path = "/api/meetings", operation_id = "list_meetings", responses((status = 200, body = Vec<MeetingSummary>)))]
async fn list(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let meetings = api.meetings.list(&actor).await?;
    Ok(([("cache-control", "no-store")], Json(meetings)))
}

#[utoipa::path(post, path = "/api/meetings", operation_id = "create_meeting", request_body = NewMeeting, responses((status = 200, body = Meeting), (status = 400), (status = 403), (status = 404), (status = 409)))]
async fn create(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<NewMeeting>,
) -> Result<Json<Meeting>, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let (meeting, invited) = api.meetings.create(&actor, &input).await?;
    let detail = json!({"name": meeting.title, "scheduled": meeting.starts_at.is_some()});
    let created = Event::by(actor.id, "meeting.create").on(meeting.id);
    api.identity.audit(created.detail(detail)).await;
    if invited.accounts + invited.agents + invited.emails > 0 {
        let event = Event::by(actor.id, "meeting.invite").on(meeting.id);
        api.identity.audit(event.detail(json!(invited))).await;
    }
    Ok(Json(meeting))
}

#[utoipa::path(get, path = "/api/meetings/{id}", operation_id = "read_meeting", params(("id" = Uuid, Path)), responses((status = 200, body = Meeting), (status = 404)))]
async fn read(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let meeting = api.meetings.meeting(&actor, id).await?;
    Ok(([("cache-control", "no-store")], Json(meeting)))
}

#[utoipa::path(patch, path = "/api/meetings/{id}", operation_id = "update_meeting", params(("id" = Uuid, Path)), request_body = ChangeMeeting, responses((status = 204), (status = 400), (status = 403), (status = 404)))]
async fn update(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<ChangeMeeting>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let title = api.meetings.update(&actor, id, &input).await?;
    let event = Event::by(actor.id, "meeting.update").on(id);
    api.identity
        .audit(event.detail(json!({"name": title})))
        .await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/meetings/{id}", operation_id = "cancel_meeting", params(("id" = Uuid, Path)), responses((status = 204), (status = 403), (status = 404), (status = 409)))]
async fn cancel(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let title = api.meetings.cancel(&actor, id).await?;
    let event = Event::by(actor.id, "meeting.cancel").on(id);
    api.identity
        .audit(event.detail(json!({"name": title})))
        .await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/meetings/{id}/invitations", operation_id = "invite_to_meeting", params(("id" = Uuid, Path)), request_body = NewInvitations, responses((status = 204), (status = 400), (status = 403), (status = 404), (status = 409)))]
async fn invite(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<NewInvitations>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let invited = api.meetings.invite(&actor, id, &input).await?;
    let event = Event::by(actor.id, "meeting.invite").on(id);
    api.identity.audit(event.detail(json!(invited))).await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/meetings/{id}/invitations/{invitation}", operation_id = "remove_meeting_invitation", params(("id" = Uuid, Path), ("invitation" = Uuid, Path)), responses((status = 204), (status = 403), (status = 404)))]
async fn uninvite(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((id, invitation)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.meetings.uninvite(&actor, id, invitation).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/meetings/{id}/link", operation_id = "set_meeting_link", params(("id" = Uuid, Path)), request_body = SetLink, responses((status = 200, body = MeetingLink), (status = 403), (status = 404)))]
async fn link(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<SetLink>,
) -> Result<Json<MeetingLink>, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let (link, title) = api.meetings.set_link(&actor, id, input.enabled).await?;
    let detail = json!({"name": title, "enabled": input.enabled});
    let event = Event::by(actor.id, "meeting.link").on(id);
    api.identity.audit(event.detail(detail)).await;
    Ok(Json(link))
}

#[utoipa::path(get, path = "/api/meetings/email-connections", operation_id = "meeting_email_connections", responses((status = 200, body = Vec<EmailConnection>)))]
async fn email_connections(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let connections = api.meetings.email_connections(&actor).await?;
    Ok(([("cache-control", "no-store")], Json(connections)))
}

#[utoipa::path(get, path = "/api/meetings/settings", operation_id = "read_meeting_settings", responses((status = 200, body = MeetingSettings)))]
async fn read_settings(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    super::identity::actor(&api.identity, &headers).await?;
    let settings = api.meetings.settings().await?;
    Ok(([("cache-control", "no-store")], Json(settings)))
}

#[utoipa::path(put, path = "/api/meetings/settings", operation_id = "save_meeting_settings", request_body = SaveMeetingSettings, responses((status = 204), (status = 400), (status = 403)))]
async fn save_settings(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<SaveMeetingSettings>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.meetings.save_settings(&actor, &input).await?;
    let event = Event::by(actor.id, "meeting.configure").detail(json!(input));
    api.identity.audit(event).await;
    Ok(StatusCode::NO_CONTENT)
}

#[tracing::instrument(skip_all, fields(meeting = %id))]
async fn live(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, IdentityError> {
    let token = super::browser::credential(&api.identity, &headers)?.to_owned();
    let console = api.identity.browser_origin.as_deref();
    if !headers.contains_key("authorization")
        && !super::public::origin_allowed(&headers, console, &api.meetings, true)
    {
        return Err(IdentityError::Forbidden);
    }
    let actor = api.identity.authenticate(&token).await?;
    let name = api.meetings.attendee(&actor, id).await?;
    let recheck = Recheck {
        identity: api.identity.clone(),
        token,
    };
    let entry = Entry::Member {
        account: actor.id,
        name,
        recheck,
    };
    let live = api.meetings.live().clone();
    Ok(upgrade
        .max_message_size(FRAME_LIMIT)
        .max_frame_size(FRAME_LIMIT)
        .on_upgrade(move |socket| live.attend(socket, id, entry)))
}

#[utoipa::path(post, path = "/api/meetings/link/{token}/join", operation_id = "join_meeting_by_link", params(("token" = String, Path)), responses((status = 200, body = JoinedMeeting), (status = 404), (status = 409)))]
async fn join_link(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Result<Json<JoinedMeeting>, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let id = api.meetings.join_by_link(&actor, &token).await?;
    Ok(Json(JoinedMeeting { id }))
}

async fn agent_screen(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(turn): Path<Uuid>,
    Json(input): Json<MeetingScreen>,
) -> Result<Response, super::runtime::Failure> {
    let bearer =
        super::bearer(&headers).ok_or(super::runtime::Failure(StatusCode::UNAUTHORIZED))?;
    let actor = api
        .runtime
        .authenticate(bearer, RuntimeAccess::Active)
        .await?;
    let live = api.meetings.live();
    let answer = live.agent_screen(&actor, turn, input.share).await;
    Ok(match answer {
        Ok(Ok(_)) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(reason)) => super::refused(reason),
        Err(error) => {
            tracing::warn!(?error, "meeting screen request failed");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    })
}

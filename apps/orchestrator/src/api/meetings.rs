use crate::{
    identity::{Identity, IdentityError, audit::Event},
    meetings::{
        CreateMeeting, CreatedMeetingLink, GuestAccess, GuestJoin, InviteMeetingAgent, Meeting,
        MeetingDetail, MeetingParticipant, MeetingSettings, Meetings, NewMeetingLink,
        TranscriptSegment,
    },
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post},
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

#[derive(Clone)]
struct Api {
    identity: Identity,
    meetings: Meetings,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(
    meeting_settings,
    save_meeting_settings,
    list_meetings,
    create_meeting,
    read_meeting,
    end_meeting,
    create_meeting_link,
    revoke_meeting_link,
    invite_meeting_agent,
    admit_guest,
    deny_guest,
    meeting_transcript,
    guest_join,
    guest_refresh
))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route(
            "/api/meetings/settings",
            get(meeting_settings).put(save_meeting_settings),
        )
        .route("/api/meetings", get(list_meetings).post(create_meeting))
        .route("/api/meetings/{meeting}", get(read_meeting))
        .route("/api/meetings/{meeting}/end", post(end_meeting))
        .route("/api/meetings/{meeting}/links", post(create_meeting_link))
        .route(
            "/api/meetings/{meeting}/links/{link}",
            delete(revoke_meeting_link),
        )
        .route("/api/meetings/{meeting}/agents", post(invite_meeting_agent))
        .route(
            "/api/meetings/{meeting}/participants/{participant}/admit",
            post(admit_guest),
        )
        .route(
            "/api/meetings/{meeting}/participants/{participant}/deny",
            post(deny_guest),
        )
        .route(
            "/api/meetings/{meeting}/transcript",
            get(meeting_transcript),
        )
        .route("/api/meeting-guests/join", post(guest_join))
        .route("/api/meeting-guests/refresh", post(guest_refresh))
        .layer(DefaultBodyLimit::max(8 * 1024))
        .with_state(Api {
            identity,
            meetings: Meetings::new(pool),
        })
}

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct After {
    /// Only segments after this sequence number; 0 for the start.
    #[serde(default)]
    after: i64,
}

/// The Meetings tab's installation defaults.
#[utoipa::path(get, path = "/api/meetings/settings", responses((status = 200, body = MeetingSettings)))]
async fn meeting_settings(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.meetings.settings().await?),
    ))
}

/// Administrators only.
#[utoipa::path(put, path = "/api/meetings/settings", request_body = MeetingSettings, responses((status = 204)))]
async fn save_meeting_settings(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<MeetingSettings>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.meetings.save_settings(&actor, &input).await?;
    let detail = json!({
        "lobby_default": input.lobby_default,
        "consent_notice": input.consent_notice,
        "agent_mode": input.agent_mode,
    });
    let event = Event::by(actor.id, "meeting.configure").detail(detail);
    api.identity.audit(event).await;
    Ok(StatusCode::NO_CONTENT)
}

/// The caller's own meetings, newest first.
#[utoipa::path(get, path = "/api/meetings", responses((status = 200, body = Vec<Meeting>)))]
async fn list_meetings(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.meetings.list(&actor).await?),
    ))
}

#[utoipa::path(post, path = "/api/meetings", request_body = CreateMeeting, responses((status = 201, body = Meeting)))]
async fn create_meeting(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<CreateMeeting>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let meeting = api.meetings.create(&actor, &input).await?;
    let event = Event::by(actor.id, "meeting.create")
        .on(meeting.id)
        .detail(json!({ "title": meeting.title }));
    api.identity.audit(event).await;
    Ok((
        StatusCode::CREATED,
        [("cache-control", "no-store")],
        Json(meeting),
    ))
}

/// With its participants and links; a link's token is never repeated.
#[utoipa::path(get, path = "/api/meetings/{meeting}", params(("meeting" = Uuid, Path)), responses((status = 200, body = MeetingDetail), (status = 404)))]
async fn read_meeting(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(meeting): Path<Uuid>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.meetings.get(&actor, meeting).await?),
    ))
}

/// For good: nobody can join afterwards and nothing more is recorded.
#[utoipa::path(post, path = "/api/meetings/{meeting}/end", params(("meeting" = Uuid, Path)), responses((status = 204), (status = 404), (status = 409)))]
async fn end_meeting(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(meeting): Path<Uuid>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.meetings.end(&actor, meeting).await?;
    api.identity
        .audit(Event::by(actor.id, "meeting.end").on(meeting))
        .await;
    Ok(StatusCode::NO_CONTENT)
}

/// The response carries the link's token once; it cannot be read back.
#[utoipa::path(post, path = "/api/meetings/{meeting}/links", params(("meeting" = Uuid, Path)), request_body = NewMeetingLink, responses((status = 201, body = CreatedMeetingLink), (status = 404), (status = 409)))]
async fn create_meeting_link(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(meeting): Path<Uuid>,
    Json(input): Json<NewMeetingLink>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let created = api.meetings.create_link(&actor, meeting, &input).await?;
    let detail = json!({ "link": created.link.id, "lobby": created.link.lobby });
    let event = Event::by(actor.id, "meeting.link_create")
        .on(meeting)
        .detail(detail);
    api.identity.audit(event).await;
    Ok((
        StatusCode::CREATED,
        [("cache-control", "no-store")],
        Json(created),
    ))
}

/// Stops new guests and ends the access of those who came in through the link.
#[utoipa::path(delete, path = "/api/meetings/{meeting}/links/{link}", params(("meeting" = Uuid, Path), ("link" = Uuid, Path)), responses((status = 204), (status = 404)))]
async fn revoke_meeting_link(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((meeting, link)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.meetings.revoke_link(&actor, meeting, link).await?;
    let event = Event::by(actor.id, "meeting.link_revoke")
        .on(meeting)
        .detail(json!({ "link": link }));
    api.identity.audit(event).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Needs `conversation.send` on the agent, as a call does.
#[utoipa::path(post, path = "/api/meetings/{meeting}/agents", params(("meeting" = Uuid, Path)), request_body = InviteMeetingAgent, responses((status = 201, body = MeetingParticipant), (status = 404), (status = 409)))]
async fn invite_meeting_agent(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(meeting): Path<Uuid>,
    Json(input): Json<InviteMeetingAgent>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let participant = api.meetings.invite_agent(&actor, meeting, &input).await?;
    let detail = json!({ "agent": input.agent, "mode": participant.agent_mode });
    let event = Event::by(actor.id, "meeting.agent_invite")
        .on(meeting)
        .detail(detail);
    api.identity.audit(event).await;
    Ok((
        StatusCode::CREATED,
        [("cache-control", "no-store")],
        Json(participant),
    ))
}

/// Lets a waiting guest in.
#[utoipa::path(post, path = "/api/meetings/{meeting}/participants/{participant}/admit", params(("meeting" = Uuid, Path), ("participant" = Uuid, Path)), responses((status = 200, body = MeetingParticipant), (status = 404), (status = 409)))]
async fn admit_guest(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((meeting, participant)): Path<(Uuid, Uuid)>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let admitted = api.meetings.admit(&actor, meeting, participant).await?;
    let event = Event::by(actor.id, "meeting.admit")
        .on(meeting)
        .detail(json!({ "participant": participant }));
    api.identity.audit(event).await;
    Ok(([("cache-control", "no-store")], Json(admitted)))
}

/// Turns a waiting guest away or removes an admitted one; their tokens stop working at once.
#[utoipa::path(post, path = "/api/meetings/{meeting}/participants/{participant}/deny", params(("meeting" = Uuid, Path), ("participant" = Uuid, Path)), responses((status = 200, body = MeetingParticipant), (status = 404), (status = 409)))]
async fn deny_guest(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((meeting, participant)): Path<(Uuid, Uuid)>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let denied = api.meetings.deny(&actor, meeting, participant).await?;
    let event = Event::by(actor.id, "meeting.deny")
        .on(meeting)
        .detail(json!({ "participant": participant }));
    api.identity.audit(event).await;
    Ok(([("cache-control", "no-store")], Json(denied)))
}

/// A page of segments, oldest first.
#[utoipa::path(get, path = "/api/meetings/{meeting}/transcript", params(("meeting" = Uuid, Path), After), responses((status = 200, body = Vec<TranscriptSegment>), (status = 404)))]
async fn meeting_transcript(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(meeting): Path<Uuid>,
    Query(query): Query<After>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(
            api.meetings
                .transcript(&actor, meeting, query.after)
                .await?,
        ),
    ))
}

/// No login: the link's token is the credential. The response holds a short-lived join token
/// for this one meeting, which no console route accepts. An unknown, revoked or expired link and
/// an ended meeting all answer 404.
#[utoipa::path(post, path = "/api/meeting-guests/join", request_body = GuestJoin, responses((status = 201, body = GuestAccess), (status = 404), (status = 429)))]
async fn guest_join(
    State(api): State<Api>,
    Json(input): Json<GuestJoin>,
) -> Result<impl IntoResponse, IdentityError> {
    let access = api.meetings.join(&input).await?;
    let detail = json!({ "participant": access.participant, "status": access.status });
    let event = Event::anonymous("meeting.join")
        .on(access.meeting)
        .detail(detail);
    api.identity.audit(event).await;
    Ok((
        StatusCode::CREATED,
        [("cache-control", "no-store")],
        Json(access),
    ))
}

/// Send the join token as a bearer token. Returns a new one for where the guest stands now: a
/// lobby token while they wait, a media token once admitted.
#[utoipa::path(post, path = "/api/meeting-guests/refresh", responses((status = 200, body = GuestAccess), (status = 401)))]
async fn guest_refresh(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let token = super::bearer(&headers).ok_or(IdentityError::Unauthorized)?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.meetings.refresh(token).await?),
    ))
}

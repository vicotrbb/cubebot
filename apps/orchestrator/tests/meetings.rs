use crate::{rooms::agent, support};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use cubebot_orchestrator::{
    identity::{Actor, Identity, IdentityError},
    meetings::{
        CreateMeeting, GUEST_LIMIT, GuestAccess, GuestJoin, InviteMeetingAgent, JoinTokenScope,
        MeetingAgentMode, MeetingParticipantStatus, MeetingSettings, Meetings, NewMeetingLink,
    },
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

struct World {
    pool: PgPool,
    identity: Identity,
    meetings: Meetings,
    admin: Actor,
    member: Actor,
}

async fn world() -> World {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    identity
        .create_account(&admin, "member", "member password", false, Uuid::new_v4())
        .await
        .unwrap();
    let session = identity.login("member", "member password").await.unwrap();
    let member = identity.authenticate(&session.token).await.unwrap();
    World {
        meetings: Meetings::new(pool.clone()),
        pool,
        identity,
        admin,
        member,
    }
}

fn meeting(title: &str) -> CreateMeeting {
    CreateMeeting {
        title: title.into(),
        consent_notice: None,
        agent_mode: None,
    }
}

fn link(lobby: bool) -> NewMeetingLink {
    NewMeetingLink {
        lobby: Some(lobby),
        expires_in_seconds: None,
    }
}

fn join(link: &str, name: &str) -> GuestJoin {
    GuestJoin {
        link: link.into(),
        name: name.into(),
    }
}

/// An open meeting of the administrator's with one link; returns the meeting, link and its token.
async fn open(world: &World, lobby: bool) -> (Uuid, Uuid, String) {
    let meeting = world
        .meetings
        .create(&world.admin, &meeting("Standup"))
        .await
        .unwrap()
        .id;
    let created = world
        .meetings
        .create_link(&world.admin, meeting, &link(lobby))
        .await
        .unwrap();
    (meeting, created.link.id, created.token)
}

async fn expire_link(pool: &PgPool, link: Uuid) {
    // Runtime SQL: only the tests move time.
    sqlx::query("UPDATE meeting_links SET expires_at = clock_timestamp() - interval '1 second' WHERE id = $1")
        .bind(link)
        .execute(pool)
        .await
        .unwrap();
}

fn digest_of(token: &str) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    Sha256::digest(token.as_bytes()).to_vec()
}

async fn expire_join_token(pool: &PgPool, access: &GuestAccess) {
    sqlx::query(
        "UPDATE meeting_join_tokens SET expires_at = clock_timestamp() - interval '1 second' WHERE token_digest = $1",
    )
    .bind(digest_of(&access.token))
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn settings_default_to_the_safe_choices_and_only_administrators_change_them() {
    let world = world().await;
    let settings = world.meetings.settings().await.unwrap();
    assert!(settings.lobby_default && settings.consent_notice);
    assert_eq!(settings.agent_mode, MeetingAgentMode::OnMention);

    let changed = MeetingSettings {
        lobby_default: false,
        consent_notice: false,
        agent_mode: MeetingAgentMode::ListenOnly,
    };
    assert!(matches!(
        world.meetings.save_settings(&world.member, &changed).await,
        Err(IdentityError::Forbidden)
    ));
    let before = world
        .meetings
        .create(&world.admin, &meeting("Before"))
        .await
        .unwrap();
    world
        .meetings
        .save_settings(&world.admin, &changed)
        .await
        .unwrap();
    let saved = world.meetings.settings().await.unwrap();
    assert!(!saved.lobby_default && !saved.consent_notice);
    assert_eq!(saved.agent_mode, MeetingAgentMode::ListenOnly);

    // A meeting keeps what was set when it was made; a new one and a new link follow the setting.
    let after = world
        .meetings
        .create(&world.admin, &meeting("After"))
        .await
        .unwrap();
    assert!(before.consent_notice && !after.consent_notice);
    assert_eq!(before.agent_mode, MeetingAgentMode::OnMention);
    assert_eq!(after.agent_mode, MeetingAgentMode::ListenOnly);
    let plain = NewMeetingLink {
        lobby: None,
        expires_in_seconds: None,
    };
    let created = world
        .meetings
        .create_link(&world.admin, after.id, &plain)
        .await
        .unwrap();
    assert!(!created.link.lobby);
    let own = NewMeetingLink {
        lobby: Some(true),
        ..plain
    };
    let created = world
        .meetings
        .create_link(&world.admin, after.id, &own)
        .await
        .unwrap();
    assert!(created.link.lobby);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_lobby_holds_a_guest_until_the_host_admits_them() {
    let world = world().await;
    let (meeting, _, token) = open(&world, true).await;
    let waiting = world.meetings.join(&join(&token, " Ada ")).await.unwrap();
    assert_eq!(waiting.status, MeetingParticipantStatus::Waiting);
    assert_eq!(waiting.scope, JoinTokenScope::Lobby);
    assert_eq!(waiting.meeting, meeting);
    assert!(waiting.consent_notice);

    // A lobby token cannot join the call, and a waiting guest cannot be recorded as speaking.
    assert!(matches!(
        world
            .meetings
            .verify(&waiting.token, JoinTokenScope::Media)
            .await,
        Err(IdentityError::Forbidden)
    ));
    assert!(
        world
            .meetings
            .verify(&waiting.token, JoinTokenScope::Lobby)
            .await
            .is_ok()
    );
    assert!(matches!(
        world
            .meetings
            .append_segment(meeting, Some(waiting.participant), "Ada", "hello")
            .await,
        Err(IdentityError::NotFound)
    ));
    let still = world.meetings.refresh(&waiting.token).await.unwrap();
    assert_eq!(still.status, MeetingParticipantStatus::Waiting);
    assert_eq!(still.scope, JoinTokenScope::Lobby);

    let admitted = world
        .meetings
        .admit(&world.admin, meeting, waiting.participant)
        .await
        .unwrap();
    assert_eq!(admitted.status, MeetingParticipantStatus::Admitted);
    assert_eq!(admitted.name, "Ada");
    let access = world.meetings.refresh(&waiting.token).await.unwrap();
    assert_eq!(access.scope, JoinTokenScope::Media);
    let claims = world
        .meetings
        .verify(&access.token, JoinTokenScope::Media)
        .await
        .unwrap();
    assert_eq!(
        (claims.meeting, claims.participant),
        (meeting, waiting.participant)
    );
    // Admitting twice is a conflict, not a second admission.
    assert!(matches!(
        world
            .meetings
            .admit(&world.admin, meeting, waiting.participant)
            .await,
        Err(IdentityError::Conflict)
    ));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn without_a_lobby_a_guest_goes_straight_to_media() {
    let world = world().await;
    let (_, _, token) = open(&world, false).await;
    let access = world.meetings.join(&join(&token, "Bo")).await.unwrap();
    assert_eq!(access.status, MeetingParticipantStatus::Admitted);
    assert_eq!(access.scope, JoinTokenScope::Media);
    assert!(access.expires_in_seconds <= 300);
    assert!(matches!(
        world
            .meetings
            .verify(&access.token, JoinTokenScope::Lobby)
            .await,
        Err(IdentityError::Forbidden)
    ));
    for name in ["", "   ", "tab\there", &"x".repeat(65)] {
        assert!(matches!(
            world.meetings.join(&join(&token, name)).await,
            Err(IdentityError::InvalidInput)
        ));
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_token_is_bound_to_its_meeting_and_expires() {
    let world = world().await;
    let (first, _, first_link) = open(&world, false).await;
    let (second, _, second_link) = open(&world, false).await;
    let one = world
        .meetings
        .join(&join(&first_link, "One"))
        .await
        .unwrap();
    let two = world
        .meetings
        .join(&join(&second_link, "Two"))
        .await
        .unwrap();
    let claims = world
        .meetings
        .verify(&one.token, JoinTokenScope::Media)
        .await
        .unwrap();
    assert_eq!(claims.meeting, first);
    assert_ne!(claims.meeting, second);
    let claims = world
        .meetings
        .verify(&two.token, JoinTokenScope::Media)
        .await
        .unwrap();
    assert_eq!(claims.meeting, second);
    // A made-up token, and a link token offered as a join token, are nobody.
    for bogus in ["", "nope", first_link.as_str()] {
        assert!(matches!(
            world.meetings.verify(bogus, JoinTokenScope::Media).await,
            Err(IdentityError::Unauthorized)
        ));
    }

    expire_join_token(&world.pool, &one).await;
    assert!(matches!(
        world
            .meetings
            .verify(&one.token, JoinTokenScope::Media)
            .await,
        Err(IdentityError::Unauthorized)
    ));
    assert!(matches!(
        world.meetings.refresh(&one.token).await,
        Err(IdentityError::Unauthorized)
    ));
    // The other guest is untouched.
    assert!(
        world
            .meetings
            .verify(&two.token, JoinTokenScope::Media)
            .await
            .is_ok()
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_link_expiry_stops_new_guests_but_not_those_already_in() {
    let world = world().await;
    let meeting = world
        .meetings
        .create(&world.admin, &meeting("Timed"))
        .await
        .unwrap()
        .id;
    for seconds in [0, 59, 365 * 24 * 3600 + 1] {
        let input = NewMeetingLink {
            lobby: Some(false),
            expires_in_seconds: Some(seconds),
        };
        assert!(matches!(
            world
                .meetings
                .create_link(&world.admin, meeting, &input)
                .await,
            Err(IdentityError::InvalidInput)
        ));
    }
    let input = NewMeetingLink {
        lobby: Some(false),
        expires_in_seconds: Some(3600),
    };
    let created = world
        .meetings
        .create_link(&world.admin, meeting, &input)
        .await
        .unwrap();
    assert!(created.link.active && created.link.expires_at.is_some());
    let inside = world
        .meetings
        .join(&join(&created.token, "In"))
        .await
        .unwrap();

    expire_link(&world.pool, created.link.id).await;
    assert!(matches!(
        world.meetings.join(&join(&created.token, "Late")).await,
        Err(IdentityError::NotFound)
    ));
    assert!(world.meetings.refresh(&inside.token).await.is_ok());
    let detail = world.meetings.get(&world.admin, meeting).await.unwrap();
    assert!(!detail.links[0].active);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn revoking_a_link_stops_new_guests_and_ends_the_access_of_those_it_let_in() {
    let world = world().await;
    let (meeting, revoked, token) = open(&world, false).await;
    let kept = world
        .meetings
        .create_link(&world.admin, meeting, &link(false))
        .await
        .unwrap();
    let via_revoked = world.meetings.join(&join(&token, "Out")).await.unwrap();
    let via_kept = world
        .meetings
        .join(&join(&kept.token, "Stay"))
        .await
        .unwrap();

    world
        .meetings
        .revoke_link(&world.admin, meeting, revoked)
        .await
        .unwrap();
    // Revoking again is harmless; a link of another meeting is not found.
    world
        .meetings
        .revoke_link(&world.admin, meeting, revoked)
        .await
        .unwrap();
    let (other, other_link, _) = open(&world, false).await;
    assert_ne!(other, meeting);
    assert!(matches!(
        world
            .meetings
            .revoke_link(&world.admin, meeting, other_link)
            .await,
        Err(IdentityError::NotFound)
    ));

    assert!(matches!(
        world.meetings.join(&join(&token, "New")).await,
        Err(IdentityError::NotFound)
    ));
    assert!(matches!(
        world
            .meetings
            .verify(&via_revoked.token, JoinTokenScope::Media)
            .await,
        Err(IdentityError::Unauthorized)
    ));
    assert!(matches!(
        world.meetings.refresh(&via_revoked.token).await,
        Err(IdentityError::Unauthorized)
    ));
    assert!(
        world
            .meetings
            .verify(&via_kept.token, JoinTokenScope::Media)
            .await
            .is_ok()
    );
    let detail = world.meetings.get(&world.admin, meeting).await.unwrap();
    let state: Vec<_> = detail.links.iter().map(|link| link.active).collect();
    assert_eq!(state, [false, true]);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn removing_a_guest_ends_their_access_at_once() {
    let world = world().await;
    let (meeting, _, token) = open(&world, false).await;
    let (elsewhere, _, _) = open(&world, false).await;
    let guest = world.meetings.join(&join(&token, "Eve")).await.unwrap();
    assert!(matches!(
        world
            .meetings
            .deny(&world.admin, elsewhere, guest.participant)
            .await,
        Err(IdentityError::NotFound)
    ));
    let denied = world
        .meetings
        .deny(&world.admin, meeting, guest.participant)
        .await
        .unwrap();
    assert_eq!(denied.status, MeetingParticipantStatus::Denied);
    assert!(matches!(
        world
            .meetings
            .verify(&guest.token, JoinTokenScope::Media)
            .await,
        Err(IdentityError::Unauthorized)
    ));
    assert!(matches!(
        world.meetings.refresh(&guest.token).await,
        Err(IdentityError::Unauthorized)
    ));
    for result in [
        world
            .meetings
            .admit(&world.admin, meeting, guest.participant)
            .await,
        world
            .meetings
            .deny(&world.admin, meeting, guest.participant)
            .await,
    ] {
        assert!(matches!(result, Err(IdentityError::Conflict)));
    }
    assert!(matches!(
        world
            .meetings
            .admit(&world.admin, meeting, Uuid::new_v4())
            .await,
        Err(IdentityError::NotFound)
    ));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_meeting_belongs_to_its_creator_alone() {
    let world = world().await;
    let (meeting, link, _) = open(&world, true).await;
    assert!(world.meetings.list(&world.member).await.unwrap().is_empty());
    let own = world
        .meetings
        .create(&world.member, &meeting_titled("Mine"))
        .await
        .unwrap();
    let listed = world.meetings.list(&world.admin).await.unwrap();
    assert!(listed.iter().all(|listed| listed.id != own.id));
    // Not even an administrator reaches another person's meeting.
    for result in [
        world.meetings.get(&world.admin, own.id).await.map(|_| ()),
        world.meetings.end(&world.admin, own.id).await,
        world
            .meetings
            .create_link(&world.admin, own.id, &link_none())
            .await
            .map(|_| ()),
        world
            .meetings
            .transcript(&world.admin, own.id, 0)
            .await
            .map(|_| ()),
    ] {
        assert!(matches!(result, Err(IdentityError::NotFound)));
    }
    for result in [
        world.meetings.get(&world.member, meeting).await.map(|_| ()),
        world
            .meetings
            .revoke_link(&world.member, meeting, link)
            .await,
        world.meetings.end(&world.member, meeting).await,
    ] {
        assert!(matches!(result, Err(IdentityError::NotFound)));
    }
    assert!(matches!(
        world
            .meetings
            .create(&world.admin, &meeting_titled("  "))
            .await,
        Err(IdentityError::InvalidInput)
    ));
}

fn meeting_titled(title: &str) -> CreateMeeting {
    meeting(title)
}

fn link_none() -> NewMeetingLink {
    NewMeetingLink {
        lobby: None,
        expires_in_seconds: None,
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn an_ended_meeting_admits_no_one_and_records_nothing_more() {
    let world = world().await;
    let (meeting, _, token) = open(&world, false).await;
    let guest = world.meetings.join(&join(&token, "Gus")).await.unwrap();
    world
        .meetings
        .append_segment(meeting, Some(guest.participant), "Gus", "last words")
        .await
        .unwrap();
    world.meetings.end(&world.admin, meeting).await.unwrap();

    assert!(matches!(
        world.meetings.end(&world.admin, meeting).await,
        Err(IdentityError::Conflict)
    ));
    assert!(matches!(
        world.meetings.join(&join(&token, "Late")).await,
        Err(IdentityError::NotFound)
    ));
    assert!(matches!(
        world
            .meetings
            .verify(&guest.token, JoinTokenScope::Media)
            .await,
        Err(IdentityError::Unauthorized)
    ));
    assert!(matches!(
        world
            .meetings
            .append_segment(meeting, None, "Gus", "more")
            .await,
        Err(IdentityError::NotFound)
    ));
    assert!(matches!(
        world
            .meetings
            .create_link(&world.admin, meeting, &link(false))
            .await,
        Err(IdentityError::Conflict)
    ));
    let detail = world.meetings.get(&world.admin, meeting).await.unwrap();
    assert!(detail.meeting.ended_at.is_some());
    let segments = world
        .meetings
        .transcript(&world.admin, meeting, 0)
        .await
        .unwrap();
    assert_eq!(segments.len(), 1);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn the_transcript_keeps_who_said_what_in_order() {
    let world = world().await;
    let ada = agent(&world.pool, &world.admin, "Ada").await;
    let (meeting, _, token) = open(&world, false).await;
    let (other, _, other_token) = open(&world, false).await;
    let agent_seat = world
        .meetings
        .invite_agent(
            &world.admin,
            meeting,
            &InviteMeetingAgent {
                agent: ada,
                mode: None,
            },
        )
        .await
        .unwrap();
    let guest = world.meetings.join(&join(&token, "Gus")).await.unwrap();
    let stranger = world
        .meetings
        .join(&join(&other_token, "Zed"))
        .await
        .unwrap();

    let spoken = [
        (Some(guest.participant), "Gus", "Shall we start?"),
        (Some(agent_seat.id), "Ada", "Yes."),
        (None, "Host", "  Welcome all.  "),
    ];
    for (participant, speaker, text) in spoken {
        world
            .meetings
            .append_segment(meeting, participant, speaker, text)
            .await
            .unwrap();
    }
    // Another meeting's participant cannot speak here, and bad input is refused.
    assert!(matches!(
        world
            .meetings
            .append_segment(meeting, Some(stranger.participant), "Zed", "hi")
            .await,
        Err(IdentityError::NotFound)
    ));
    for (speaker, text) in [("", "x"), ("Gus", "   "), ("Gus", &"x".repeat(4097)[..])] {
        assert!(matches!(
            world
                .meetings
                .append_segment(meeting, None, speaker, text)
                .await,
            Err(IdentityError::InvalidInput)
        ));
    }
    world
        .meetings
        .append_segment(other, None, "Host", "elsewhere")
        .await
        .unwrap();

    let all = world
        .meetings
        .transcript(&world.admin, meeting, 0)
        .await
        .unwrap();
    let said: Vec<_> = all
        .iter()
        .map(|s| (s.speaker.as_str(), s.text.as_str()))
        .collect();
    assert_eq!(
        said,
        [
            ("Gus", "Shall we start?"),
            ("Ada", "Yes."),
            ("Host", "Welcome all.")
        ]
    );
    assert_eq!(all[1].participant, Some(agent_seat.id));
    assert!(all[0].sequence < all[1].sequence && all[1].sequence < all[2].sequence);
    let rest = world
        .meetings
        .transcript(&world.admin, meeting, all[0].sequence)
        .await
        .unwrap();
    assert_eq!(rest.len(), 2);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn an_agent_is_invited_once_and_only_by_someone_who_may_message_it() {
    let world = world().await;
    let ada = agent(&world.pool, &world.admin, "Ada").await;
    let (meeting, _, _) = open(&world, true).await;
    let invite = InviteMeetingAgent {
        agent: ada,
        mode: None,
    };
    let seat = world
        .meetings
        .invite_agent(&world.admin, meeting, &invite)
        .await
        .unwrap();
    assert_eq!(
        (seat.name.as_str(), seat.status),
        ("Ada", MeetingParticipantStatus::Invited)
    );
    assert_eq!(seat.agent_mode, Some(MeetingAgentMode::OnMention));
    assert!(matches!(
        world
            .meetings
            .invite_agent(&world.admin, meeting, &invite)
            .await,
        Err(IdentityError::Conflict)
    ));
    // A member with no access to the agent cannot seat it, in a meeting of their own either.
    let own = world
        .meetings
        .create(&world.member, &meeting_titled("Mine"))
        .await
        .unwrap();
    assert!(matches!(
        world
            .meetings
            .invite_agent(&world.member, own.id, &invite)
            .await,
        Err(IdentityError::NotFound)
    ));
    // Nor can an agent be admitted or denied like a guest.
    assert!(matches!(
        world.meetings.admit(&world.admin, meeting, seat.id).await,
        Err(IdentityError::NotFound)
    ));
    let bob = agent(&world.pool, &world.admin, "Bob").await;
    let quiet = InviteMeetingAgent {
        agent: bob,
        mode: Some(MeetingAgentMode::ListenOnly),
    };
    let seat = world
        .meetings
        .invite_agent(&world.admin, meeting, &quiet)
        .await
        .unwrap();
    assert_eq!(seat.agent_mode, Some(MeetingAgentMode::ListenOnly));
    let detail = world.meetings.get(&world.admin, meeting).await.unwrap();
    assert_eq!(detail.participants.len(), 2);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_link_cannot_flood_a_meeting_with_guests() {
    let world = world().await;
    let (meeting, _, token) = open(&world, true).await;
    let mut first = None;
    for number in 0..GUEST_LIMIT {
        let access = world
            .meetings
            .join(&join(&token, &format!("Guest {number}")))
            .await
            .unwrap();
        first.get_or_insert(access.participant);
    }
    assert!(matches!(
        world.meetings.join(&join(&token, "One too many")).await,
        Err(IdentityError::Busy)
    ));
    world
        .meetings
        .deny(&world.admin, meeting, first.unwrap())
        .await
        .unwrap();
    assert!(
        world
            .meetings
            .join(&join(&token, "Room again"))
            .await
            .is_ok()
    );
}

async fn request(
    router: &axum::Router,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let response = router
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_guest_token_never_reaches_the_console_api() {
    let world = world().await;
    let router = crate::database_router(world.pool.clone());
    let session = world
        .identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = Some(session.token.as_str());

    let (status, created) = request(
        &router,
        "POST",
        "/api/meetings",
        admin,
        json!({"title":"Public"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["id"].as_str().unwrap().to_owned();
    let (status, link) = request(
        &router,
        "POST",
        &format!("/api/meetings/{id}/links"),
        admin,
        json!({"lobby": false}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let link_token = link["token"].as_str().unwrap().to_owned();
    // The token is shown once: reading the meeting never repeats it or its digest.
    let (_, detail) = request(
        &router,
        "GET",
        &format!("/api/meetings/{id}"),
        admin,
        Value::Null,
    )
    .await;
    assert!(!detail.to_string().contains(&link_token));
    assert!(!detail.to_string().contains(&hex(&digest_of(&link_token))));

    // A guest needs no login to join, and a bad link is a plain 404.
    let (status, access) = request(
        &router,
        "POST",
        "/api/meeting-guests/join",
        None,
        json!({"link": link_token, "name": "Gus"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(access["scope"], "media");
    let guest = access["token"].as_str().unwrap();
    let (status, _) = request(
        &router,
        "POST",
        "/api/meeting-guests/join",
        None,
        json!({"link": "not a link", "name": "Gus"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // No console route accepts the guest's token, whatever it is asked to do.
    let calls = [
        ("GET", "/api/meetings".to_owned(), Value::Null),
        ("GET", format!("/api/meetings/{id}"), Value::Null),
        ("GET", format!("/api/meetings/{id}/transcript"), Value::Null),
        ("POST", "/api/meetings".to_owned(), json!({"title":"Mine"})),
        ("POST", format!("/api/meetings/{id}/end"), Value::Null),
        (
            "PUT",
            "/api/meetings/settings".to_owned(),
            json!({"lobby_default":false,"consent_notice":false,"agent_mode":"listen_only"}),
        ),
        ("GET", "/api/meetings/settings".to_owned(), Value::Null),
        ("GET", "/api/identity/me".to_owned(), Value::Null),
        ("GET", "/api/agents".to_owned(), Value::Null),
    ];
    for (method, path, body) in calls {
        let (status, _) = request(&router, method, &path, Some(guest), body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
    }
    // And the guest endpoints accept only a guest's token.
    let (status, _) = request(
        &router,
        "POST",
        "/api/meeting-guests/refresh",
        admin,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = request(
        &router,
        "POST",
        "/api/meeting-guests/refresh",
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = request(
        &router,
        "POST",
        "/api/meeting-guests/refresh",
        Some(&link_token),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, fresh) = request(
        &router,
        "POST",
        "/api/meeting-guests/refresh",
        Some(guest),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fresh["scope"], "media");

    // The host revokes the link; the guest is out.
    let link_id = link["link"]["id"].as_str().unwrap();
    let (status, _) = request(
        &router,
        "DELETE",
        &format!("/api/meetings/{id}/links/{link_id}"),
        admin,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = request(
        &router,
        "POST",
        "/api/meeting-guests/refresh",
        Some(guest),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

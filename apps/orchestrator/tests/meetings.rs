use crate::{
    connections::{Console, Vendor},
    runtime_identity::http_server,
    support,
};
use cubebot_orchestrator::{
    agents::{Agents, CreateAgent},
    calendar::{Calendar, Window},
    connections::{ConfigureProvider, Connections, Credentials, FinishSignIn, SignInStarted},
    identity::{
        Actor, Identity, IdentityError,
        access::{Capability, GrantRequest, Recipient, Scope},
    },
    meetings::{Meetings, NewInvitations, NewMeeting, SaveMeetingSettings},
    speech::{Activity, Audio, Engines, Speech, SpeechError, Stt, Tts, Vad},
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const ORIGIN: &str = "https://meet.example.com";
const GMAIL: &str = "/gmail.googleapis.com/gmail/v1/users/me";

struct World {
    pool: sqlx::PgPool,
    identity: Identity,
    admin: Actor,
    ana: Actor,
    bo: Actor,
    cy: Actor,
    vendor: Vendor,
    connections: Connections,
    meetings: Meetings,
}

async fn world() -> World {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let admin = login(&identity, "admin", "administrator password").await;
    let mut members = Vec::new();
    for name in ["ana", "bob", "cyd"] {
        let password = format!("{name} member password");
        identity
            .create_account(&admin, name, &password, false, Uuid::new_v4())
            .await
            .unwrap();
        members.push(login(&identity, name, &password).await);
    }
    let [ana, bo, cy] = members.try_into().ok().unwrap();
    let vendor = Vendor::default();
    let connections = Connections::new(pool.clone(), Credentials::new([7; 32]))
        .with_vendor_for_loopback_tests(&vendor.serve().await);
    let meetings = Meetings::new(pool.clone(), Some(connections.clone()), Some(ORIGIN.into()));
    World {
        pool,
        identity,
        admin,
        ana,
        bo,
        cy,
        vendor,
        connections,
        meetings,
    }
}

async fn login(identity: &Identity, name: &str, password: &str) -> Actor {
    let session = identity.login(name, password).await.unwrap();
    identity.authenticate(&session.token).await.unwrap()
}

fn new(input: Value) -> NewMeeting {
    serde_json::from_value(input).unwrap()
}

fn invitations(input: Value) -> NewInvitations {
    serde_json::from_value(input).unwrap()
}

async fn agent(w: &World, name: &str) -> Uuid {
    let input = CreateAgent {
        request_id: Uuid::new_v4(),
        name: name.into(),
        instructions: "Be brief".into(),
        avatar: None,
        voice: None,
    };
    Agents::new(w.pool.clone())
        .create(&w.admin, &input)
        .await
        .unwrap()
        .id
}

async fn gmail(w: &World, owner: &Actor) -> Uuid {
    let start = serde_json::from_value(json!({
        "connector": "google", "slug": "mail", "auth": "oauth2",
        "config": {"client_id": "client-1.apps.googleusercontent.com", "products": ["gmail"]},
        "secrets": {"client_secret": "GOCSPX-secret"},
    }))
    .unwrap();
    let started = w
        .connections
        .start_connection_sign_in(owner.id, start, None);
    let SignInStarted::Redirect { sign_in, url, .. } = started.await.unwrap() else {
        panic!("a redirect sign-in");
    };
    let url = reqwest::Url::parse(&url).unwrap();
    let (_, state) = url.query_pairs().find(|(key, _)| key == "state").unwrap();
    w.vendor.answer(
        "/oauth2.googleapis.com/token",
        200,
        json!({"access_token": "ya29.mail", "refresh_token": "1//refresh", "expires_in": 3599}),
    );
    w.vendor.answer(
        "/openidconnect.googleapis.com/v1/userinfo",
        200,
        json!({"email": "host@example.com"}),
    );
    let landed = FinishSignIn {
        url: Some(format!(
            "http://127.0.0.1:1457/callback?state={state}&code=c"
        )),
    };
    let finished = w
        .connections
        .finish_connection_sign_in(owner.id, sign_in, landed)
        .await;
    finished.unwrap().unwrap().id
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn only_participants_read_a_meeting_and_only_its_owner_changes_it() {
    let w = world().await;
    let (meeting, invited) = w
        .meetings
        .create(
            &w.ana,
            &new(json!({"title": " Planning ", "invite": {"accounts": [w.bo.id]}})),
        )
        .await
        .unwrap();
    assert_eq!(
        (meeting.title.as_str(), meeting.state.as_str()),
        ("Planning", "scheduled")
    );
    assert!(meeting.owner && meeting.starts_at.is_none());
    assert_eq!(
        (invited.accounts, invited.agents, invited.emails),
        (1, 0, 0)
    );
    let link = meeting.link.clone().unwrap();
    assert!(link.starts_with(&format!("{ORIGIN}/m/")));

    let read = w.meetings.meeting(&w.bo, meeting.id).await.unwrap();
    assert!(!read.owner);
    assert_eq!(read.link.as_deref(), Some(link.as_str()));
    assert_eq!(read.invitations[0].name, "bob");

    let missing = w.meetings.meeting(&w.cy, Uuid::new_v4()).await;
    assert!(matches!(missing, Err(IdentityError::NotFound)));
    let hidden = w.meetings.meeting(&w.cy, meeting.id).await;
    assert!(matches!(hidden, Err(IdentityError::NotFound)));
    let change = serde_json::from_value(json!({"title": "Mine"})).unwrap();
    let outsider = w.meetings.update(&w.cy, meeting.id, &change).await;
    assert!(matches!(outsider, Err(IdentityError::NotFound)));

    let invitee = w.meetings.update(&w.bo, meeting.id, &change).await;
    assert!(matches!(invitee, Err(IdentityError::Forbidden)));
    let relink = w.meetings.set_link(&w.bo, meeting.id, true).await;
    assert!(matches!(relink, Err(IdentityError::Forbidden)));
    let cancel = w.meetings.cancel(&w.bo, meeting.id).await;
    assert!(matches!(cancel, Err(IdentityError::Forbidden)));
    let guest = invitations(json!({"invite": {"emails": ["Guest@Example.com"]}}));
    let added = w.meetings.invite(&w.bo, meeting.id, &guest).await.unwrap();
    assert_eq!(added.emails, 1);
    let read = w.meetings.meeting(&w.ana, meeting.id).await.unwrap();
    let email = read.invitations.iter().find(|i| i.kind == "email").unwrap();
    assert_eq!(email.email.as_deref(), Some("guest@example.com"));
    let removed = w.meetings.uninvite(&w.bo, meeting.id, email.id).await;
    assert!(matches!(removed, Err(IdentityError::Forbidden)));

    let (off, _) = w
        .meetings
        .set_link(&w.ana, meeting.id, false)
        .await
        .unwrap();
    assert!(off.link.is_none());
    let (on, _) = w.meetings.set_link(&w.ana, meeting.id, true).await.unwrap();
    assert_ne!(on.link.as_deref(), Some(link.as_str()));
    let schedule = serde_json::from_value(json!({
        "starts_at": "2030-01-08T10:00:00Z", "ends_at": "2030-01-08T09:00:00Z",
    }))
    .unwrap();
    let backwards = w.meetings.update(&w.ana, meeting.id, &schedule).await;
    assert!(matches!(
        backwards,
        Err(IdentityError::Invalid(_) | IdentityError::InvalidInput)
    ));
    assert_eq!(
        w.meetings
            .update(&w.ana, meeting.id, &change)
            .await
            .unwrap(),
        "Mine"
    );
    w.meetings
        .uninvite(&w.ana, meeting.id, email.id)
        .await
        .unwrap();

    let listed = w.meetings.list(&w.bo).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].people, listed[0].owner_name.as_str()),
        (1, "ana")
    );
    assert!(w.meetings.list(&w.cy).await.unwrap().is_empty());
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_live_meeting_cannot_be_cancelled() {
    let w = world().await;
    let (meeting, _) = w
        .meetings
        .create(&w.ana, &new(json!({"title": "Standup"})))
        .await
        .unwrap();
    sqlx::query("UPDATE meetings SET started_at = now() WHERE id = $1")
        .bind(meeting.id)
        .execute(&w.pool)
        .await
        .unwrap();
    let live = w.meetings.meeting(&w.ana, meeting.id).await.unwrap();
    assert_eq!(live.state, "live");
    let refused = w.meetings.cancel(&w.ana, meeting.id).await;
    assert!(matches!(refused, Err(IdentityError::Conflict)));
    sqlx::query("UPDATE meetings SET ended_at = now() WHERE id = $1")
        .bind(meeting.id)
        .execute(&w.pool)
        .await
        .unwrap();
    assert_eq!(
        w.meetings.cancel(&w.ana, meeting.id).await.unwrap(),
        "Standup"
    );
    let gone = w.meetings.meeting(&w.ana, meeting.id).await;
    assert!(matches!(gone, Err(IdentityError::NotFound)));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn inviting_an_agent_needs_conversation_send_and_agents_allowed_in_meetings() {
    let w = world().await;
    let scout = agent(&w, "Scout").await;
    let invite = json!({"title": "Review", "invite": {"agents": [scout]}});
    let refused = w.meetings.create(&w.ana, &new(invite.clone())).await;
    assert!(matches!(refused, Err(IdentityError::NotFound)));
    assert!(w.meetings.list(&w.ana).await.unwrap().is_empty());

    for capability in [Capability::ConversationRead, Capability::ConversationSend] {
        let grant = GrantRequest {
            request_id: Uuid::new_v4(),
            recipient: Recipient::User { id: w.ana.id },
            capability,
            scope: Scope::Conversation { id: scout },
        };
        w.identity.grant(&w.admin, &grant).await.unwrap();
    }
    let (meeting, invited) = w
        .meetings
        .create(&w.ana, &new(invite.clone()))
        .await
        .unwrap();
    assert_eq!(invited.agents, 1);
    let scout_invited = &meeting.invitations[0];
    assert_eq!(
        (scout_invited.kind.as_str(), scout_invited.name.as_str()),
        ("agent", "Scout")
    );
    assert!(scout_invited.avatar.is_some());

    assert!(!scout_invited.listening);

    let quiet = json!({"title": "Quiet", "invite": {"agents": [scout], "listening": [scout]}});
    let (quiet, _) = w.meetings.create(&w.ana, &new(quiet)).await.unwrap();
    let seat = quiet.invitations[0].id;
    assert!(quiet.invitations[0].listening);
    let forbidden = w.meetings.set_listening(&w.bo, quiet.id, seat, false).await;
    assert!(matches!(forbidden, Err(IdentityError::NotFound)));
    w.meetings
        .set_listening(&w.ana, quiet.id, seat, false)
        .await
        .unwrap();
    let read = w.meetings.meeting(&w.ana, quiet.id).await.unwrap();
    assert!(!read.invitations[0].listening);
    let unknown = w
        .meetings
        .set_listening(&w.ana, quiet.id, Uuid::new_v4(), true);
    assert!(matches!(unknown.await, Err(IdentityError::NotFound)));

    let settings = w.meetings.settings().await.unwrap();
    let off = SaveMeetingSettings {
        agents_join: false,
        agent_screens: settings.agent_screens,
        guests: settings.guests,
        max_participants: settings.max_participants,
        agent_mode: settings.agent_mode,
    };
    w.meetings.save_settings(&w.admin, &off).await.unwrap();
    let refused = w.meetings.create(&w.ana, &new(invite)).await;
    assert!(matches!(refused, Err(IdentityError::Forbidden)));

    let retired = agent(&w, "Gone").await;
    sqlx::query("UPDATE agents SET desired_state = 'retired' WHERE id = $1")
        .bind(retired)
        .execute(&w.pool)
        .await
        .unwrap();
    let on = SaveMeetingSettings {
        agents_join: true,
        ..off
    };
    w.meetings.save_settings(&w.admin, &on).await.unwrap();
    let invite = invitations(json!({"invite": {"agents": [retired]}}));
    let refused = w.meetings.invite(&w.admin, meeting.id, &invite).await;
    assert!(
        matches!(refused, Err(IdentityError::NotFound)),
        "not a participant"
    );
    let refused = w.meetings.invite(&w.ana, meeting.id, &invite).await;
    assert!(matches!(refused, Err(IdentityError::NotFound)), "no grant");
    let (theirs, _) = w
        .meetings
        .create(&w.admin, &new(json!({"title": "Admins"})))
        .await
        .unwrap();
    let refused = w.meetings.invite(&w.admin, theirs.id, &invite).await;
    assert!(matches!(refused, Err(IdentityError::Conflict)), "retired");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn meeting_settings_are_read_by_members_and_saved_by_administrators() {
    let w = world().await;
    let settings = w.meetings.settings().await.unwrap();
    assert_eq!(
        serde_json::to_value(&settings).unwrap(),
        json!({"agents_join": true, "agent_screens": true, "guests": true,
            "max_participants": 12, "agent_mode": "relevant", "public_origin": ORIGIN})
    );
    let input: SaveMeetingSettings = serde_json::from_value(json!({
        "agents_join": true, "agent_screens": false, "guests": false,
        "max_participants": 6, "agent_mode": "named",
    }))
    .unwrap();
    let refused = w.meetings.save_settings(&w.ana, &input).await;
    assert!(matches!(refused, Err(IdentityError::Forbidden)));
    w.meetings.save_settings(&w.admin, &input).await.unwrap();
    let saved = w.meetings.settings().await.unwrap();
    assert_eq!((saved.guests, saved.max_participants), (false, 6));
    let too_many = SaveMeetingSettings {
        max_participants: 25,
        ..input
    };
    let refused = w.meetings.save_settings(&w.admin, &too_many).await;
    assert!(matches!(refused, Err(IdentityError::InvalidInput)));

    let (meeting, _) = w
        .meetings
        .create(&w.ana, &new(json!({"title": "Defaults"})))
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(meeting.agent_mode).unwrap(), "named");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn invitations_are_emailed_through_the_inviters_own_connection() {
    let w = world().await;
    let mail = gmail(&w, &w.ana).await;
    let offered = w.meetings.email_connections(&w.ana).await.unwrap();
    assert_eq!(offered.len(), 1);
    assert_eq!(
        (offered[0].id, offered[0].connector.as_str()),
        (mail, "google")
    );
    assert!(
        w.meetings
            .email_connections(&w.bo)
            .await
            .unwrap()
            .is_empty()
    );

    let theirs = json!({"title": "Sync", "email_connection": mail,
        "invite": {"emails": ["guest@example.com"]}});
    let refused = w.meetings.create(&w.bo, &new(theirs)).await;
    assert!(matches!(refused, Err(IdentityError::NotFound)));
    assert!(w.vendor.seen(GMAIL).is_empty());

    for _ in 0..2 {
        w.vendor.answer(
            &format!("{GMAIL}/profile"),
            200,
            json!({"emailAddress": "host@example.com"}),
        );
    }
    w.vendor.answer(
        &format!("{GMAIL}/messages/send"),
        200,
        json!({"id": "m1", "threadId": "t1", "labelIds": ["SENT"]}),
    );
    let input = json!({"title": "Sync", "starts_at": "2030-01-08T10:00:00Z",
        "ends_at": "2030-01-08T11:00:00Z", "email_connection": mail,
        "invite": {"emails": ["one@example.com", "two@example.com"]}});
    let (meeting, invited) = w.meetings.create(&w.ana, &new(input)).await.unwrap();
    assert_eq!(invited.emails, 2);
    assert_eq!(w.vendor.seen(&format!("{GMAIL}/messages/send")).len(), 2);
    let sent = meeting
        .invitations
        .iter()
        .filter(|i| i.emailed_at.is_some());
    let failed = meeting
        .invitations
        .iter()
        .filter(|i| i.email_error.is_some());
    assert_eq!((sent.count(), failed.count()), (1, 1));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn the_calendar_shows_a_meeting_to_its_owner_and_invitees_only() {
    let w = world().await;
    let input = json!({"title": "Planning", "starts_at": "2030-01-08T10:00:00Z",
        "ends_at": "2030-01-08T11:00:00Z", "invite": {"accounts": [w.bo.id]}});
    let (meeting, _) = w.meetings.create(&w.ana, &new(input)).await.unwrap();
    let calendar = Calendar::new(w.pool.clone(), None);
    let window = Window {
        from: "2030-01-07T00:00:00Z".into(),
        to: "2030-01-14T00:00:00Z".into(),
        time_zone: None,
    };
    for viewer in [&w.ana, &w.bo] {
        let found = calendar.range(viewer, &window).await.unwrap();
        assert_eq!(found.sources.len(), 1);
        assert_eq!(found.sources[0].kind, "meetings");
        let item = &found.items[0];
        assert_eq!(
            (item.kind, item.start.as_str(), item.end.as_deref()),
            (
                "meeting",
                "2030-01-08T10:00:00Z",
                Some("2030-01-08T11:00:00Z")
            )
        );
        let detail = item.meeting.as_ref().unwrap();
        assert_eq!(detail.join, format!("/meetings/{}", meeting.id));
        assert_eq!(
            (detail.owner.as_str(), detail.state.as_str()),
            ("ana", "scheduled")
        );
    }
    let found = calendar.range(&w.cy, &window).await.unwrap();
    assert!(found.sources.is_empty() && found.items.is_empty());
    let later = Window {
        from: "2030-02-01T00:00:00Z".into(),
        to: "2030-02-08T00:00:00Z".into(),
        ..window
    };
    assert!(
        calendar
            .range(&w.ana, &later)
            .await
            .unwrap()
            .items
            .is_empty()
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn the_meeting_routes_answer_beside_a_meeting_id() {
    let w = world().await;
    let router = cubebot_orchestrator::api::configured_router_with_credentials(
        w.pool.clone(),
        None,
        None,
        Some(Credentials::new([7; 32])),
    )
    .await
    .unwrap();
    let (origin, _task) = http_server(router).await;
    let session = w
        .identity
        .login("ana", "ana member password")
        .await
        .unwrap();
    let api = Console {
        http: openidconnect::reqwest::Client::new(),
        origin,
        token: session.token,
    };
    let (status, settings) = api.json("GET", "/api/meetings/settings", None).await;
    assert_eq!((status, &settings["public_origin"]), (200, &Value::Null));
    let (status, body) = api
        .json("GET", "/api/meetings/email-connections", None)
        .await;
    assert_eq!((status, body), (200, json!([])));
    let (status, made) = api
        .json("POST", "/api/meetings", Some(json!({"title": "Now"})))
        .await;
    assert_eq!(status, 200);
    let link = made["link"].as_str().unwrap();
    assert!(link.starts_with("/m/") && link.len() == 46);
    let (status, read) = api
        .json(
            "GET",
            &format!("/api/meetings/{}", made["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!((status, &read["title"]), (200, &json!("Now")));
    let (status, _) = api
        .json("PUT", "/api/meetings/settings", Some(settings))
        .await;
    assert_eq!(status, 422, "the read-only origin is not a setting");
}

const CONSOLE: &str = "https://console.example.com";

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Served {
    origin: String,
    _task: tokio::task::JoinHandle<()>,
}

async fn served(w: &World) -> Served {
    let router = cubebot_orchestrator::api::configured_router_with_credentials(
        w.pool.clone(),
        Some(CONSOLE.into()),
        None,
        Some(Credentials::new([7; 32])),
    )
    .await
    .unwrap();
    let (origin, task) = http_server(router).await;
    Served {
        origin,
        _task: task,
    }
}

async fn token_of(w: &World, name: &str) -> String {
    let password = format!("{name} member password");
    w.identity.login(name, &password).await.unwrap().token
}

async fn connect(
    served: &Served,
    path: &str,
    bearer: Option<&str>,
) -> Result<Socket, tokio_tungstenite::tungstenite::Error> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let url = format!("{}{path}", served.origin.replacen("http", "ws", 1));
    let mut request = url.into_client_request().unwrap();
    let headers = request.headers_mut();
    headers.insert("origin", CONSOLE.parse().unwrap());
    if let Some(bearer) = bearer {
        headers.insert("authorization", format!("Bearer {bearer}").parse().unwrap());
    }
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(socket, _)| socket)
}

async fn say(socket: &mut Socket, value: Value) {
    use futures_util::SinkExt;
    let text = tokio_tungstenite::tungstenite::Message::Text(value.to_string().into());
    socket.send(text).await.unwrap();
}

enum Heard {
    Text(Value),
    Binary(Vec<u8>),
    Closed(String),
}

async fn hear(socket: &mut Socket) -> Heard {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    loop {
        let next = tokio::time::timeout(std::time::Duration::from_secs(30), socket.next())
            .await
            .expect("a frame from the room");
        match next {
            Some(Ok(Message::Text(text))) => {
                return Heard::Text(serde_json::from_str(&text).unwrap());
            }
            Some(Ok(Message::Binary(bytes))) => return Heard::Binary(bytes.to_vec()),
            Some(Ok(Message::Close(frame))) => {
                return Heard::Closed(
                    frame
                        .map(|frame| frame.reason.to_string())
                        .unwrap_or_default(),
                );
            }
            Some(Ok(_)) => {}
            _ => return Heard::Closed(String::new()),
        }
    }
}

async fn until(socket: &mut Socket, kind: &str) -> Value {
    loop {
        match hear(socket).await {
            Heard::Text(value) if value["kind"] == kind => return value,
            Heard::Closed(reason) => panic!("closed waiting for {kind}: {reason}"),
            _ => {}
        }
    }
}

async fn closed(socket: &mut Socket) -> String {
    loop {
        if let Heard::Closed(reason) = hear(socket).await {
            return reason;
        }
    }
}

async fn member(served: &Served, token: &str, meeting: Uuid) -> (Socket, u16) {
    let path = format!("/api/meetings/{meeting}/live");
    let mut socket = connect(served, &path, Some(token)).await.unwrap();
    say(&mut socket, json!({"kind": "hello"})).await;
    let welcome = until(&mut socket, "welcome").await;
    (socket, welcome["you"].as_u64().unwrap() as u16)
}

async fn guest(served: &Served, link: &str, name: &str) -> Socket {
    let token = link.rsplit('/').next().unwrap();
    let path = format!("/api/public/meetings/{token}/live");
    let mut socket = connect(served, &path, None).await.unwrap();
    say(&mut socket, json!({"kind": "hello", "name": name})).await;
    socket
}

async fn eventually(w: &World, id: Uuid, check: &str) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let held: bool = sqlx::query_scalar(check)
            .bind(id)
            .fetch_one(&w.pool)
            .await
            .unwrap();
        if held {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "never held: {check}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn meeting_with(w: &World, input: Value) -> (Uuid, String) {
    let (meeting, _) = w.meetings.create(&w.ana, &new(input)).await.unwrap();
    (meeting.id, meeting.link.unwrap())
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn two_members_hear_each_other_by_slot() {
    let w = world().await;
    let served = served(&w).await;
    let (id, _) = meeting_with(
        &w,
        json!({"title": "Sync", "invite": {"accounts": [w.bo.id]}}),
    )
    .await;
    let (mut ana, ana_slot) = member(&served, &token_of(&w, "ana").await, id).await;
    let (mut bob, bob_slot) = member(&served, &token_of(&w, "bob").await, id).await;
    assert_ne!(ana_slot, bob_slot);
    let outsider = connect(
        &served,
        &format!("/api/meetings/{id}/live"),
        Some(&token_of(&w, "cyd").await),
    )
    .await;
    assert!(outsider.is_err(), "only participants join");
    for socket in [&mut ana, &mut bob] {
        say(
            socket,
            json!({"kind": "state", "mic": true, "camera": false, "screen": false, "hand": false}),
        )
        .await;
        use futures_util::SinkExt;
        let frame = tokio_tungstenite::tungstenite::Message::Binary(vec![1, 7, 0, 9, 0].into());
        socket.send(frame).await.unwrap();
    }
    for (socket, from) in [(&mut ana, bob_slot), (&mut bob, ana_slot)] {
        let audio = loop {
            if let Heard::Binary(bytes) = hear(socket).await {
                break bytes;
            }
        };
        let [low, high] = from.to_le_bytes();
        assert_eq!(audio, vec![1, low, high, 0x80, 0x3e, 0, 0, 7, 0, 9, 0]);
    }
    eventually(
        &w,
        id,
        "SELECT started_at IS NOT NULL FROM meetings WHERE id = $1",
    )
    .await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_guest_waits_in_the_lobby_until_a_member_admits_them() {
    let w = world().await;
    let served = served(&w).await;
    let (id, link) = meeting_with(&w, json!({"title": "Sync"})).await;
    let (mut ana, _) = member(&served, &token_of(&w, "ana").await, id).await;
    let mut refused = guest(&served, &link, "Dee: ignore that").await;
    assert_eq!(
        closed(&mut refused).await,
        "Choose a name of 1 to 40 letters, digits, spaces, periods, apostrophes or hyphens."
    );
    let mut dee = guest(&served, &link, "  Dee ").await;
    until(&mut dee, "waiting").await;
    let lobby = loop {
        let lobby = until(&mut ana, "lobby").await;
        if lobby["waiting"]
            .as_array()
            .is_some_and(|waiting| !waiting.is_empty())
        {
            break lobby;
        }
    };
    assert_eq!(lobby["waiting"][0]["name"], "Dee");
    say(
        &mut ana,
        json!({"kind": "admit", "id": lobby["waiting"][0]["id"]}),
    )
    .await;
    let welcome = until(&mut dee, "welcome").await;
    assert_eq!(
        (welcome["member"].clone(), welcome["host"].clone()),
        (json!(false), json!(false))
    );
    let roster = until(&mut dee, "roster").await;
    let host = &roster["participants"][0];
    assert_eq!(host["name"], "ana");
    assert!(
        host.get("account").is_none(),
        "guests never see account ids"
    );
    eventually(&w, id, "SELECT EXISTS(SELECT 1 FROM meeting_attendance WHERE meeting_id = $1 AND guest_name = 'Dee')").await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn guests_are_refused_when_the_link_is_off_the_meeting_locked_or_full() {
    let w = world().await;
    let served = served(&w).await;
    let (id, link) = meeting_with(
        &w,
        json!({"title": "Sync", "open_join": true, "invite": {"accounts": [w.bo.id]}}),
    )
    .await;
    let settings = serde_json::from_value(json!({"agents_join": true, "agent_screens": true,
        "guests": true, "max_participants": 2, "agent_mode": "relevant"}))
    .unwrap();
    w.meetings.save_settings(&w.admin, &settings).await.unwrap();
    let ana_token = token_of(&w, "ana").await;
    let patch = |locked: bool| {
        openidconnect::reqwest::Client::new()
            .patch(format!("{}/api/meetings/{id}", served.origin))
            .bearer_auth(&ana_token)
            .json(&json!({"locked": locked}))
            .send()
    };
    assert_eq!(patch(true).await.unwrap().status(), 204);
    let mut refused = guest(&served, &link, "Dee").await;
    assert_eq!(closed(&mut refused).await, "The host locked the meeting.");
    assert_eq!(patch(false).await.unwrap().status(), 204);
    let (_ana, _) = member(&served, &ana_token, id).await;
    let (_bob, _) = member(&served, &token_of(&w, "bob").await, id).await;
    let mut full = guest(&served, &link, "Dee").await;
    assert_eq!(closed(&mut full).await, "The meeting is full.");

    w.meetings.set_link(&w.ana, id, false).await.unwrap();
    let token = link.rsplit('/').next().unwrap();
    let off = connect(&served, &format!("/api/public/meetings/{token}/live"), None).await;
    assert!(off.is_err(), "a link that is off is unknown");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_distinct_meetings_host_answers_only_the_public_meeting_routes() {
    let w = world().await;
    let (_, link) = meeting_with(&w, json!({"title": "Sync"})).await;
    let token = link.rsplit('/').next().unwrap();
    let router = cubebot_orchestrator::api::configured_router_with_credentials(
        w.pool.clone(),
        Some(CONSOLE.into()),
        None,
        Some(Credentials::new([7; 32])),
    )
    .await
    .unwrap();
    let router = cubebot_orchestrator::api::public_host(router, Some(CONSOLE), Some(ORIGIN));
    let (origin, _task) = http_server(router).await;
    let session = token_of(&w, "ana").await;
    let http = openidconnect::reqwest::Client::new();
    let get = |host: &str, path: &str| {
        http.get(format!("{origin}{path}"))
            .header("host", host)
            .bearer_auth(&session)
            .send()
    };
    let unknown = get(
        "console.example.com",
        &format!("/api/public/meetings/{}", "x".repeat(43)),
    )
    .await
    .unwrap();
    assert_eq!(unknown.status(), 404);
    let unknown = unknown.text().await.unwrap();
    for host in ["meet.example.com", "MEET.example.com:443"] {
        let me = get(host, "/api/identity/me").await.unwrap();
        assert_eq!(me.status(), 404, "{host}");
        assert_eq!(me.text().await.unwrap(), unknown);
        let found = get(host, &format!("/api/public/meetings/{token}"))
            .await
            .unwrap();
        assert_eq!(found.status(), 200, "{host}");
    }
    let me = get("console.example.com", "/api/identity/me")
        .await
        .unwrap();
    assert_eq!(me.status(), 200);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn the_public_lookup_answers_five_fields_or_the_same_404() {
    let w = world().await;
    let served = served(&w).await;
    let (_, link) = meeting_with(&w, json!({"title": "Sync"})).await;
    let token = link.rsplit('/').next().unwrap();
    let http = openidconnect::reqwest::Client::new();
    let get = |path: String| http.get(format!("{}{path}", served.origin)).send();
    let found = get(format!("/api/public/meetings/{token}")).await.unwrap();
    assert_eq!(found.status(), 200);
    assert_eq!(
        found.json::<Value>().await.unwrap(),
        json!({"title": "Sync", "host": "ana", "state": "scheduled", "locked": false, "lobby": true})
    );
    let unknown = get(format!("/api/public/meetings/{}", "x".repeat(43)))
        .await
        .unwrap();
    assert_eq!(unknown.status(), 404);
    let unknown_body = unknown.text().await.unwrap();
    let cyd = token_of(&w, "cyd").await;
    let join = |token: String| {
        http.post(format!("{}/api/meetings/link/{token}/join", served.origin))
            .bearer_auth(&cyd)
            .send()
    };
    let joined: Value = join(token.to_owned()).await.unwrap().json().await.unwrap();
    let id: Uuid = serde_json::from_value(joined["id"].clone()).unwrap();
    assert_eq!(w.meetings.meeting(&w.cy, id).await.unwrap().title, "Sync");
    sqlx::query("UPDATE meetings SET started_at = now(), ended_at = now() WHERE id = $1")
        .bind(id)
        .execute(&w.pool)
        .await
        .unwrap();
    assert_eq!(join(token.to_owned()).await.unwrap().status(), 409);
    let settings = serde_json::from_value(json!({"agents_join": true, "agent_screens": true,
        "guests": false, "max_participants": 12, "agent_mode": "relevant"}))
    .unwrap();
    w.meetings.save_settings(&w.admin, &settings).await.unwrap();
    let disabled = get(format!("/api/public/meetings/{token}")).await.unwrap();
    assert_eq!(disabled.status(), 404);
    assert_eq!(disabled.text().await.unwrap(), unknown_body);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn the_host_ends_the_meeting_for_everyone() {
    let w = world().await;
    let served = served(&w).await;
    let (id, _) = meeting_with(
        &w,
        json!({"title": "Sync", "invite": {"accounts": [w.bo.id]}}),
    )
    .await;
    let (mut ana, _) = member(&served, &token_of(&w, "ana").await, id).await;
    let (mut bob, _) = member(&served, &token_of(&w, "bob").await, id).await;
    say(&mut bob, json!({"kind": "end"})).await;
    say(&mut ana, json!({"kind": "end"})).await;
    until(&mut bob, "ended").await;
    until(&mut ana, "ended").await;
    assert_eq!(closed(&mut bob).await, "The meeting has ended.");
    eventually(
        &w,
        id,
        "SELECT ended_at IS NOT NULL FROM meetings WHERE id = $1",
    )
    .await;
    let open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM meeting_attendance WHERE meeting_id = $1 AND left_at IS NULL",
    )
    .bind(id)
    .fetch_one(&w.pool)
    .await
    .unwrap();
    assert_eq!(open, 0);

    let path = format!("/api/meetings/{id}/live");
    let mut again = connect(&served, &path, Some(&token_of(&w, "ana").await))
        .await
        .unwrap();
    say(&mut again, json!({"kind": "hello"})).await;
    let Heard::Text(ended) = hear(&mut again).await else {
        panic!("the ended meeting answers ended");
    };
    assert_eq!(ended["kind"], "ended");
    assert_eq!(closed(&mut again).await, "The meeting has ended.");
    let reopened: bool = sqlx::query_scalar("SELECT ended_at IS NULL FROM meetings WHERE id = $1")
        .bind(id)
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert!(!reopened, "an ended meeting stays ended");
}

struct Loud(bool);
impl Vad for Loud {
    fn accept(&mut self, window: &[f32]) -> Activity {
        let speaking = window[0] > 0.0;
        let ended = self.0 && !speaking;
        self.0 = speaking;
        Activity { speaking, ended }
    }
}

struct Script(Arc<Mutex<VecDeque<String>>>);
impl Stt for Script {
    fn accept(&mut self, _: &[f32]) -> Result<Option<String>, SpeechError> {
        Ok(Some("Cara".to_owned()))
    }
    fn finish(&mut self) -> Result<String, SpeechError> {
        Ok(self.0.lock().unwrap().pop_front().unwrap_or_default())
    }
}

struct Says;
impl Tts for Says {
    fn speak(&self, text: &str, sink: &mut dyn FnMut(Audio) -> bool) -> Result<(), SpeechError> {
        sink(Audio {
            rate: 22050,
            pcm: text.as_bytes().to_vec(),
        });
        Ok(())
    }
}

async fn summarizer(axum::Json(body): axum::Json<Value>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let asked = body.to_string();
    let text = if asked.contains("summary of a meeting") && asked.contains("Cara: It is noon.") {
        "Decisions: lunch at noon."
    } else {
        "UNSCRIPTED"
    };
    let output = json!([{"type": "message", "content": [{"type": "output_text", "text": text}]}]);
    let completed = json!({"type": "response.completed", "response": {"status": "completed",
        "output": output, "usage": {"input_tokens": 10, "output_tokens": 5}}});
    (
        [("content-type", "text/event-stream")],
        format!("data: {completed}\n\n"),
    )
        .into_response()
}

struct Voiced {
    served: Served,
    meetings: Meetings,
    cara: Uuid,
    meeting: Uuid,
    link: String,
    admin: String,
}

async fn voiced(w: &World, said: &[&str]) -> Voiced {
    let script = Arc::new(Mutex::new(
        said.iter().map(|line| (*line).to_owned()).collect(),
    ));
    let speech = Speech::stand_in(Arc::new(move || Engines {
        vad: Box::new(Loud(false)),
        stt: Box::new(Script(script.clone())),
        tts: Box::new(Says),
    }));
    let model = axum::Router::new().route("/responses", axum::routing::post(summarizer));
    let (model, _) = http_server(model).await;
    let connections = Connections::new(w.pool.clone(), Credentials::new([7; 32]))
        .with_vendor_for_loopback_tests(&format!("{model}/"));
    connections
        .configure(
            &w.admin,
            &ConfigureProvider {
                request_id: Uuid::new_v4(),
                expected_revision: 0,
                provider: cubebot_protocol::runtime::ProviderKind::Openai,
                model: "controlled-model".into(),
                context_window: Some(128_000),
                api_key: Some("controlled-key".into()),
                access: None,
                reasoning_effort: None,
            },
        )
        .await
        .unwrap();
    let meetings =
        Meetings::new(w.pool.clone(), Some(connections), Some(CONSOLE.into())).with_speech(speech);
    let cara = agent(w, "Cara").await;
    let input = new(
        json!({"title": "Sync", "agent_mode": "named", "open_join": true,
        "invite": {"agents": [cara]}}),
    );
    let (meeting, _) = meetings.create(&w.admin, &input).await.unwrap();
    let router =
        cubebot_orchestrator::api::meetings_router(w.pool.clone(), CONSOLE, meetings.clone());
    let (origin, task) = http_server(router).await;
    let admin = w
        .identity
        .login("admin", "administrator password")
        .await
        .unwrap()
        .token;
    Voiced {
        served: Served {
            origin,
            _task: task,
        },
        meetings,
        cara,
        meeting: meeting.id,
        link: meeting.link.unwrap(),
        admin,
    }
}

async fn agent_slot(socket: &mut Socket) -> u16 {
    loop {
        let roster = until(socket, "roster").await;
        let agents = roster["participants"].as_array().unwrap();
        if let Some(agent) = agents.iter().find(|seat| seat["kind"] == "agent") {
            assert_eq!(agent["name"], "Cara");
            return agent["slot"].as_u64().unwrap() as u16;
        }
    }
}

async fn speaks(socket: &mut Socket) {
    use futures_util::SinkExt;
    say(
        socket,
        json!({"kind": "state", "mic": true, "camera": false, "screen": false, "hand": false}),
    )
    .await;
    for sample in [16384_i16, 0] {
        let mut frame = vec![1_u8];
        frame.extend(std::iter::repeat_n(sample.to_le_bytes(), 512).flatten());
        let frame = tokio_tungstenite::tungstenite::Message::Binary(frame.into());
        socket.send(frame).await.unwrap();
    }
}

async fn meeting_turn(w: &World, meeting: Uuid) -> (Uuid, Uuid, String, bool) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let turn = sqlx::query!(
            "SELECT id, actor_id, message, talk_only FROM chat_turns WHERE meeting_id=$1 AND response IS NULL",
            meeting,
        )
        .fetch_optional(&w.pool)
        .await
        .unwrap();
        if let Some(turn) = turn {
            return (turn.id, turn.actor_id, turn.message, turn.talk_only);
        }
        assert!(tokio::time::Instant::now() < deadline, "no meeting turn");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn answers(w: &World, turn: Uuid, text: &str) {
    sqlx::query!(
        "INSERT INTO turn_events (turn_id, sequence, payload) VALUES ($1, 1, $2)",
        turn,
        json!({"kind": "text_delta", "text": text}),
    )
    .execute(&w.pool)
    .await
    .unwrap();
    sqlx::query!(
        "UPDATE chat_turns SET status='complete', response=$2, completed_at=clock_timestamp() WHERE id=$1",
        turn,
        text,
    )
    .execute(&w.pool)
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_named_agent_hears_a_member_and_answers_aloud_and_the_end_is_summarized() {
    let w = world().await;
    let room = voiced(&w, &["Cara, what time is it?"]).await;
    let (mut admin, _) = member(&room.served, &room.admin, room.meeting).await;
    let cara = agent_slot(&mut admin).await;
    speaks(&mut admin).await;
    let caption = loop {
        let caption = until(&mut admin, "caption").await;
        if caption["final"] == true {
            break caption;
        }
    };
    assert_eq!(caption["text"], "Cara, what time is it?");

    let (turn, actor, message, talk_only) = meeting_turn(&w, room.meeting).await;
    assert_eq!((actor, talk_only), (w.admin.id, false));
    assert_eq!(message, "admin: Cara, what time is it?");
    loop {
        if until(&mut admin, "agent").await["state"] == "thinking" {
            break;
        }
    }
    answers(&w, turn, "It is noon.").await;

    let [low, high] = cara.to_le_bytes();
    let audio = loop {
        if let Heard::Binary(frame) = hear(&mut admin).await
            && frame[1..3] == [low, high]
        {
            break frame;
        }
    };
    assert_eq!(&audio[3..7], 22050_u32.to_le_bytes());
    assert_eq!(&audio[7..], b"It is noon.");
    let spoken = loop {
        let caption = until(&mut admin, "caption").await;
        if caption["source"] == cara {
            break caption;
        }
    };
    assert_eq!(
        (spoken["text"].clone(), spoken["final"].clone()),
        (json!("It is noon."), json!(true))
    );
    eventually(
        &w,
        room.meeting,
        "SELECT count(*) = 2 FROM meeting_utterances WHERE meeting_id = $1",
    )
    .await;

    say(&mut admin, json!({"kind": "end"})).await;
    until(&mut admin, "ended").await;
    eventually(
        &w,
        room.meeting,
        "SELECT summary IS NOT DISTINCT FROM 'Decisions: lunch at noon.' FROM meetings WHERE id = $1",
    )
    .await;
    eventually(
        &w,
        room.meeting,
        "SELECT EXISTS(SELECT 1 FROM chat_turns WHERE meeting_id = $1 AND meeting_summary
            AND message LIKE 'Meeting \"Sync\" ended. Summary of the meeting transcript%lunch at noon.')",
    )
    .await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_guests_named_question_is_a_talk_only_turn_as_the_owner() {
    let w = world().await;
    let room = voiced(&w, &["Cara, can you hear me?"]).await;
    let (mut admin, _) = member(&room.served, &room.admin, room.meeting).await;
    agent_slot(&mut admin).await;
    let mut dee = guest(&room.served, &room.link, "Dee").await;
    until(&mut dee, "welcome").await;
    speaks(&mut dee).await;
    let (_, actor, message, talk_only) = meeting_turn(&w, room.meeting).await;
    assert_eq!((actor, talk_only), (w.admin.id, true));
    assert_eq!(message, "Dee (guest): Cara, can you hear me?");
    eventually(
        &w,
        room.meeting,
        "SELECT EXISTS(SELECT 1 FROM meeting_utterances WHERE meeting_id = $1 AND guest_name = 'Dee')",
    )
    .await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn an_agent_uninvited_during_the_meeting_leaves_and_one_invited_joins_at_once() {
    let w = world().await;
    let room = voiced(&w, &[]).await;
    let (mut admin, _) = member(&room.served, &room.admin, room.meeting).await;
    agent_slot(&mut admin).await;
    let invitation = room
        .meetings
        .meeting(&w.admin, room.meeting)
        .await
        .unwrap()
        .invitations[0]
        .id;
    room.meetings
        .uninvite(&w.admin, room.meeting, invitation)
        .await
        .unwrap();
    loop {
        let roster = until(&mut admin, "roster").await;
        if roster["participants"].as_array().unwrap().len() == 1 {
            break;
        }
    }
    eventually(
        &w,
        room.meeting,
        "SELECT NOT EXISTS(SELECT 1 FROM meeting_attendance WHERE meeting_id = $1 AND agent_id IS NOT NULL AND left_at IS NULL)",
    )
    .await;
    let again = invitations(json!({"invite": {"agents": [room.cara]}}));
    room.meetings
        .invite(&w.admin, room.meeting, &again)
        .await
        .unwrap();
    agent_slot(&mut admin).await;
    eventually(
        &w,
        room.meeting,
        "SELECT EXISTS(SELECT 1 FROM meeting_attendance WHERE meeting_id = $1 AND agent_id IS NOT NULL AND left_at IS NULL)",
    )
    .await;
}

async fn agent_seat(socket: &mut Socket, check: impl Fn(&Value) -> bool) -> Value {
    loop {
        let roster = until(socket, "roster").await;
        let seats = roster["participants"].as_array().unwrap();
        if let Some(agent) = seats.iter().find(|seat| seat["kind"] == "agent")
            && check(agent)
        {
            return agent.clone();
        }
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_member_who_may_view_an_agents_desktop_shows_it_to_everyone_and_a_guest_may_not() {
    use cubebot_orchestrator::agents::sandbox::{Desktop, Runtime};
    let w = world().await;
    let cara = agent(&w, "Cara").await;
    let grant = GrantRequest {
        request_id: Uuid::new_v4(),
        recipient: Recipient::User { id: w.bo.id },
        capability: Capability::BrowserView,
        scope: Scope::Agent { id: cara },
    };
    w.identity.grant(&w.admin, &grant).await.unwrap();
    let desktop = Desktop::new(
        Arc::new(crate::desktop_http::StandIn),
        &Runtime::Container { class: None },
    );
    let meetings =
        Meetings::new(w.pool.clone(), None, Some(CONSOLE.into())).with_desktop(Some(desktop));
    let input = new(json!({"title": "Demo", "open_join": true,
        "invite": {"agents": [cara], "accounts": [w.bo.id, w.cy.id]}}));
    let (meeting, _) = meetings.create(&w.admin, &input).await.unwrap();
    let router =
        cubebot_orchestrator::api::meetings_router(w.pool.clone(), CONSOLE, meetings.clone());
    let (origin, task) = http_server(router).await;
    let served = Served {
        origin,
        _task: task,
    };

    let (mut bo, _) = member(&served, &token_of(&w, "bob").await, meeting.id).await;
    let seat = agent_seat(&mut bo, |agent| agent["can_present"] == true).await;
    let slot = seat["slot"].clone();
    let (mut cy, _) = member(&served, &token_of(&w, "cyd").await, meeting.id).await;
    let seen = agent_seat(&mut cy, |_| true).await;
    assert_eq!(seen["can_present"], false);
    let mut dee = guest(&served, meeting.link.as_ref().unwrap(), "Dee").await;
    let seen = agent_seat(&mut dee, |_| true).await;
    assert!(seen.get("can_present").is_none());

    say(
        &mut dee,
        json!({"kind": "present", "source": slot, "share": true}),
    )
    .await;
    let notice = until(&mut dee, "notice").await;
    assert_eq!(
        notice["text"],
        "Only signed-in members can ask an agent to share its screen."
    );
    say(
        &mut cy,
        json!({"kind": "present", "source": slot, "share": true}),
    )
    .await;
    let notice = until(&mut cy, "notice").await;
    assert_eq!(notice["text"], "You may not view this agent's desktop.");

    say(
        &mut bo,
        json!({"kind": "present", "source": slot, "share": true}),
    )
    .await;
    let frame = until(&mut dee, "desktop").await;
    assert_eq!(frame["source"], slot);
    assert_eq!(frame["tiles"][0]["png"], "iVBORw0KGgo=");
    agent_seat(&mut bo, |agent| agent["screen"] == true).await;
    until(&mut cy, "desktop").await;

    say(
        &mut bo,
        json!({"kind": "present", "source": slot, "share": false}),
    )
    .await;
    let ended = loop {
        let frame = until(&mut dee, "desktop").await;
        if frame["ended"] == true {
            break frame;
        }
    };
    assert_eq!((ended["source"].clone(), ended.get("tiles")), (slot, None));
    agent_seat(&mut bo, |agent| agent["screen"] == false).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_meeting_closed_at_startup_is_still_summarized_for_its_agents() {
    let w = world().await;
    let room = voiced(&w, &[]).await;
    sqlx::query!(
        "UPDATE meetings SET started_at=clock_timestamp() WHERE id=$1",
        room.meeting
    )
    .execute(&w.pool)
    .await
    .unwrap();
    cubebot_orchestrator::work::Work::new(w.pool.clone())
        .join_meeting(
            cubebot_orchestrator::agents::AgentId(room.cara),
            room.meeting,
        )
        .await
        .unwrap();
    sqlx::query!(
        "INSERT INTO meeting_utterances (meeting_id, agent_id, text) VALUES ($1, $2, 'It is noon.')",
        room.meeting,
        room.cara,
    )
    .execute(&w.pool)
    .await
    .unwrap();
    room.meetings.close_open().await.unwrap();
    eventually(
        &w,
        room.meeting,
        "SELECT ended_at IS NOT NULL AND summary IS NOT DISTINCT FROM 'Decisions: lunch at noon.' FROM meetings WHERE id = $1",
    )
    .await;
    eventually(
        &w,
        room.meeting,
        "SELECT EXISTS(SELECT 1 FROM chat_turns WHERE meeting_id = $1 AND meeting_summary)",
    )
    .await;
}

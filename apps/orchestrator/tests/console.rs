//! The console contract (docs/design/console.md): agent summaries by grant, read cursors, the
//! installation stream and the activity inbox, against real PostgreSQL.
use crate::support;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use cubebot_orchestrator::{
    agents::{Accessory, AgentId, Agents, Avatar, CreateAgent, Eyes, Paint},
    api,
    identity::{
        Actor, Identity,
        access::{Capability, GrantRequest, Recipient, Scope},
    },
    work::{CreateRoom, Work},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

struct Fixture {
    pool: sqlx::PgPool,
    identity: Identity,
    admin: Actor,
    member: Actor,
    member_id: Uuid,
    member_token: String,
    ada: Uuid,
    bob: Uuid,
}

async fn fixture() -> Fixture {
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
    let member_id = identity
        .create_account(&admin, "member", "member password", false, Uuid::new_v4())
        .await
        .unwrap();
    let member_token = identity
        .login("member", "member password")
        .await
        .unwrap()
        .token;
    let member = identity.authenticate(&member_token).await.unwrap();
    let agents = Agents::new(pool.clone());
    let mut ids = Vec::new();
    for (name, avatar) in [
        (
            "Ada",
            Some(Avatar {
                paint: Paint::Sky,
                eyes: Eyes::Wide,
                accessory: Accessory::Glasses,
            }),
        ),
        ("Bob", None),
    ] {
        let input = CreateAgent {
            request_id: Uuid::new_v4(),
            name: name.into(),
            instructions: format!("You are {name}."),
            avatar,
        };
        ids.push(agents.create(&admin, &input).await.unwrap().id);
    }
    Fixture {
        pool,
        identity,
        admin,
        member,
        member_id,
        member_token,
        ada: ids[0],
        bob: ids[1],
    }
}

impl Fixture {
    async fn grant(&self, scope: Scope, capability: Capability) {
        let request = GrantRequest {
            request_id: Uuid::new_v4(),
            recipient: Recipient::User { id: self.member_id },
            scope,
            capability,
        };
        self.identity.grant(&self.admin, &request).await.unwrap();
    }

    /// A turn in the agent's conversation; its sequence is the conversation's newest.
    async fn turn(&self, agent: Uuid, status: &str) -> (Uuid, i64) {
        let row = sqlx::query!(
            "
            INSERT INTO chat_turns (conversation_id, actor_id, request_id, message, status, error)
            VALUES ($1, $2, gen_random_uuid(), 'Look into it', $3, CASE WHEN $3 = 'failed' THEN 'provider refused' END)
            RETURNING id, sequence
            ",
            agent,
            self.admin.id,
            status,
        )
        .fetch_one(&self.pool)
        .await
        .unwrap();
        (row.id, row.sequence)
    }

    fn router(&self) -> axum::Router {
        api::database_router(self.pool.clone())
    }
}

async fn request(
    router: axum::Router,
    method: &str,
    path: &str,
    token: &str,
    value: Value,
) -> (StatusCode, Value) {
    let response = router
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(value.to_string()))
                .unwrap(),
        )
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
async fn summaries_show_every_agent_the_caller_holds_a_capability_on() {
    let f = fixture().await;
    let agents = Agents::new(f.pool.clone());
    assert!(agents.list(&f.member, None).await.unwrap().is_empty());

    // A conversation grant alone makes the agent visible, without its instructions.
    f.grant(
        Scope::Conversation { id: f.ada },
        Capability::ConversationSend,
    )
    .await;
    let visible = agents.list(&f.member, None).await.unwrap();
    assert_eq!(visible.len(), 1);
    let ada = &visible[0];
    assert_eq!(ada.id, AgentId(f.ada));
    assert_eq!(ada.capabilities, vec!["conversation_send"]);
    assert_eq!(ada.instructions, None);
    assert_eq!(
        serde_json::to_value(ada.avatar.0).unwrap(),
        json!({"paint":"sky","eyes":"wide","accessory":"glasses"})
    );
    assert!(!ada.desktop_active && !ada.pending_login);
    assert_eq!((ada.last_sequence, ada.read_sequence), (0, 0));
    assert!(agents.get(&f.member, AgentId(f.bob)).await.is_err());

    // Managing adds the instructions and the capability.
    f.grant(Scope::Agent { id: f.ada }, Capability::AgentManage)
        .await;
    let ada = agents.get(&f.member, AgentId(f.ada)).await.unwrap();
    assert_eq!(ada.capabilities, vec!["agent_manage", "conversation_send"]);
    assert_eq!(ada.instructions.as_deref(), Some("You are Ada."));

    // Administrators see everything with every capability; Bob has the default cube.
    let all = agents.list(&f.admin, None).await.unwrap();
    assert_eq!(all.len(), 2);
    let bob = all.iter().find(|agent| agent.id == AgentId(f.bob)).unwrap();
    assert_eq!(bob.capabilities.len(), 9);
    assert_eq!(bob.avatar.paint, Paint::Orange);
    assert_eq!(bob.instructions.as_deref(), Some("You are Bob."));

    // Live flags: a pending login, a desktop tool call in a running turn, the newest turn.
    let (turn, sequence) = f.turn(f.ada, "running").await;
    sqlx::query!(
        r#"
        INSERT INTO login_handoffs (agent_id, turn_id, call_id, site, reason)
        VALUES ($1, $2, gen_random_uuid(), 'github.com', 'Read your notifications')
        "#,
        f.ada,
        turn,
    )
    .execute(&f.pool)
    .await
    .unwrap();
    sqlx::query!(
        r#"
        INSERT INTO turn_events (turn_id, sequence, payload)
        VALUES ($1, 1, '{"kind":"tool_call","id":"c1","name":"browser_click","input":{}}')
        "#,
        turn,
    )
    .execute(&f.pool)
    .await
    .unwrap();
    let ada = agents.get(&f.member, AgentId(f.ada)).await.unwrap();
    assert!(ada.desktop_active && ada.pending_login);
    assert_eq!(ada.last_sequence, sequence);
    sqlx::query!(
        "UPDATE turn_events SET created_at = created_at - interval '121 seconds' WHERE turn_id = $1",
        turn
    )
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(
        !agents
            .get(&f.member, AgentId(f.ada))
            .await
            .unwrap()
            .desktop_active
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn read_cursors_only_move_forward_and_need_read_access() {
    let f = fixture().await;
    let conversation = format!("/api/conversations/{}/read", f.ada);
    let (status, _) = request(
        f.router(),
        "PUT",
        &conversation,
        &f.member_token,
        json!({"sequence": 5}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    f.grant(
        Scope::Conversation { id: f.ada },
        Capability::ConversationRead,
    )
    .await;
    for (sent, stored) in [(5, 5), (3, 5), (7, 7)] {
        let (status, body) = request(
            f.router(),
            "PUT",
            &conversation,
            &f.member_token,
            json!({"sequence": sent}),
        )
        .await;
        assert_eq!(
            (status, body["sequence"].as_i64()),
            (StatusCode::OK, Some(stored))
        );
    }
    let (status, _) = request(
        f.router(),
        "PUT",
        &conversation,
        &f.member_token,
        json!({"sequence": -1}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let ada = Agents::new(f.pool.clone())
        .get(&f.member, AgentId(f.ada))
        .await
        .unwrap();
    assert_eq!(ada.read_sequence, 7);

    let work = Work::new(f.pool.clone());
    let room = work
        .create_room(
            &f.admin,
            &CreateRoom {
                request_id: Uuid::new_v4(),
                name: "Launch".into(),
                agents: vec![f.ada],
            },
        )
        .await
        .unwrap();
    assert_eq!((room.last_sequence, room.read_sequence), (0, 0));
    let path = format!("/api/rooms/{}/read", room.id);
    let (status, _) = request(
        f.router(),
        "PUT",
        &path,
        &f.member_token,
        json!({"sequence": 2}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    f.grant(Scope::Room { id: room.id }, Capability::ConversationRead)
        .await;
    let (status, body) = request(
        f.router(),
        "PUT",
        &path,
        &f.member_token,
        json!({"sequence": 2}),
    )
    .await;
    assert_eq!(
        (status, body["sequence"].as_i64()),
        (StatusCode::OK, Some(2))
    );
    let (_, rooms) = request(
        f.router(),
        "GET",
        "/api/rooms",
        &f.member_token,
        Value::Null,
    )
    .await;
    assert_eq!(rooms[0]["read_sequence"], 2);
    assert_eq!(rooms[0]["last_sequence"], 0);
}

/// SSE frames from the stream's first poll: (event name, data).
async fn first_frames(router: axum::Router, token: &str, count: usize) -> Vec<(String, Value)> {
    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/events")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut body = response.into_body();
    let mut frames = Vec::new();
    while frames.len() < count {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("a frame within five seconds")
            .unwrap()
            .unwrap();
        let text = String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap();
        let mut event = String::new();
        let mut data = Value::Null;
        for line in text.lines() {
            if let Some(name) = line.strip_prefix("event: ") {
                event = name.into();
            } else if let Some(json) = line.strip_prefix("data: ") {
                data = serde_json::from_str(json).unwrap();
            }
        }
        if !event.is_empty() {
            frames.push((event, data));
        }
    }
    frames
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn the_installation_stream_opens_with_every_visible_agent_and_room() {
    let f = fixture().await;
    let room = Work::new(f.pool.clone())
        .create_room(
            &f.admin,
            &CreateRoom {
                request_id: Uuid::new_v4(),
                name: "Launch".into(),
                agents: vec![f.ada, f.bob],
            },
        )
        .await
        .unwrap();
    f.grant(Scope::Agent { id: f.ada }, Capability::BrowserView)
        .await;
    f.grant(Scope::Room { id: room.id }, Capability::ConversationRead)
        .await;
    let (turn, sequence) = f.turn(f.ada, "waiting").await;
    let _ = turn;

    let frames = first_frames(f.router(), &f.member_token, 2).await;
    assert_eq!(frames[0].0, "agent");
    assert_eq!(frames[0].1["id"], Value::String(f.ada.to_string()));
    assert_eq!(frames[0].1["capabilities"], json!(["browser_view"]));
    assert_eq!(frames[0].1["last_sequence"], sequence);
    assert_eq!(frames[0].1["instructions"], Value::Null);
    assert_eq!(frames[1].0, "room");
    assert_eq!(frames[1].1, json!({"id": room.id, "last_sequence": 0}));

    let admin_token = f
        .identity
        .login("admin", "administrator password")
        .await
        .unwrap()
        .token;
    let frames = first_frames(f.router(), &admin_token, 3).await;
    let agents: Vec<_> = frames
        .iter()
        .filter(|(event, _)| event == "agent")
        .map(|(_, data)| data["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        agents,
        vec![f.ada.to_string(), f.bob.to_string()]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    );
    assert_eq!(frames[2].0, "room");

    let response = f
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/events")
                .header("authorization", "Bearer not-a-session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn activity_lists_what_needs_the_caller_newest_first() {
    let f = fixture().await;
    let work = Work::new(f.pool.clone());
    let (running, _) = f.turn(f.ada, "running").await;
    sqlx::query!(
        "
        INSERT INTO login_handoffs (agent_id, turn_id, call_id, site, reason)
        VALUES ($1, $2, gen_random_uuid(), 'github.com', 'Read your notifications')
        ",
        f.ada,
        running,
    )
    .execute(&f.pool)
    .await
    .unwrap();
    let (waiting, _) = f.turn(f.ada, "waiting").await;
    let (failed, _) = f.turn(f.bob, "failed").await;
    f.identity
        .invite(&f.admin, "newcomer", Uuid::new_v4())
        .await
        .unwrap();
    let attempt = sqlx::query_scalar!(
        "
        INSERT INTO agent_resources (id, agent_id, kind, epoch, target_generation, definition_revision, namespace, name, manifest, source_manifest)
        VALUES (gen_random_uuid(), $1, 'pod', 0, 0, 1, 'test', 'ada-0', '{}', '{}')
        RETURNING id
        ",
        f.ada,
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    let dream = sqlx::query_scalar!(
        "
        INSERT INTO dream_runs (id, agent_id, attempt_id, model, status, ended_at, outcome)
        VALUES (gen_random_uuid(), $1, $2, 'dream-model', 'finished', clock_timestamp(), 'completed')
        RETURNING id
        ",
        f.ada,
        attempt,
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    sqlx::query!(
        r#"
        WITH skill AS (INSERT INTO skills (name) VALUES ('release-notes') RETURNING id)
        INSERT INTO skill_revisions (skill_id, revision, description, files, dream_run_id)
        SELECT id, 1, 'How Ada writes release notes', '{"SKILL.md":"Release notes"}', $1 FROM skill
        "#,
        dream,
    )
    .execute(&f.pool)
    .await
    .unwrap();

    let items = work.activity(&f.admin).await.unwrap();
    let kinds: Vec<&str> = items.iter().map(|item| item.kind.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "dream_published_skill",
            "invitation_pending",
            "turn_failed",
            "turn_waiting",
            "login_pending",
        ]
    );
    assert_eq!(items[0].text, "Ada published skill release-notes");
    assert_eq!(items[1].text, "Invitation for newcomer is pending");
    assert_eq!(items[2].text, "Bob failed: provider refused");
    assert_eq!((items[2].turn, items[2].agent), (Some(failed), Some(f.bob)));
    assert_eq!(
        (items[3].turn, items[3].agent),
        (Some(waiting), Some(f.ada))
    );
    assert_eq!(items[4].text, "Ada asks you to log in on github.com");
    assert_eq!(items[4].turn, Some(running));
    assert!(items.iter().all(|item| item.at.starts_with("20")));

    assert!(work.activity(&f.member).await.unwrap().is_empty());
    f.grant(
        Scope::Conversation { id: f.ada },
        Capability::ConversationRead,
    )
    .await;
    let kinds: Vec<String> = work
        .activity(&f.member)
        .await
        .unwrap()
        .into_iter()
        .map(|item| item.kind)
        .collect();
    assert_eq!(kinds, ["turn_waiting", "login_pending"]);
    f.grant(Scope::Agent { id: f.ada }, Capability::MemoryRead)
        .await;
    assert_eq!(work.activity(&f.member).await.unwrap().len(), 3);

    let (status, body) = request(
        f.router(),
        "GET",
        "/api/activity",
        &f.member_token,
        Value::Null,
    )
    .await;
    assert_eq!(
        (status, body.as_array().map(Vec::len)),
        (StatusCode::OK, Some(3))
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn preferences_and_installation_are_per_session() {
    let f = fixture().await;
    let (status, body) = request(
        f.router(),
        "GET",
        "/api/identity/me",
        &f.member_token,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (body["login"].as_str(), body["name"].clone()),
        (Some("member"), Value::Null)
    );
    let (_, body) = request(
        f.router(),
        "GET",
        "/api/identity/me/preferences",
        &f.member_token,
        Value::Null,
    )
    .await;
    assert_eq!(
        body,
        json!({"favorites": [], "theme": "system", "collapsed": []})
    );
    let document = json!({"favorites": [{"kind": "agent", "id": f.ada}], "theme": "dark", "collapsed": ["rooms"]});
    let (status, body) = request(
        f.router(),
        "PUT",
        "/api/identity/me/preferences",
        &f.member_token,
        document.clone(),
    )
    .await;
    assert_eq!((status, body.clone()), (StatusCode::OK, document.clone()));
    let (_, body) = request(
        f.router(),
        "GET",
        "/api/identity/me/preferences",
        &f.member_token,
        Value::Null,
    )
    .await;
    assert_eq!(body, document);
    let (status, _) = request(
        f.router(),
        "PUT",
        "/api/identity/me/preferences",
        &f.member_token,
        json!({"theme": "neon"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = request(
        f.router(),
        "PUT",
        "/api/identity/me/preferences",
        &f.member_token,
        json!({"collapsed": vec!["section"; 101]}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = request(
        f.router(),
        "GET",
        "/api/installation",
        &f.member_token,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["controller"], "database_only");
    assert_eq!(body["terminal_enabled"], false);
    assert!(body["name"].is_string());
}

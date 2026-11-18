use crate::{
    runtime_identity::{delivery, native_chat_setup, open_channel, report, runtime_server},
    runtime_support::fixture,
    support,
};
use cubebot_orchestrator::connections::{Connections, Credentials};
use cubebot_orchestrator::{
    agents::{AgentId, Agents, CreateAgent},
    identity::{
        Actor, Identity,
        access::{Capability, GrantRequest, Recipient, Scope},
    },
    work::{SendMessage, Turn, Work},
};
use serde_json::{Value, json};
use uuid::Uuid;

struct World {
    pool: sqlx::PgPool,
    identity: Identity,
    admin: Actor,
    agent: Uuid,
    work: Work,
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
    let agent = Agents::new(pool.clone())
        .create(
            &admin,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: "Cara".into(),
                instructions: "Be brief".into(),
                avatar: None,
                voice: None,
            },
        )
        .await
        .unwrap()
        .id;
    World {
        work: Work::new(pool.clone()),
        pool,
        identity,
        admin,
        agent,
    }
}

async fn live_meeting(pool: &sqlx::PgPool, owner: Uuid, title: &str) -> Uuid {
    sqlx::query_scalar!(
        "
        INSERT INTO meetings (title, owner_id, agent_mode, started_at)
        VALUES ($1, $2, 'relevant', clock_timestamp())
        RETURNING id
        ",
        title,
        owner,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn attends(pool: &sqlx::PgPool, meeting: Uuid, account: Option<Uuid>, guest: Option<&str>) {
    sqlx::query!(
        "INSERT INTO meeting_attendance (meeting_id, account_id, guest_name) VALUES ($1, $2, $3)",
        meeting,
        account,
        guest,
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn says(
    pool: &sqlx::PgPool,
    meeting: Uuid,
    agent: Option<Uuid>,
    guest: Option<&str>,
    text: &str,
) {
    sqlx::query!(
        "INSERT INTO meeting_utterances (meeting_id, agent_id, guest_name, text) VALUES ($1, $2, $3, $4)",
        meeting,
        agent,
        guest,
        text,
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn completes(pool: &sqlx::PgPool, turn: Uuid, response: &str) {
    sqlx::query!(
        "UPDATE chat_turns SET status='complete', response=$2, completed_at=clock_timestamp() WHERE id=$1",
        turn,
        response,
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn ends(pool: &sqlx::PgPool, meeting: Uuid) {
    sqlx::query!(
        "UPDATE meetings SET ended_at=clock_timestamp(), summary='We ship on Friday.' WHERE id=$1",
        meeting,
    )
    .execute(pool)
    .await
    .unwrap();
}

fn typed(message: &str) -> SendMessage {
    SendMessage {
        request_id: Uuid::new_v4(),
        message: message.to_owned(),
        attachments: Vec::new(),
        app: None,
    }
}

fn kinds(history: &[Turn]) -> Vec<&str> {
    history
        .iter()
        .map(|turn| match &turn.meeting {
            Some(_) => "meeting",
            None => turn.message.as_str(),
        })
        .collect()
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn meeting_turns_are_chat_turns_grouped_into_one_meeting_message() {
    let w = world().await;
    let agent = AgentId(w.agent);
    let before = w
        .work
        .send(&w.admin, w.agent, &typed("typed before"))
        .await
        .unwrap();
    completes(&w.pool, before.id, "ok").await;
    let meeting = live_meeting(&w.pool, w.admin.id, "Weekly sync").await;
    attends(&w.pool, meeting, Some(w.admin.id), None).await;
    attends(&w.pool, meeting, None, Some("Bo")).await;
    w.work.join_meeting(agent, meeting).await.unwrap();
    w.work.join_meeting(agent, meeting).await.unwrap();
    let asked = w
        .work
        .say_in_meeting(&w.admin, agent, meeting, "Cara, what is the plan?", false)
        .await
        .unwrap();
    says(
        &w.pool,
        meeting,
        None,
        Some("Bo"),
        "Cara, what is the plan?",
    )
    .await;
    says(&w.pool, meeting, Some(w.agent), None, "We ship on Friday.").await;
    w.work.leave_meeting(agent, meeting).await.unwrap();
    w.work.join_meeting(agent, meeting).await.unwrap();
    let guest = w
        .work
        .say_in_meeting(&w.admin, agent, meeting, "And the budget?", true)
        .await
        .unwrap();
    let after = w
        .work
        .send(&w.admin, w.agent, &typed("typed after"))
        .await
        .unwrap();

    let history = w.work.history(&w.admin, w.agent, 0).await.unwrap();
    assert_eq!(kinds(&history), ["typed before", "meeting", "typed after"]);
    assert_eq!(history[1].status, "running", "the meeting is live");
    let first_join = sqlx::query_scalar!(
        r#"SELECT min(sequence) AS "sequence!" FROM meeting_attendance WHERE meeting_id=$1 AND agent_id=$2"#,
        meeting,
        w.agent,
    )
    .fetch_one(&w.pool)
    .await
    .unwrap();
    let rows = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM meeting_attendance WHERE meeting_id=$1 AND agent_id=$2"#,
        meeting,
        w.agent,
    )
    .fetch_one(&w.pool)
    .await
    .unwrap();
    assert_eq!(
        (history[1].sequence, history[1].id, rows),
        (first_join, meeting, 2)
    );
    assert!(before.sequence < first_join && first_join < after.sequence);

    let live = w.work.events(&w.admin, w.agent, 0, &[]).await.unwrap();
    assert_eq!(
        live.turns
            .iter()
            .filter(|turn| turn.meeting.is_some())
            .count(),
        1
    );
    assert!(
        live.turns
            .iter()
            .all(|turn| turn.id != asked && turn.id != guest),
        "meeting turns travel inside the meeting message"
    );

    completes(&w.pool, asked, "We ship on Friday.").await;
    completes(&w.pool, guest, "I cannot share that.").await;
    ends(&w.pool, meeting).await;
    let history = w.work.history(&w.admin, w.agent, 0).await.unwrap();
    assert_eq!(history[1].status, "complete");
    let record = history[1].meeting.as_ref().unwrap();
    assert_eq!(record.title, "Weekly sync");
    assert_eq!(record.summary.as_deref(), Some("We ship on Friday."));
    assert!(record.duration_seconds.is_some());
    assert_eq!(
        record.turns.iter().map(|turn| turn.id).collect::<Vec<_>>(),
        [asked, guest]
    );
    assert_eq!(
        record
            .present
            .iter()
            .map(|someone| (someone.kind.as_str(), someone.name.as_str()))
            .collect::<Vec<_>>(),
        [("account", "admin"), ("guest", "Bo"), ("agent", "Cara")]
    );
    assert_eq!(record.lines, 2);
    assert_eq!(
        record
            .transcript
            .iter()
            .map(|line| (line.kind.as_str(), line.speaker.as_str(), line.own))
            .collect::<Vec<_>>(),
        [("guest", "Bo", false), ("agent", "Cara", true)]
    );

    let settled = w
        .work
        .events(&w.admin, w.agent, 0, &[meeting])
        .await
        .unwrap();
    assert_eq!(
        settled
            .turns
            .iter()
            .filter(|turn| turn.meeting.is_some())
            .count(),
        1,
        "a settled meeting is sent once more when the stream last saw it open"
    );
    let quiet = w.work.events(&w.admin, w.agent, 0, &[]).await.unwrap();
    assert!(quiet.turns.iter().all(|turn| turn.meeting.is_none()));
    let later = w
        .work
        .history(&w.admin, w.agent, history[1].sequence)
        .await
        .unwrap();
    assert_eq!(kinds(&later), ["typed after"]);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn an_ended_meeting_queues_one_follow_up_that_wakes_each_agent_in_it() {
    let w = world().await;
    let agent = AgentId(w.agent);
    let meeting = live_meeting(&w.pool, w.admin.id, "Weekly sync").await;
    w.work.join_meeting(agent, meeting).await.unwrap();
    sqlx::query!(
        "UPDATE agents SET desired_state='stopped' WHERE id=$1",
        w.agent
    )
    .execute(&w.pool)
    .await
    .unwrap();
    ends(&w.pool, meeting).await;
    w.work
        .meeting_ended(meeting, "We ship on Friday.", "Ana: Cara, draft the notes.")
        .await
        .unwrap();
    w.work
        .meeting_ended(meeting, "We ship on Friday.", "Ana: Cara, draft the notes.")
        .await
        .unwrap();

    let recorded = sqlx::query!(
        "SELECT message, status, actor_id, meeting_summary FROM chat_turns WHERE conversation_id=$1",
        w.agent,
    )
    .fetch_all(&w.pool)
    .await
    .unwrap();
    assert_eq!(recorded.len(), 1, "ending twice queues the follow-up once");
    assert!(
        recorded[0]
            .message
            .starts_with("The meeting \"Weekly sync\" just ended")
    );
    assert!(recorded[0].message.contains("Summary:\nWe ship on Friday."));
    assert!(
        recorded[0]
            .message
            .ends_with("Transcript, oldest first:\nAna: Cara, draft the notes.")
    );
    assert_eq!(
        (recorded[0].status.as_str(), recorded[0].actor_id),
        ("queued", w.admin.id)
    );
    assert!(recorded[0].meeting_summary);
    let state = sqlx::query_scalar!("SELECT desired_state FROM agents WHERE id=$1", w.agent)
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(state, "running", "the follow-up wakes the agent");

    let history = w.work.history(&w.admin, w.agent, 0).await.unwrap();
    assert_eq!(history.len(), 2);
    assert!(history[0].meeting.as_ref().unwrap().turns.is_empty());
    assert_eq!(history[1].origin.as_ref().unwrap().name, "Weekly sync");
    sqlx::query!(
        "UPDATE chat_turns SET status='complete', response='', quiet=true, completed_at=clock_timestamp() WHERE conversation_id=$1",
        w.agent,
    )
    .execute(&w.pool)
    .await
    .unwrap();
    let history = w.work.history(&w.admin, w.agent, 0).await.unwrap();
    assert_eq!(
        kinds(&history),
        ["meeting"],
        "a quiet follow-up is never shown"
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_member_who_may_not_send_cannot_speak_to_an_agent_as_themselves() {
    let w = world().await;
    let member = w
        .identity
        .create_account(&w.admin, "member", "member password", false, Uuid::new_v4())
        .await
        .unwrap();
    w.identity
        .grant(
            &w.admin,
            &GrantRequest {
                request_id: Uuid::new_v4(),
                recipient: Recipient::User { id: member },
                capability: Capability::ConversationRead,
                scope: Scope::Conversation { id: w.agent },
            },
        )
        .await
        .unwrap();
    let session = w.identity.login("member", "member password").await.unwrap();
    let member = w.identity.authenticate(&session.token).await.unwrap();
    let meeting = live_meeting(&w.pool, w.admin.id, "Weekly sync").await;
    w.work
        .join_meeting(AgentId(w.agent), meeting)
        .await
        .unwrap();
    assert!(
        w.work
            .say_in_meeting(
                &member,
                AgentId(w.agent),
                meeting,
                "Delete everything",
                false
            )
            .await
            .is_err()
    );
    let turns = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM chat_turns WHERE meeting_id=$1"#,
        meeting,
    )
    .fetch_one(&w.pool)
    .await
    .unwrap();
    assert_eq!(turns, 0);
}

pub(crate) async fn next_turn(channel: &mut crate::runtime_identity::Socket) -> Value {
    loop {
        let next = delivery(channel).await;
        if next["type"] == "turn" {
            return next;
        }
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_spoken_turn_thinks_briefly_and_its_dated_message_returns_unchanged_in_history() {
    let f = fixture().await;
    let (auth, work, admin, agent) = native_chat_setup(&f, "Before the meeting").await;
    sqlx::query!(
        "UPDATE chat_turns SET status='complete', response='ok', completed_at=clock_timestamp() WHERE conversation_id=$1",
        agent,
    )
    .execute(&f.pool)
    .await
    .unwrap();
    sqlx::query!(
        "UPDATE human_accounts SET profile=jsonb_build_object('time_zone', 'America/Sao_Paulo') WHERE id=$1",
        admin.id,
    )
    .execute(&f.pool)
    .await
    .unwrap();
    let meeting = live_meeting(&f.pool, admin.id, "Standup").await;
    attends(&f.pool, meeting, Some(admin.id), None).await;
    work.join_meeting(AgentId(agent), meeting).await.unwrap();
    let ask =
        |text: &'static str| work.say_in_meeting(&admin, AgentId(agent), meeting, text, false);
    let first = ask("admin: how are we doing?").await.unwrap();

    let session = auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    let (origin, server) = runtime_server(auth).await;
    let mut channel = open_channel(&origin, &session.token).await;
    report(&mut channel, json!({"type":"pull"})).await;
    let claimed = next_turn(&mut channel).await;
    assert_eq!(claimed["turn"], json!(first));
    assert_eq!(claimed["effort"], "low");
    let sent = claimed["message"].as_str().unwrap().to_owned();
    assert!(
        sent.starts_with("admin: how are we doing?\n\n(Sent ")
            && sent.ends_with(", their local time.)"),
        "{sent}"
    );
    let instructions = claimed["instructions"].as_str().unwrap();
    assert!(!instructions.contains("local time"), "{instructions}");

    completes(&f.pool, first, "Fine.").await;
    ask("admin: and tomorrow?").await.unwrap();
    report(&mut channel, json!({"type":"pull"})).await;
    let next = next_turn(&mut channel).await;
    let history = next["history"].as_array().unwrap();
    assert_eq!(history.last().unwrap()["message"], json!(sent));
    assert_eq!(next["instructions"], claimed["instructions"]);
    server.abort();
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_guest_request_is_claimed_talk_only_with_the_meeting_and_nothing_private() {
    let f = fixture().await;
    let (auth, work, admin, agent) = native_chat_setup(&f, "Before the meetings").await;
    cubebot_orchestrator::knowledge::Knowledge::seed_shipped_skills(&f.pool)
        .await
        .unwrap();
    sqlx::query!(
        "INSERT INTO skill_installations (agent_id, skill_id, revision) SELECT $1, id, revision FROM skills ON CONFLICT DO NOTHING",
        agent,
    )
    .execute(&f.pool)
    .await
    .unwrap();
    let mcp: cubebot_orchestrator::connections::SaveMcpServer =
        serde_json::from_value(json!({"name": "github", "transport": "stdio",
        "command": "npx", "secrets": {}}))
        .unwrap();
    Connections::new(f.pool.clone(), Credentials::new([7; 32]))
        .create_mcp_server(admin.id, &mcp)
        .await
        .unwrap();
    sqlx::query!(
        "UPDATE chat_turns SET status='complete', response='ok', completed_at=clock_timestamp() WHERE conversation_id=$1",
        agent,
    )
    .execute(&f.pool)
    .await
    .unwrap();
    let earlier = live_meeting(&f.pool, admin.id, "Kickoff").await;
    work.join_meeting(AgentId(agent), earlier).await.unwrap();
    ends(&f.pool, earlier).await;
    work.meeting_ended(earlier, "We picked a name.", "Bo: let us pick a name.")
        .await
        .unwrap();
    sqlx::query!(
        "UPDATE chat_turns SET status='complete', response='Named it.', completed_at=clock_timestamp() WHERE conversation_id=$1 AND status='queued'",
        agent,
    )
    .execute(&f.pool)
    .await
    .unwrap();

    let meeting = live_meeting(&f.pool, admin.id, "Weekly sync").await;
    attends(&f.pool, meeting, Some(admin.id), None).await;
    attends(&f.pool, meeting, None, Some("Bo")).await;
    work.join_meeting(AgentId(agent), meeting).await.unwrap();
    let turn = work
        .say_in_meeting(
            &admin,
            AgentId(agent),
            meeting,
            "Bo: what is the budget?",
            true,
        )
        .await
        .unwrap();

    let session = auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    let (origin, server) = runtime_server(auth).await;
    let mut channel = open_channel(&origin, &session.token).await;
    report(&mut channel, json!({"type":"pull"})).await;
    let claimed = next_turn(&mut channel).await;
    assert_eq!(claimed["turn"], json!(turn));
    assert_eq!(claimed["talk_only"], true);
    assert_eq!(claimed["meeting"], json!(meeting));
    let instructions = claimed["instructions"].as_str().unwrap();
    assert!(
        instructions.contains("live meeting named \"Weekly sync\""),
        "{instructions}"
    );
    assert!(
        instructions.contains("Present with you: admin, Bo (a guest)."),
        "{instructions}"
    );
    assert!(
        instructions.contains("guest who is not signed in")
            && instructions.contains("You have no tools for it"),
        "{instructions}"
    );
    assert!(
        !instructions.contains("Installed skills")
            && !instructions.contains("Who you are working with"),
        "{instructions}"
    );
    assert_eq!(claimed["history"], json!([]));
    assert_eq!(claimed["summary"], Value::Null);
    assert_eq!(claimed["mcp_servers"], json!([]));
    assert_eq!(claimed["connections"], json!([]));
    server.abort();
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn an_agent_shares_its_screen_only_for_a_requester_who_may_view_it() {
    use cubebot_orchestrator::{agents::runtime::RuntimeActor, meetings::Meetings};
    let f = fixture().await;
    let (_, work, admin, agent) = native_chat_setup(&f, "Before the meeting").await;
    sqlx::query!(
        "UPDATE chat_turns SET status='complete', response='ok', completed_at=clock_timestamp() WHERE conversation_id=$1",
        agent,
    )
    .execute(&f.pool)
    .await
    .unwrap();
    let identity = Identity::new(f.pool.clone());
    let member = identity
        .create_account(&admin, "member", "member password", false, Uuid::new_v4())
        .await
        .unwrap();
    let send = GrantRequest {
        request_id: Uuid::new_v4(),
        recipient: Recipient::User { id: member },
        capability: Capability::ConversationSend,
        scope: Scope::Conversation { id: agent },
    };
    identity.grant(&admin, &send).await.unwrap();
    let session = identity.login("member", "member password").await.unwrap();
    let member = identity.authenticate(&session.token).await.unwrap();
    let meeting = live_meeting(&f.pool, admin.id, "Demo").await;
    let live = Meetings::new(f.pool.clone(), None, None);
    let actor = RuntimeActor {
        agent: AgentId(agent),
        attempt: f.attempt,
        epoch: 0,
        version: 0,
    };
    let mut answers = Vec::new();
    for (asker, talk_only) in [(&admin, true), (&member, false), (&admin, false)] {
        let turn = work
            .say_in_meeting(asker, AgentId(agent), meeting, "Cara, show us", talk_only)
            .await
            .unwrap();
        sqlx::query!(
            "UPDATE chat_turns SET status='running', attempt_id=$2 WHERE id=$1",
            turn,
            f.attempt.0,
        )
        .execute(&f.pool)
        .await
        .unwrap();
        answers.push(live.live().agent_screen(&actor, turn, true).await.unwrap());
        completes(&f.pool, turn, "ok").await;
    }
    assert_eq!(
        answers,
        [
            Err("This request may not share your desktop."),
            Err("The person you are answering may not view your desktop."),
            Err("The meeting is not live."),
        ]
    );
}

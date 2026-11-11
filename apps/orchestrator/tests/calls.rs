use crate::{desktop_http::ORIGIN, runtime_identity::http_server, support};
use cubebot_orchestrator::{
    agents::{AgentId, Agents, CreateAgent},
    identity::{
        Actor, Identity,
        access::{Capability, GrantRequest, Recipient, Scope},
    },
    speech::{Activity, Audio, Engines, Speech, SpeechError, Stt, Tts, Vad},
    work::{SendMessage, Work},
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use uuid::Uuid;

struct Loud(bool);
impl Vad for Loud {
    fn accept(&mut self, window: &[f32]) -> Activity {
        let speaking = window[0] > 0.0;
        let ended = self.0 && !speaking;
        self.0 = speaking;
        Activity { speaking, ended }
    }
}
struct Hears;
impl Stt for Hears {
    fn accept(&mut self, _: &[f32]) -> Result<Option<String>, SpeechError> {
        Ok(Some("what".to_owned()))
    }
    fn finish(&mut self) -> Result<String, SpeechError> {
        Ok("what time is it".to_owned())
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

fn stand_in() -> Speech {
    Speech::stand_in(Arc::new(|| Engines {
        vad: Box::new(Loud(false)),
        stt: Box::new(Hears),
        tts: Box::new(Says),
    }))
}

struct World {
    pool: sqlx::PgPool,
    identity: Identity,
    admin: Actor,
    token: String,
    agent: Uuid,
    server: String,
    task: tokio::task::JoinHandle<()>,
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
                name: "Caller".into(),
                instructions: "Be brief".into(),
                avatar: None,
                voice: Some(cubebot_orchestrator::speech::Voice::Male),
            },
        )
        .await
        .unwrap()
        .id;
    let router = cubebot_orchestrator::api::calls_router(pool.clone(), ORIGIN, stand_in());
    let (server, task) = http_server(router).await;
    World {
        pool,
        identity,
        admin,
        token: session.token,
        agent,
        server,
        task,
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn open(
    world: &World,
    token: &str,
    origin: &str,
) -> Result<Socket, tokio_tungstenite::tungstenite::Error> {
    let url = format!(
        "{}/api/agents/{}/call",
        world.server.replace("http", "ws"),
        world.agent
    );
    let mut request = url.into_client_request().unwrap();
    let cookie = format!("__Host-cubebot_session={token}").parse().unwrap();
    request.headers_mut().insert("cookie", cookie);
    request
        .headers_mut()
        .insert("origin", origin.parse().unwrap());
    Ok(tokio_tungstenite::connect_async(request).await?.0)
}

async fn next_json(socket: &mut Socket) -> Value {
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(10), socket.next())
            .await
            .expect("the call answers")
            .expect("the call stays open")
            .unwrap()
        {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Binary(_) => panic!("audio where a message was expected"),
            _ => {}
        }
    }
}

async fn until(socket: &mut Socket, kind: &str) -> Value {
    loop {
        let message = next_json(socket).await;
        if message["kind"] == kind {
            return message;
        }
    }
}

fn pcm(sample: i16, windows: usize) -> Message {
    let frame: Vec<u8> = std::iter::repeat_n(sample.to_le_bytes(), 512 * windows)
        .flatten()
        .collect();
    Message::Binary(frame.into())
}

async fn replies(pool: &sqlx::PgPool, turn: Uuid, parts: &[&str]) {
    sqlx::query!("UPDATE chat_turns SET status='running' WHERE id=$1", turn)
        .execute(pool)
        .await
        .unwrap();
    for (sequence, part) in parts.iter().enumerate() {
        sqlx::query!(
            "INSERT INTO turn_events (turn_id, sequence, payload) VALUES ($1, $2, $3)",
            turn,
            sequence as i64 + 1,
            json!({"kind":"text_delta","text":part}),
        )
        .execute(pool)
        .await
        .unwrap();
    }
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

fn turn_of(message: &Value) -> Uuid {
    message["turn"].as_str().unwrap().parse().unwrap()
}

async fn next_audio(socket: &mut Socket) -> Vec<u8> {
    loop {
        let message = tokio::time::timeout(std::time::Duration::from_secs(10), socket.next())
            .await
            .expect("audio arrives")
            .expect("the call stays open")
            .unwrap();
        if let Message::Binary(frame) = message {
            return frame.to_vec();
        }
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_call_is_one_message_whose_turns_are_ordinary_chat_turns() {
    let world = world().await;
    let work = Work::new(world.pool.clone());
    let typed = |message: &str| SendMessage {
        request_id: Uuid::new_v4(),
        message: message.to_owned(),
        attachments: Vec::new(),
    };
    let before = work
        .send(&world.admin, world.agent, &typed("typed before"))
        .await
        .unwrap();
    completes(&world.pool, before.id, "ok").await;

    let mut call = open(&world, &world.token, ORIGIN).await.unwrap();
    let ready = next_json(&mut call).await;
    assert_eq!(
        (
            ready["kind"].as_str(),
            ready["stt"].as_str(),
            ready["voice"].as_str()
        ),
        (Some("ready"), Some("embedded"), Some("male")),
        "{ready}"
    );

    call.send(pcm(16384, 1)).await.unwrap();
    assert_eq!(next_json(&mut call).await["kind"], "interrupt");
    assert_eq!(next_json(&mut call).await["text"], "what");
    call.send(pcm(0, 1)).await.unwrap();
    let said = until(&mut call, "user").await;
    assert_eq!(said["text"], "what time is it");
    let first = turn_of(&said);

    replies(&world.pool, first, &["It is noon. ", "**Enjoy** lunch"]).await;
    let audio = next_audio(&mut call).await;
    assert_eq!(&audio[..4], 22050u32.to_le_bytes());
    assert_eq!(&audio[4..], b"It is noon.");
    completes(&world.pool, first, "It is noon. **Enjoy** lunch").await;
    assert_eq!(&next_audio(&mut call).await[4..], b"Enjoy lunch");

    call.send(pcm(16384, 1)).await.unwrap();
    call.send(pcm(0, 1)).await.unwrap();
    let second = turn_of(&until(&mut call, "user").await);
    call.send(pcm(16384, 1)).await.unwrap();
    until(&mut call, "interrupt").await;
    replies(&world.pool, second, &["Never heard. "]).await;
    let shown = until(&mut call, "agent").await;
    assert_eq!(shown["text"], "Never heard. ");
    completes(&world.pool, second, "Never heard.").await;
    until(&mut call, "answered").await;
    call.close(None).await.unwrap();

    let after = work
        .send(&world.admin, world.agent, &typed("typed after"))
        .await
        .unwrap();
    completes(&world.pool, after.id, "ok").await;

    let messages = async {
        for _ in 0..50 {
            let history = work.history(&world.admin, world.agent, 0).await.unwrap();
            if history[1]
                .call
                .as_ref()
                .is_some_and(|call| call.duration_seconds.is_some())
            {
                return history;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        panic!("the closed call was not ended");
    }
    .await;
    assert_eq!(
        messages
            .iter()
            .map(|turn| turn.call.is_some())
            .collect::<Vec<_>>(),
        [false, true, false]
    );
    assert_eq!(messages[0].message, "typed before");
    assert_eq!(messages[2].message, "typed after");
    let record = messages[1].call.as_ref().unwrap();
    assert_eq!(messages[1].status, "complete");
    assert_eq!(record.turns.len(), 2);
    assert_eq!(record.turns[0].message, "what time is it");
    assert_eq!(
        record.turns[0].response.as_deref(),
        Some("It is noon. **Enjoy** lunch")
    );

    let later = work
        .history(&world.admin, world.agent, messages[1].sequence)
        .await
        .unwrap();
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].message, "typed after");

    let on_call = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM chat_turns WHERE conversation_id=$1 AND call_id IS NOT NULL AND actor_id=$2"#,
        world.agent,
        world.admin.id,
    )
    .fetch_one(&world.pool)
    .await
    .unwrap();
    assert_eq!(on_call, 2);
    let conversation = world.agent;
    let page = work
        .events(&world.admin, conversation, 0, &[messages[1].id])
        .await
        .unwrap();
    assert!(
        page.events.is_empty(),
        "a call's events stay out of the chat stream"
    );
    assert_eq!(
        page.turns.iter().filter(|turn| turn.call.is_some()).count(),
        1
    );
    world.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn work_that_outlives_a_call_keeps_the_message_open() {
    let world = world().await;
    let work = Work::new(world.pool.clone());
    let route = cubebot_orchestrator::speech::Route {
        stt: cubebot_orchestrator::speech::Provider::Embedded,
        tts: cubebot_orchestrator::speech::Provider::Embedded,
        voice: cubebot_orchestrator::speech::Voice::Female,
    };
    let agent = AgentId(world.agent);
    let call = work.start_call(&world.admin, agent, route).await.unwrap();
    let turn = work
        .say_in_call(&world.admin, agent, call, "do the long thing")
        .await
        .unwrap();
    work.end_call(call).await.unwrap();
    let history = work.history(&world.admin, world.agent, 0).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, "running", "its turn is unsettled");
    assert_eq!(history[0].call.as_ref().unwrap().turns[0].id, turn);
    work.cancel(&world.admin, world.agent, turn).await.unwrap();
    let history = work.history(&world.admin, world.agent, 0).await.unwrap();
    assert_eq!(history[0].status, "complete");
    assert_eq!(
        history[0].call.as_ref().unwrap().turns[0].status,
        "cancelled"
    );
    world.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn restart_closes_open_calls_at_the_last_thing_said() {
    let world = world().await;
    let work = Work::new(world.pool.clone());
    let route = cubebot_orchestrator::speech::Route {
        stt: cubebot_orchestrator::speech::Provider::Embedded,
        tts: cubebot_orchestrator::speech::Provider::Embedded,
        voice: cubebot_orchestrator::speech::Voice::Female,
    };
    work.start_call(&world.admin, AgentId(world.agent), route)
        .await
        .unwrap();
    work.close_open_calls().await.unwrap();
    let history = work.history(&world.admin, world.agent, 0).await.unwrap();
    assert_eq!(history[0].status, "complete");
    assert_eq!(history[0].call.as_ref().unwrap().duration_seconds, Some(0));
    world.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_call_needs_the_right_to_send_and_the_consoles_origin() {
    let world = world().await;
    assert!(
        open(&world, &world.token, "https://elsewhere.test")
            .await
            .is_err(),
        "another site's page"
    );

    world
        .identity
        .create_account(
            &world.admin,
            "member",
            "member password",
            false,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    let member = world
        .identity
        .login("member", "member password")
        .await
        .unwrap();
    let member_id = world.identity.authenticate(&member.token).await.unwrap().id;
    assert!(
        open(&world, &member.token, ORIGIN).await.is_err(),
        "no grant"
    );
    grant(&world, member_id, Capability::ConversationRead).await;
    assert!(
        open(&world, &member.token, ORIGIN).await.is_err(),
        "reading is not sending"
    );
    grant(&world, member_id, Capability::ConversationSend).await;
    let mut call = open(&world, &member.token, ORIGIN).await.unwrap();
    assert_eq!(next_json(&mut call).await["kind"], "ready");
    call.close(None).await.unwrap();
    world.task.abort();
}

async fn grant(world: &World, member: Uuid, capability: Capability) {
    world
        .identity
        .grant(
            &world.admin,
            &GrantRequest {
                request_id: Uuid::new_v4(),
                recipient: Recipient::User { id: member },
                capability,
                scope: Scope::Conversation { id: world.agent },
            },
        )
        .await
        .unwrap();
}

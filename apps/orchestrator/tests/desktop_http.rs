//! The desktop WebSocket (ADRs 0039 and 0085) through the configured browser router, against a
//! stand-in sandbox whose host channel answers like the arbiter.
use crate::{runtime_identity::http_server, support};
use cubebot_orchestrator::{
    agents::{
        AgentId, Agents, CreateAgent,
        kubernetes::Error,
        sandbox::{Desktop, ExecChannel, Sandbox, Status},
    },
    identity::{
        Identity,
        access::{Capability, GrantRequest, Recipient, Scope},
    },
    work::{SendMessage, Work},
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use uuid::Uuid;

pub(crate) const ORIGIN: &str = "https://cubebot.test";

/// Its host channel keeps the arbiter's mode rules: one person holds control, and only they
/// change the mode while they hold it.
pub(crate) struct StandIn;

impl Sandbox for StandIn {
    async fn create(&self, _: AgentId) -> Result<(), Error> {
        Ok(())
    }
    async fn start(&self, _: AgentId) -> Result<(), Error> {
        Ok(())
    }
    async fn stop(&self, _: AgentId) -> Result<(), Error> {
        Ok(())
    }
    async fn exec_channel(&self, _: AgentId, command: &[&str]) -> Result<ExecChannel, Error> {
        assert_eq!(command, ["/usr/local/bin/cubebot-sandbox", "desktop"]);
        let (orchestrator, sandbox) = tokio::io::duplex(1 << 16);
        let (input, output) = tokio::io::split(orchestrator);
        tokio::spawn(arbiter(sandbox));
        Ok(ExecChannel {
            stdin: Box::new(output),
            stdout: Box::new(input),
            _session: Box::new(()),
        })
    }
    async fn workspace(&self, _: AgentId) -> Result<Option<String>, Error> {
        Ok(None)
    }
    async fn status(&self, _: AgentId) -> Result<Status, Error> {
        Ok(Status::Running)
    }
}

async fn arbiter(stream: tokio::io::DuplexStream) {
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    let (mut mode, mut holder) = ("agent".to_owned(), Value::Null);
    while let Ok(Some(line)) = lines.next_line().await {
        let request: Value = serde_json::from_str(&line).unwrap();
        let mut response = json!({});
        if request == "frame" {
            response["tiles"] = json!([{"x":0,"y":0,"png":"iVBORw0KGgo="}]);
        } else if let Some(change) = request.get("mode") {
            let account = change.get("account").cloned().unwrap_or(Value::Null);
            if mode == "human" && holder != account {
                response["error"] = json!("another person has control");
            } else {
                mode = change["mode"].as_str().unwrap().to_owned();
                holder = account;
            }
        }
        response["mode"] = json!(mode);
        let mut line = response.to_string().into_bytes();
        line.push(b'\n');
        if write.write_all(&line).await.is_err() {
            return;
        }
    }
}

pub(crate) type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub(crate) async fn open(
    server: &str,
    agent: Uuid,
    token: &str,
    origin: Option<&str>,
) -> Result<Socket, tokio_tungstenite::tungstenite::Error> {
    let url = format!(
        "{}/api/agents/{agent}/desktop",
        server.replacen("http", "ws", 1)
    );
    let mut request = url.into_client_request().unwrap();
    let headers = request.headers_mut();
    let cookie = format!("__Host-cubebot_session={token}");
    headers.insert("cookie", cookie.parse().unwrap());
    if let Some(origin) = origin {
        headers.insert("origin", origin.parse().unwrap());
    }
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(socket, _)| socket)
}

/// The answer to the message sent at `t`, skipping frames.
pub(crate) async fn ask(socket: &mut Socket, message: Value, t: f64) -> Value {
    let mut message = message;
    message["t"] = json!(t);
    socket
        .send(Message::Text(message.to_string().into()))
        .await
        .unwrap();
    loop {
        let Some(Ok(Message::Text(text))) = socket.next().await else {
            panic!("the desktop closed before answering");
        };
        let answer: Value = serde_json::from_str(&text).unwrap();
        if answer["ack"] == json!(t) {
            return answer;
        }
        assert!(answer.get("tiles").is_some(), "only frames come unasked");
    }
}

async fn grant(
    identity: &Identity,
    admin: &cubebot_orchestrator::identity::Actor,
    member: Uuid,
    agent: Uuid,
    capability: Capability,
) {
    let request = GrantRequest {
        request_id: Uuid::new_v4(),
        recipient: Recipient::User { id: member },
        scope: Scope::Agent { id: agent },
        capability,
    };
    identity.grant(admin, &request).await.unwrap();
}

async fn tool_call(pool: &sqlx::PgPool, turn: Uuid, sequence: i64) -> Uuid {
    let call = Uuid::new_v4();
    let events = [
        json!({"kind":"tool_call","id":call,"name":"computer","input":{"action":"screenshot"}}),
        json!({"kind":"tool_result","id":call,"outcome":{"status":"screenshot","output":"OK","image":"iVBORw0KGgo="}}),
    ];
    for (offset, payload) in events.into_iter().enumerate() {
        sqlx::query!(
            "
            INSERT INTO turn_events (turn_id,sequence,payload)
            VALUES ($1,$2,$3)
            ",
            turn,
            sequence + offset as i64,
            payload,
        )
        .execute(pool)
        .await
        .unwrap();
    }
    call
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_person_watches_takes_control_and_hands_back_with_each_change_recorded() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let admin_session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&admin_session.token).await.unwrap();
    identity
        .create_account(&admin, "member", "member password", false, Uuid::new_v4())
        .await
        .unwrap();
    let member = identity.login("member", "member password").await.unwrap();
    let member_id = identity.authenticate(&member.token).await.unwrap().id;
    let create = CreateAgent {
        request_id: Uuid::new_v4(),
        name: "Desk".into(),
        instructions: String::new(),
    };
    let agent = Agents::new(pool.clone())
        .create(&admin, &create)
        .await
        .unwrap()
        .id;
    let router = cubebot_orchestrator::api::configured_router_with_credentials(
        pool.clone(),
        Some(ORIGIN.into()),
        None,
        None,
    )
    .await
    .unwrap()
    .layer(axum::Extension(Desktop::new(Arc::new(StandIn))));
    let (server, task) = http_server(router).await;

    assert!(
        open(&server, agent, &member.token, Some(ORIGIN))
            .await
            .is_err(),
        "no grant"
    );
    grant(&identity, &admin, member_id, agent, Capability::BrowserView).await;
    assert!(
        open(
            &server,
            agent,
            &member.token,
            Some("https://elsewhere.test")
        )
        .await
        .is_err(),
        "a cookie from another site's page"
    );
    let mut desktop = open(&server, agent, &member.token, Some(ORIGIN))
        .await
        .unwrap();
    let Some(Ok(Message::Text(frame))) = desktop.next().await else {
        panic!("no first frame");
    };
    let frame: Value = serde_json::from_str(&frame).unwrap();
    assert_eq!(
        (frame["mode"].as_str(), frame["tiles"][0]["x"].as_u64()),
        (Some("watch"), Some(0))
    );
    let refused = ask(&mut desktop, json!({"kind":"take"}), 1.0).await;
    assert_eq!(
        refused["error"],
        "you do not have permission to control this desktop"
    );

    // A screenshot before the person takes control is not a re-observation.
    let work = Work::new(pool.clone());
    let message = SendMessage {
        request_id: Uuid::new_v4(),
        message: "Use the desktop".into(),
    };
    let turn = work.send(&admin, agent, &message).await.unwrap().id;
    let before = tool_call(&pool, turn, 1).await;
    grant(
        &identity,
        &admin,
        member_id,
        agent,
        Capability::BrowserControl,
    )
    .await;
    let taken = ask(&mut desktop, json!({"kind":"take"}), 2.0).await;
    assert_eq!(
        (taken["mode"].as_str(), taken.get("error")),
        (Some("human"), None)
    );
    let typed = ask(
        &mut desktop,
        json!({"kind":"input","action":{"action":"type","text":"hi"}}),
        3.0,
    )
    .await;
    assert_eq!(typed["mode"], "human");
    let back = ask(&mut desktop, json!({"kind":"hand_back"}), 4.0).await;
    assert_eq!(
        (back["mode"].as_str(), back.get("error")),
        (Some("watch"), None)
    );
    let after = tool_call(&pool, turn, 3).await;
    let later = tool_call(&pool, turn, 5).await;
    desktop.close(None).await.unwrap();

    let history = work.history(&admin, agent, 0).await.unwrap();
    let marked: Vec<_> = history[0]
        .terminal
        .iter()
        .map(|call| (call.id, call.reobserved))
        .collect();
    assert_eq!(marked, [(before, false), (after, true), (later, false)]);
    for _ in 0..50 {
        let (modes, ended): (Value, bool) = sqlx::query!(
            r#"
            SELECT jsonb_path_query_array(mode_changes,'$[*].mode') AS "modes!",ended_at IS NOT NULL AS "ended!"
            FROM desktop_sessions
            WHERE agent_id=$1 AND viewer_id=$2
            "#,
            agent as _,
            member_id,
        )
        .fetch_one(&pool)
        .await
        .map(|row| (row.modes, row.ended))
        .unwrap();
        if ended {
            assert_eq!(modes, json!(["human", "watch"]));
            task.abort();
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the closed desktop session was not ended");
}

//! The vault (ADR 0086) end to end: a person saves a login through the console, the real agent
//! binary lists and fills it and saves a secret of its own with a scripted model, and a
//! recording stand-in arbiter receives the fill. Neither secret is in the turn ledger, the
//! model's requests or the desktop records; only the arbiter's line holds what it types.
use crate::{
    memory_agent::{Model, Step, call, respond, text},
    runtime_identity::{http_server, native_chat_setup, spawn_native_text},
    runtime_support::fixture,
};
use axum::Router;
use cubebot_orchestrator::{
    agents::{
        AgentId,
        kubernetes::Error,
        sandbox::{Desktop, ExecChannel, Runtime, Sandbox, Status},
    },
    connections::Credentials,
    identity::Identity,
    work::Work,
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

const PASSWORD: &str = "correct horse 9!";
const MADE: &str = "agent-made S3cret";

/// Answers every host channel line as an arbiter in `agent` mode would, and keeps the lines.
#[derive(Clone, Default)]
struct Recording(Arc<Mutex<Vec<Value>>>);

impl Sandbox for Recording {
    async fn create(&self, _: AgentId) -> Result<(), Error> {
        Ok(())
    }
    async fn start(&self, _: AgentId) -> Result<(), Error> {
        Ok(())
    }
    async fn stop(&self, _: AgentId) -> Result<(), Error> {
        Ok(())
    }
    async fn exec_channel(&self, _: AgentId, _: &[&str]) -> Result<ExecChannel, Error> {
        let (orchestrator, sandbox) = tokio::io::duplex(1 << 16);
        let (input, output) = tokio::io::split(orchestrator);
        let lines = self.0.clone();
        tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(sandbox);
            let mut reader = BufReader::new(read).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                lines
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str(&line).unwrap());
                if write.write_all(b"{\"mode\":\"agent\"}\n").await.is_err() {
                    return;
                }
            }
        });
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

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_fill_types_the_secret_without_it_reaching_the_ledger_or_records() {
    let f = fixture().await;
    let (auth, work, admin, agent) = native_chat_setup(&f, "Log in to GitHub").await;
    let arbiter = Recording::default();
    let desktop = Desktop::new(Arc::new(arbiter.clone()), &Runtime::Kvm {});
    let console = cubebot_orchestrator::api::configured_router_with_credentials(
        f.pool.clone(),
        None,
        None,
        Some(Credentials::new([7; 32])),
    )
    .await
    .unwrap()
    .layer(axum::Extension(desktop.clone()));
    let (console, console_task) = http_server(console).await;
    let token = Identity::new(f.pool.clone())
        .login("admin", "administrator password")
        .await
        .unwrap()
        .token;
    let http = openidconnect::reqwest::Client::new();
    let save = |body: Value| {
        http.post(format!("{console}/api/vault"))
            .bearer_auth(&token)
            .json(&body)
            .send()
    };
    let untypable = json!({"kind":"login","name":"GitHub","site":"github.com","username":"octocat","secret":"pässword"});
    assert_eq!(save(untypable).await.unwrap().status(), 400);
    let saved = save(json!({"kind":"login","name":"GitHub","site":"github.com","username":"octocat","secret":PASSWORD}))
        .await
        .unwrap();
    assert_eq!(saved.status(), 200);
    let saved = saved.text().await.unwrap();
    assert!(!saved.contains(PASSWORD), "{saved}");
    let item: Uuid = serde_json::from_str::<Value>(&saved).unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let model = Arc::new(Model::default());
    let put = json!({"kind":"secret","name":"API key","site":null,"username":null,"secret":MADE});
    model.turn(
        vec![
            Step::Reply(json!([call("vault_list", json!({}))])),
            Step::Reply(json!([call(
                "vault_fill",
                json!({"item":item,"field":"secret"})
            )])),
            Step::Reply(json!([call("vault_put", put)])),
            Step::Reply(json!([text("Filled and saved.")])),
        ],
        json!([]),
    );
    let (provider, provider_task) = http_server(
        Router::new()
            .route("/responses", axum::routing::post(respond))
            .with_state(model.clone()),
    )
    .await;
    let runtime = cubebot_orchestrator::api::runtime_router(auth).layer(axum::Extension(desktop));
    let (origin, server) = http_server(runtime).await;
    let child = spawn_native_text(&origin, &provider);
    settled(&work, &admin, agent).await;

    let requests = model.requests();
    let listed = requests[1].to_string();
    assert!(listed.contains("octocat") && listed.contains(&item.to_string()));
    assert!(
        requests[2]
            .to_string()
            .contains("Typed at the keyboard focus")
    );
    assert!(
        requests[3]
            .to_string()
            .contains("Saved API key in your vault.")
    );
    for request in &requests {
        assert!(!request.to_string().contains(PASSWORD), "{request}");
    }
    let fills: Vec<Value> = arbiter.0.lock().unwrap().clone();
    assert_eq!(
        fills,
        [json!({"fill":{"item":item,"text":PASSWORD,"account":null}})]
    );

    let ledger: Vec<String> = sqlx::query_scalar!(
        r#"
        SELECT e.payload::text AS "payload!"
        FROM turn_events e
        JOIN chat_turns t ON t.id = e.turn_id
        WHERE t.conversation_id = $1
        "#,
        agent,
    )
    .fetch_all(&f.pool)
    .await
    .unwrap();
    let ledger = ledger.join("\n");
    assert!(ledger.contains("[redacted]"), "{ledger}");
    for secret in [PASSWORD, MADE] {
        assert!(!ledger.contains(secret), "{ledger}");
    }
    let records: Vec<String> = sqlx::query_scalar!(
        r#"
        SELECT to_jsonb(s)::text AS "record!"
        FROM desktop_sessions s
        "#
    )
    .fetch_all(&f.pool)
    .await
    .unwrap();
    assert!(!records.join("\n").contains(PASSWORD));

    // The agent's own item is listed for the console without its secret, and fills for a person.
    let listed = http
        .get(format!("{console}/api/agents/{agent}/vault"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        listed.contains("API key") && listed.contains("GitHub"),
        "{listed}"
    );
    assert!(
        !listed.contains(MADE) && !listed.contains(PASSWORD),
        "{listed}"
    );
    let filled = http
        .post(format!("{console}/api/agents/{agent}/vault/{item}/fill"))
        .bearer_auth(&token)
        .json(&json!({"field":"username"}))
        .send()
        .await
        .unwrap();
    assert_eq!(filled.status(), 204);
    let last = arbiter.0.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last["fill"]["text"], "octocat");
    assert!(last["fill"]["account"].is_string());

    drop(child);
    console_task.abort();
    server.abort();
    provider_task.abort();
    f.task.abort();
}

async fn settled(work: &Work, admin: &cubebot_orchestrator::identity::Actor, agent: Uuid) {
    for _ in 0..200 {
        let turns = work.history(admin, agent, 0).await.unwrap();
        if turns.first().is_some_and(|turn| turn.status == "complete") {
            assert_eq!(turns[0].response.as_deref(), Some("Filled and saved."));
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the turn did not complete");
}

//! The real agent binary against a real orchestrator and PostgreSQL, with a scripted model that
//! calls the memory tools across turns.
use crate::{
    runtime_identity::{http_server, native_chat_setup, runtime_server, spawn_native_text},
    runtime_support::fixture,
};
use axum::{Router, extract::State, response::IntoResponse};
use cubebot_orchestrator::{
    agents::{AgentId, Agents, CreateAgent},
    identity::Actor,
    knowledge::{CreateMemory, Knowledge, Memory, ReviseMemory},
    work::{SendMessage, Turn, Work},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Notify;
use uuid::Uuid;

const EXTRACTION: &str = "Before this turn ends";

pub(crate) enum Step {
    Reply(Value),
    /// Waits for `release` before replying; never released, it holds the turn open.
    Gated(Value),
    Reject,
}

#[derive(Default)]
pub(crate) struct Model {
    script: Mutex<(VecDeque<Step>, VecDeque<Value>, Vec<Value>)>,
    pub(crate) gated: Notify,
    pub(crate) release: Notify,
}

impl Model {
    pub(crate) fn turn(&self, steps: Vec<Step>, extracted: Value) {
        let mut script = self.script.lock().unwrap();
        script.0.extend(steps);
        script.1.push_back(extracted);
    }
    pub(crate) fn requests(&self) -> Vec<Value> {
        self.script.lock().unwrap().2.clone()
    }
}

pub(crate) async fn respond(
    State(model): State<Arc<Model>>,
    axum::Json(body): axum::Json<Value>,
) -> axum::response::Response {
    let last = body["input"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let step = {
        let mut script = model.script.lock().unwrap();
        script.2.push(body);
        if last.starts_with(EXTRACTION) {
            let facts = script.1.pop_front().unwrap_or(json!([]));
            Step::Reply(json!([text(&facts.to_string())]))
        } else {
            script
                .0
                .pop_front()
                .unwrap_or(Step::Reply(json!([text("UNSCRIPTED")])))
        }
    };
    let output = match step {
        Step::Reply(output) => output,
        Step::Gated(output) => {
            model.gated.notify_one();
            model.release.notified().await;
            output
        }
        Step::Reject => {
            return (axum::http::StatusCode::UNAUTHORIZED, "controlled rejection").into_response();
        }
    };
    let completed =
        json!({"type":"response.completed","response":{"status":"completed","output":output}});
    (
        [("content-type", "text/event-stream")],
        format!("data: {completed}\n\n"),
    )
        .into_response()
}

pub(crate) fn text(text: &str) -> Value {
    json!({"type":"message","content":[{"type":"output_text","text":text}]})
}
pub(crate) fn call(name: &str, arguments: Value) -> Value {
    json!({"type":"function_call","name":name,"call_id":Uuid::new_v4().to_string(),"arguments":arguments.to_string()})
}
fn remember(content: &str, memory: Option<Uuid>, revision: Option<i64>) -> Value {
    call(
        "remember",
        json!({"content":content,"memory":memory,"revision":revision}),
    )
}

struct Session {
    work: Work,
    admin: Actor,
    conversation: Uuid,
    knowledge: Knowledge,
    agent: AgentId,
    pool: sqlx::PgPool,
}

impl Session {
    async fn send(&self, message: &str) -> Uuid {
        let input = SendMessage {
            request_id: Uuid::new_v4(),
            message: message.into(),
        };
        self.work
            .send(&self.admin, self.conversation, &input)
            .await
            .unwrap()
            .id
    }
    async fn turn(&self, id: Uuid) -> Turn {
        let history = self
            .work
            .history(&self.admin, self.conversation, 0)
            .await
            .unwrap();
        history.into_iter().find(|turn| turn.id == id).unwrap()
    }
    /// Waits until the turn leaves `queued` and `running`.
    async fn settled(&self, id: Uuid) -> Turn {
        for _ in 0..200 {
            let turn = self.turn(id).await;
            if !matches!(turn.status.as_str(), "queued" | "running") {
                return turn;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("turn {id} did not settle");
    }
    async fn memories(&self) -> Vec<Memory> {
        self.knowledge
            .search(&self.admin, self.agent, None)
            .await
            .unwrap()
    }
    async fn events(&self, turn: Uuid) -> Vec<Value> {
        sqlx::query_scalar("SELECT payload FROM turn_events WHERE turn_id=$1 ORDER BY sequence")
            .bind(turn)
            .fetch_all(&self.pool)
            .await
            .unwrap()
    }
}

/// The tool output the model received for the search call in this request.
fn search_results(requests: &[Value]) -> Vec<Value> {
    let output = requests
        .iter()
        .flat_map(|request| request["input"].as_array().unwrap().clone())
        .rfind(|item| item["type"] == "function_call_output")
        .unwrap();
    let outcome: Value = serde_json::from_str(output["output"].as_str().unwrap()).unwrap();
    assert_eq!(outcome["code"], 0, "{outcome}");
    serde_json::from_str(outcome["output"].as_str().unwrap()).unwrap()
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_native_agent_saves_recalls_revises_and_forgets_across_turns() {
    let f = fixture().await;
    let first = "Our launch deadline is November 15. Please remember it.";
    let (auth, work, admin, conversation) = native_chat_setup(&f, first).await;
    let knowledge = Knowledge::new(f.pool.clone());
    let s = Session {
        work,
        admin,
        conversation,
        knowledge: knowledge.clone(),
        agent: f.agent,
        pool: f.pool.clone(),
    };
    // Another agent's memory shares the search words; it must never reach this agent.
    let other = Agents::new(f.pool.clone())
        .create(
            &s.admin,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: "Other".into(),
                instructions: String::new(),
            },
        )
        .await
        .unwrap();
    let foreign = CreateMemory {
        content: "The other team's launch deadline is March 3".into(),
    };
    knowledge
        .create(&s.admin, AgentId(other.id), &foreign)
        .await
        .unwrap();

    let model = Arc::new(Model::default());
    model.turn(
        vec![
            Step::Reply(json!([remember(
                "The launch deadline is November 15",
                None,
                None
            )])),
            Step::Gated(json!([text("Noted: November 15.")])),
        ],
        // The duplicate of the explicit save is not stored twice.
        json!([
            "The launch deadline is November 15",
            "The user wants reminders a week ahead"
        ]),
    );
    let (provider, provider_task) = http_server(
        Router::new()
            .route("/responses", axum::routing::post(respond))
            .with_state(model.clone()),
    )
    .await;
    let (origin, server) = runtime_server(auth).await;
    let child = spawn_native_text(&origin, &provider);

    // Staged, not committed, while the turn is still running.
    tokio::time::timeout(Duration::from_secs(20), model.gated.notified())
        .await
        .unwrap();
    let turn1 = s.work.history(&s.admin, s.conversation, 0).await.unwrap()[0].id;
    assert_eq!(s.turn(turn1).await.status, "running");
    assert!(s.memories().await.is_empty());
    model.release.notify_one();
    assert_eq!(s.settled(turn1).await.status, "complete");
    let saved = s.memories().await;
    assert_eq!(saved.len(), 2, "explicit save plus one new extracted fact");
    for memory in &saved {
        assert_eq!(
            (memory.revision, memory.source_turn, memory.edited_by_human),
            (1, Some(turn1), false)
        );
    }
    let deadline = saved
        .iter()
        .find(|memory| memory.content.contains("November 15"))
        .unwrap()
        .id;
    let requests = model.requests();
    let tools: Vec<&Value> = requests[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| &tool["name"])
        .collect();
    assert_eq!(tools[..3], ["search_memory", "remember", "forget"]);
    let extraction = requests.last().unwrap();
    assert!(
        extraction["input"].as_array().unwrap().last().unwrap()["content"]
            .as_str()
            .unwrap()
            .starts_with(EXTRACTION)
    );
    let done = s
        .events(turn1)
        .await
        .into_iter()
        .find(|event| event["kind"] == "done")
        .unwrap();
    assert_eq!(
        done["memories"].as_array().unwrap().len(),
        3,
        "the staged changes are kept as evidence"
    );

    // Recall searches only this agent's memories, with ids and revisions.
    model.turn(
        vec![
            Step::Reply(json!([call(
                "search_memory",
                json!({"query":"when is the launch deadline"})
            )])),
            Step::Reply(json!([text("The launch is due November 15.")])),
        ],
        json!([]),
    );
    let turn2 = s.send("When is the launch?").await;
    assert_eq!(s.settled(turn2).await.status, "complete");
    let found = search_results(&model.requests());
    assert_eq!(
        found,
        [json!({"id":deadline,"revision":1,"content":"The launch deadline is November 15"})]
    );
    assert_eq!(
        s.memories().await.len(),
        2,
        "a turn that saves nothing changes nothing"
    );

    // A person corrects the memory; the agent's write against the older revision loses.
    let correction = ReviseMemory {
        content: "The launch deadline is November 22".into(),
        expected_revision: 1,
    };
    knowledge
        .revise(&s.admin, s.agent, deadline, &correction)
        .await
        .unwrap();
    model.turn(
        vec![
            Step::Reply(json!([
                remember(
                    "The launch deadline is November 29",
                    Some(deadline),
                    Some(1)
                ),
                call("forget", json!({"memory":deadline,"revision":1})),
            ])),
            Step::Reply(json!([text("Updated.")])),
        ],
        json!([]),
    );
    let turn3 = s.send("Actually it moved to November 29.").await;
    assert_eq!(s.settled(turn3).await.status, "complete");
    let current = knowledge
        .search_for_agent(s.agent, "deadline")
        .await
        .unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(
        (current[0].revision, current[0].content.as_str()),
        (2, "The launch deadline is November 22")
    );
    assert!(current[0].edited_by_human);

    // With the current revision, the agent revises and then forgets.
    model.turn(
        vec![
            Step::Reply(json!([remember(
                "The launch deadline is November 29",
                Some(deadline),
                Some(2)
            )])),
            Step::Reply(json!([text("Updated.")])),
        ],
        json!([]),
    );
    let turn4 = s.send("It really is November 29 now.").await;
    assert_eq!(s.settled(turn4).await.status, "complete");
    let revised = knowledge
        .search_for_agent(s.agent, "deadline")
        .await
        .unwrap();
    assert_eq!(
        (revised[0].revision, revised[0].content.as_str()),
        (3, "The launch deadline is November 29")
    );
    let latest = s
        .memories()
        .await
        .into_iter()
        .find(|memory| memory.id == deadline)
        .unwrap();
    assert_eq!(
        (latest.source_turn, latest.edited_by_human),
        (Some(turn4), false)
    );
    model.turn(
        vec![
            Step::Reply(json!([call(
                "forget",
                json!({"memory":deadline,"revision":3})
            )])),
            Step::Reply(json!([text("Forgotten.")])),
        ],
        // A real model's extraction leaked the forgotten fact back as a note.
        json!(["The deadline was November 29 but has been forgotten"]),
    );
    let asked = model.requests().len();
    let turn5 = s
        .send("The launch is cancelled; forget the deadline.")
        .await;
    assert_eq!(s.settled(turn5).await.status, "complete");
    assert_eq!(
        model.requests().len(),
        asked + 2,
        "a forgetting turn skips extraction"
    );
    model.script.lock().unwrap().1.clear();
    assert!(
        knowledge
            .search_for_agent(s.agent, "deadline")
            .await
            .unwrap()
            .is_empty()
    );
    let history: Vec<(i64, bool, Option<Uuid>)> = sqlx::query_as("SELECT revision,deleted,turn_id FROM memory_revisions WHERE memory_id=$1 ORDER BY revision")
        .bind(deadline).fetch_all(&f.pool).await.unwrap();
    assert_eq!(
        history,
        [
            (1, false, Some(turn1)),
            (2, false, None),
            (3, false, Some(turn4)),
            (4, true, Some(turn5))
        ]
    );

    // A failed turn commits none of its staged changes.
    model.turn(
        vec![
            Step::Reply(json!([remember("Saved by a failed turn", None, None)])),
            Step::Reject,
        ],
        json!([]),
    );
    let turn6 = s.send("Remember something, then fail.").await;
    let failed = s.settled(turn6).await;
    assert_eq!(
        (failed.status.as_str(), failed.error.as_deref()),
        ("waiting", Some("provider_rejected"))
    );
    assert!(
        knowledge
            .search_for_agent(s.agent, "failed")
            .await
            .unwrap()
            .is_empty()
    );
    s.work
        .cancel(&s.admin, s.conversation, turn6)
        .await
        .unwrap();

    // Neither does a cancelled one, and it makes no extraction request.
    model.turn(
        vec![
            Step::Reply(json!([remember("Saved by a cancelled turn", None, None)])),
            Step::Gated(json!([text("Too late.")])),
        ],
        json!(["Extracted by a cancelled turn"]),
    );
    let turn7 = s.send("Remember something, then get cancelled.").await;
    tokio::time::timeout(Duration::from_secs(20), model.gated.notified())
        .await
        .unwrap();
    let before = model.requests().len();
    s.work
        .cancel(&s.admin, s.conversation, turn7)
        .await
        .unwrap();
    for _ in 0..100 {
        if s.events(turn7)
            .await
            .iter()
            .any(|event| event["kind"] == "error")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let error = s
        .events(turn7)
        .await
        .into_iter()
        .find(|event| event["kind"] == "error")
        .unwrap();
    assert_eq!(error["code"], "cancelled");
    assert_eq!(model.requests().len(), before);
    assert!(
        knowledge
            .search_for_agent(s.agent, "cancelled")
            .await
            .unwrap()
            .is_empty()
    );
    let left: Vec<String> = s
        .memories()
        .await
        .into_iter()
        .map(|memory| memory.content)
        .collect();
    assert_eq!(left, ["The user wants reminders a week ahead"]);
    let foreign = knowledge
        .search_for_agent(AgentId(other.id), "launch")
        .await
        .unwrap();
    assert_eq!(foreign.len(), 1, "the other agent's memory is untouched");

    // The runtime search endpoint refuses callers without an agent credential.
    let client = openidconnect::reqwest::Client::new();
    let url = format!("{origin}/api/runtime/memories?query=launch");
    assert_eq!(client.get(&url).send().await.unwrap().status(), 401);
    let human = cubebot_orchestrator::identity::Identity::new(f.pool.clone())
        .login("admin", "administrator password")
        .await
        .unwrap();
    let refused = client
        .get(&url)
        .bearer_auth(&human.token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        refused.status(),
        401,
        "a human session is not an agent credential"
    );

    drop(child);
    server.abort();
    provider_task.abort();
    f.task.abort();
}

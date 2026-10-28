use axum::{
    Json, Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::HeaderMap,
    response::Response,
    routing::{get, post},
};
use cubebot_agent::control::{Config, Error, RuntimeClient};
use serde_json::{Value, json};
use std::{
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

fn issued() -> Value {
    json!({"token":format!("cb_runtime_{}", "a".repeat(64)),"agent":Uuid::from_u128(1),"attempt":Uuid::from_u128(2),"epoch":1,"version":1})
}

/// A scripted orchestrator: the test reads every report and chooses every delivery.
#[derive(Clone)]
struct Channel {
    reports: mpsc::UnboundedSender<Value>,
    deliveries: Arc<Mutex<mpsc::UnboundedReceiver<Value>>>,
    provider: Arc<std::sync::Mutex<Vec<Value>>>,
}
struct Fake {
    origin: String,
    reports: mpsc::UnboundedReceiver<Value>,
    deliveries: mpsc::UnboundedSender<Value>,
    provider: Arc<std::sync::Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
async fn fake(replies: Vec<Value>) -> Fake {
    let (reports, received) = mpsc::unbounded_channel();
    let (deliveries, scripted) = mpsc::unbounded_channel();
    let provider = Arc::new(std::sync::Mutex::new(replies));
    let router = Router::new()
        .route(
            "/api/runtime/bootstrap",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["token"], "projected-token");
                Json(issued())
            }),
        )
        .route("/api/runtime/channel", get(upgrade))
        .route("/responses", post(model))
        .route("/messages", post(model))
        .route(
            "/models/{id}",
            get(|| async { Json(json!({"max_input_tokens":1000,"max_tokens":64})) }),
        )
        .with_state(Channel {
            reports,
            deliveries: Arc::new(Mutex::new(scripted)),
            provider: provider.clone(),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Fake {
        origin,
        reports: received,
        deliveries,
        provider,
        task,
    }
}
async fn upgrade(
    State(channel): State<Channel>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    assert_eq!(
        headers["authorization"],
        format!("Bearer cb_runtime_{}", "a".repeat(64))
    );
    upgrade.on_upgrade(move |socket| relay(channel, socket))
}
async fn relay(channel: Channel, mut socket: WebSocket) {
    let mut scripted = channel.deliveries.lock().await;
    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    channel.reports.send(serde_json::from_str(&text).unwrap()).unwrap();
                }
                _ => return,
            },
            delivery = scripted.recv() => match delivery {
                Some(Value::Null) | None => return,
                Some(delivery) => {
                    socket.send(Message::Text(delivery.to_string().into())).await.unwrap();
                }
            },
        }
    }
}
/// Each scripted reply is a list of SSE payloads, or `{"status":..,"retry_after":..}`.
async fn model(State(channel): State<Channel>, Json(body): Json<Value>) -> Response {
    use axum::response::IntoResponse;
    let mut replies = channel.provider.lock().unwrap();
    let reply = replies.remove(0);
    replies.push(body);
    if let Some(status) = reply["status"].as_u64() {
        let status = axum::http::StatusCode::from_u16(status as u16).unwrap();
        let retry = reply["retry_after"].as_str().unwrap().to_owned();
        return (status, [("retry-after", retry)], "controlled failure").into_response();
    }
    let body: String = reply
        .as_array()
        .unwrap()
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect();
    ([("content-type", "text/event-stream")], body).into_response()
}
/// An OpenAI stream: one delta per message text, then the completed response.
fn responses(output: Value, usage: Value) -> Value {
    let mut events: Vec<Value> = output
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "message")
        .map(|item| json!({"type":"response.output_text.delta","delta":item["content"][0]["text"]}))
        .collect();
    events.push(json!({"type":"response.completed","response":{"status":"completed","output":output,"usage":usage}}));
    events.into()
}
/// An Anthropic stream of one text block.
fn messages(text: &str, input_tokens: u64, output_tokens: u64) -> Value {
    json!([
        {"type":"message_start","message":{"usage":{"input_tokens":input_tokens,"output_tokens":1}}},
        {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},
        {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}},
        {"type":"content_block_stop","index":0},
        {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":output_tokens}},
        {"type":"message_stop"}
    ])
}

impl Fake {
    async fn expect(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(15), self.reports.recv())
            .await
            .expect("agent report")
            .unwrap()
    }
    fn deliver(&self, delivery: Value) {
        self.deliveries.send(delivery).unwrap();
    }
    async fn event(&mut self, sequence: u64) -> Value {
        let report = self.expect().await;
        assert_eq!(report["type"], "event", "{report}");
        assert_eq!(report["sequence"], sequence, "{report}");
        report["event"].clone()
    }
    fn ack(&self, turn: Uuid, sequences: impl IntoIterator<Item = u64>) {
        for sequence in sequences {
            self.deliver(json!({"type":"ack","turn":turn,"sequence":sequence}));
        }
    }
    async fn drain(&mut self, child: &mut std::process::Child) {
        let request = Uuid::new_v4();
        self.deliver(json!({"type":"drain","request_id":request}));
        assert_eq!(
            self.expect().await,
            json!({"type":"drained","request_id":request})
        );
        self.deliver(json!({"type":"revoked"}));
        assert!(child_exit(child).await.success());
        self.task.abort();
    }
}

fn root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("cubebot-native-{}", Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    std::fs::write(root.join("token"), "projected-token").unwrap();
    std::fs::create_dir(root.join("workspace")).unwrap();
    root
}
fn spawn(origin: &str, root: &std::path::Path) -> std::process::Child {
    // The test's own user stands in for the tool user, as on the development backend.
    let owner = std::fs::metadata(root).unwrap();
    std::process::Command::new(env!("CARGO_BIN_EXE_cubebot-agent"))
        .env("CUBEBOT_ORCHESTRATOR_ORIGIN", origin)
        .env("CUBEBOT_BOOTSTRAP_TOKEN_FILE", root.join("token"))
        .env("CUBEBOT_JOURNAL_DIRECTORY", root)
        .env("CUBEBOT_WORKSPACE_DIRECTORY", root.join("workspace"))
        .env("CUBEBOT_TEST_PROVIDER_ORIGIN", format!("{origin}/"))
        .env("CUBEBOT_TERMINAL_ENABLED", "1")
        .env(
            "CUBEBOT_TOOL_USER",
            format!("{}:{}", owner.uid(), owner.gid()),
        )
        .spawn()
        .unwrap()
}
async fn child_exit(child: &mut std::process::Child) -> std::process::ExitStatus {
    for _ in 0..500 {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    child.kill().unwrap();
    child.wait().unwrap();
    panic!("native runtime did not exit");
}
fn turn(id: Uuid, sequence: u64, message: &str) -> Value {
    json!({"type":"turn","turn":id,"sequence":sequence,"message":message,"instructions":"Be brief.",
        "provider":"openai","model":"test-model","api_key":"test-key","context_window":100000,
        "history":[{"message":"Earlier","response":"Before"}],"prior":[]})
}

#[tokio::test]
async fn native_turn_streams_events_and_resends_unacknowledged_ones_after_reconnect() {
    let mut fake = fake(vec![responses(
        json!([{"type":"message","content":[{"type":"output_text","text":"Hello back"}]}]),
        json!({"input_tokens":9,"output_tokens":2}),
    )])
    .await;
    let root = root();
    let mut child = spawn(&fake.origin, &root);
    assert_eq!(fake.expect().await, json!({"type":"pull"}));
    let id = Uuid::new_v4();
    fake.deliver(turn(id, 3, "Hello"));
    assert_eq!(
        fake.event(3).await,
        json!({"kind":"text_delta","text":"Hello back"})
    );
    assert_eq!(
        fake.event(4).await,
        json!({"kind":"usage","input_tokens":9,"output_tokens":2})
    );
    assert_eq!(
        fake.event(5).await,
        json!({"kind":"done","response":"Hello back"})
    );
    assert_eq!(fake.expect().await, json!({"type":"pull"}));
    let request = fake.provider.lock().unwrap()[0].clone();
    assert_eq!(request["model"], "test-model");
    assert_eq!(request["stream"], true);
    assert!(request.get("max_output_tokens").is_none());
    assert_eq!(
        request["input"],
        json!([{"role":"system","content":"Be brief."},{"role":"user","content":"Earlier"},
            {"role":"assistant","content":"Before"},{"role":"user","content":"Hello"}])
    );
    // Only the first event reaches the ledger before the connection drops.
    fake.ack(id, [3]);
    fake.deliver(Value::Null);
    fake.event(4).await;
    fake.event(5).await;
    assert_eq!(fake.expect().await, json!({"type":"pull"}));
    fake.ack(id, [4, 5]);
    fake.drain(&mut child).await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn crash_recovery_reports_unfinished_tool_effects_once_and_cancel_stops_parallel_calls() {
    let root = root();
    let earlier = Uuid::new_v4();
    let call = Uuid::new_v4();
    let mut journal = cubebot_agent::journal::Journal::open(&root.join("tools")).unwrap();
    journal
        .append(
            json!({"turn":earlier,"sequence":7,"event":{"kind":"tool_call","id":call,"name":"bash","input":{"command":"touch effect"}}})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    drop(journal);
    let mut fake = fake(vec![responses(
        json!([{"type":"function_call","name":"bash","call_id":"c1","arguments":"{\"command\":\"sleep 30\"}"},
            {"type":"function_call","name":"edit_file","call_id":"c2","arguments":"{\"path\":\"notes.txt\",\"old_string\":\"world\",\"new_string\":\"there\",\"replace_all\":false}"}]),
        json!({"input_tokens":9,"output_tokens":2}),
    )])
    .await;
    std::fs::write(root.join("workspace/notes.txt"), "hello world").unwrap();
    let mut child = spawn(&fake.origin, &root);
    assert_eq!(fake.event(7).await["kind"], "tool_call");
    assert_eq!(
        fake.event(8).await,
        json!({"kind":"tool_result","id":call,"outcome":{"status":"uncertain"}})
    );
    assert_eq!(fake.expect().await, json!({"type":"pull"}));
    fake.ack(earlier, [7, 8]);

    let id = Uuid::new_v4();
    fake.deliver(turn(id, 1, "Wait"));
    assert_eq!(fake.event(1).await["kind"], "usage");
    let started = fake.event(2).await;
    assert_eq!(started["kind"], "tool_call");
    assert_eq!(started["input"], json!({"command":"sleep 30"}));
    // Both calls are journaled before either runs; the edit finishes while the command runs.
    let edit = fake.event(3).await;
    assert_eq!(edit["name"], "edit_file");
    let edited = fake.event(4).await;
    assert_eq!(edited["id"], edit["id"]);
    assert_eq!(edited["outcome"]["code"], 0, "{edited}");
    assert_eq!(
        std::fs::read_to_string(root.join("workspace/notes.txt")).unwrap(),
        "hello there"
    );
    fake.deliver(json!({"type":"cancel","turn":id}));
    let stopped = fake.event(5).await;
    assert_eq!(stopped["id"], started["id"]);
    assert_eq!(stopped["outcome"]["status"], "stopped");
    assert_eq!(stopped["outcome"]["reason"], "cancelled");
    assert_eq!(
        fake.event(6).await,
        json!({"kind":"error","code":"cancelled"})
    );
    assert_eq!(fake.expect().await, json!({"type":"pull"}));
    fake.ack(id, 1..=6);
    fake.drain(&mut child).await;
    let journal = cubebot_agent::journal::Journal::open(&root.join("tools")).unwrap();
    assert!(journal.pending().is_empty());
    drop(journal);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn a_malformed_call_answers_the_model_while_the_other_calls_run() {
    let mut fake = fake(vec![
        responses(
            json!([{"type":"function_call","name":"nope","call_id":"c1","arguments":"{}"},
                {"type":"function_call","name":"bash","call_id":"c2","arguments":"{\"command\":\"printf hi\"}"},
                {"type":"function_call","name":"bash","call_id":"c3","arguments":"not json"}]),
            json!({"input_tokens":9,"output_tokens":2}),
        ),
        responses(
            json!([{"type":"message","content":[{"type":"output_text","text":"Fixed"}]}]),
            json!({"input_tokens":20,"output_tokens":1}),
        ),
    ])
    .await;
    let root = root();
    let mut child = spawn(&fake.origin, &root);
    assert_eq!(fake.expect().await, json!({"type":"pull"}));
    let id = Uuid::new_v4();
    fake.deliver(turn(id, 1, "Go"));
    assert_eq!(fake.event(1).await["kind"], "usage");
    let unknown = fake.event(2).await;
    assert_eq!(
        (&unknown["name"], &unknown["input"]),
        (&json!("nope"), &json!({}))
    );
    let ran = fake.event(3).await;
    let garbled = fake.event(4).await;
    assert_eq!(garbled["input"], "not json");
    let mut results = [
        fake.event(5).await,
        fake.event(6).await,
        fake.event(7).await,
    ];
    let order = |result: &Value| {
        [&unknown, &ran, &garbled]
            .iter()
            .position(|call| call["id"] == result["id"])
    };
    results.sort_by_key(order);
    assert_eq!(
        results[0]["outcome"],
        json!({"status":"exited","code":null,"output":"The call was not run: no tool has this name."})
    );
    assert_eq!(results[1]["outcome"]["code"], 0);
    assert_eq!(
        results[2]["outcome"]["output"],
        "The call was not run: arguments are not valid JSON."
    );
    assert_eq!(fake.event(8).await["kind"], "text_delta");
    assert_eq!(fake.event(9).await["kind"], "usage");
    assert_eq!(
        fake.event(10).await,
        json!({"kind":"done","response":"Fixed"})
    );
    assert_eq!(fake.expect().await, json!({"type":"pull"}));
    // Both results go back to the model, in call order.
    let request = fake.provider.lock().unwrap()[1].clone();
    let outputs: Vec<&Value> = request["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "function_call_output")
        .collect();
    let ids: Vec<&Value> = outputs.iter().map(|output| &output["call_id"]).collect();
    assert_eq!(ids, [&json!("c1"), &json!("c2"), &json!("c3")]);
    assert!(
        outputs[0]["output"]
            .as_str()
            .unwrap()
            .contains("no tool has this name")
    );
    fake.ack(id, 1..=10);
    fake.drain(&mut child).await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn anthropic_turn_discovers_limits_retries_a_rate_limit_and_compacts_at_eighty_percent() {
    let mut fake = fake(vec![
        json!({"status":429,"retry_after":"0"}),
        messages("Hello back", 790, 20),
        messages("The gist", 800, 5),
    ])
    .await;
    let root = root();
    let mut child = spawn(&fake.origin, &root);
    assert_eq!(fake.expect().await, json!({"type":"pull"}));
    let id = Uuid::new_v4();
    let mut work = turn(id, 1, "Hello");
    work["provider"] = json!("anthropic");
    work["context_window"] = Value::Null;
    fake.deliver(work);
    assert_eq!(
        fake.event(1).await,
        json!({"kind":"text_delta","text":"Hello back"})
    );
    assert_eq!(
        fake.event(2).await,
        json!({"kind":"usage","input_tokens":790,"output_tokens":20})
    );
    // 810 of the discovered 1000-token window: summarize before the reply settles the turn.
    assert_eq!(fake.event(3).await["kind"], "usage");
    assert_eq!(
        fake.event(4).await,
        json!({"kind":"summary","text":"The gist"})
    );
    assert_eq!(
        fake.event(5).await,
        json!({"kind":"done","response":"Hello back"})
    );
    assert_eq!(fake.expect().await, json!({"type":"pull"}));
    let requests = fake.provider.lock().unwrap().clone();
    // The rate-limited request was resent unchanged to the same model.
    assert_eq!(requests[0], requests[1]);
    assert_eq!(requests[0]["model"], "test-model");
    assert_eq!(requests[0]["max_tokens"], 64);
    assert_eq!(requests[0]["tool_choice"]["type"], "auto");
    let summary = &requests[2];
    assert_eq!(summary["model"], "test-model");
    assert_eq!(summary["tool_choice"], json!({"type":"none"}));
    let sent = summary["messages"].as_array().unwrap();
    assert_eq!(
        sent[sent.len() - 2]["content"],
        json!([{"type":"text","text":"Hello back"}])
    );
    assert!(
        sent.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .starts_with("Write a structured summary")
    );
    fake.ack(id, 1..=5);
    fake.drain(&mut child).await;
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_nonconfidential_remote_origins_and_access_bearing_urls() {
    for origin in [
        "http://example.test",
        "https://user:password@example.test",
        "https://example.test/path",
        "https://example.test?token=secret",
        "https://example.test/#fragment",
    ] {
        assert!(Config::new(origin, "/token".into()).is_err());
    }
    assert!(Config::new("https://example.test", "/token".into()).is_ok());
    assert!(Config::new("http://127.0.0.1:1234", "/token".into()).is_ok());
}

#[derive(Clone, Copy)]
enum BadReply {
    Oversized,
    Redirect,
}
async fn bad_bootstrap(State(reply): State<BadReply>) -> axum::response::Response {
    use axum::response::IntoResponse;
    match reply {
        BadReply::Oversized => "x".repeat(20 * 1024 + 1).into_response(),
        BadReply::Redirect => (
            axum::http::StatusCode::TEMPORARY_REDIRECT,
            [("location", "/leak")],
        )
            .into_response(),
    }
}
#[tokio::test]
async fn bootstrap_rejects_oversized_and_redirected_responses() {
    for case in [BadReply::Oversized, BadReply::Redirect] {
        let router = Router::new()
            .route("/api/runtime/bootstrap", post(bad_bootstrap))
            .route("/leak", axum::routing::any(|| async { Json(issued()) }))
            .with_state(case);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let path = std::env::temp_dir().join(format!("cubebot-bad-control-{}", Uuid::new_v4()));
        std::fs::write(&path, "projected").unwrap();
        let mut client = RuntimeClient::new(Config::new(&origin, path.clone()).unwrap()).unwrap();
        let result = client.bootstrap().await;
        assert!(matches!(
            (case, result),
            (BadReply::Oversized, Err(Error::Protocol))
                | (BadReply::Redirect, Err(Error::Unavailable))
        ));
        assert!(matches!(client.connect().await, Err(Error::Credential)));
        task.abort();
        std::fs::remove_file(path).unwrap();
    }
}

use axum::{
    Json, Router,
    body::Body,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use cubebot_providers::{Event, Message, OpenAi, Provider, Request, Tool, Usage};
use futures_util::StreamExt;
use serde_json::{Value, json};

/// Streams events in seven-byte chunks so frames are split across reads.
fn sse(events: &[Value]) -> Response {
    let text: String = events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect();
    let chunks: Vec<Result<Vec<u8>, std::io::Error>> = text
        .into_bytes()
        .chunks(7)
        .map(|chunk| Ok(chunk.to_vec()))
        .collect();
    (
        [("content-type", "text/event-stream")],
        Body::from_stream(futures_util::stream::iter(chunks)),
    )
        .into_response()
}

async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (origin, task)
}

fn request<'a>(messages: &'a [Message], tools: &'a [Tool]) -> Request<'a> {
    Request {
        model: "test-model",
        key: "controlled-key",
        system: "Be brief.",
        messages,
        tools,
        tools_disabled: false,
        max_output_tokens: None,
    }
}

async fn collect(provider: &OpenAi, request: &Request<'_>) -> Vec<Event> {
    provider
        .stream(request)
        .map(|event| event.unwrap())
        .collect()
        .await
}

async fn reply(headers: HeaderMap, Json(body): Json<Value>) -> Response {
    assert_eq!(headers["authorization"], "Bearer controlled-key");
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert_eq!(
        body["input"][0],
        json!({"role":"system","content":"Be brief."})
    );
    assert_eq!(body["input"][1]["content"], "Hello");
    assert!(body.get("max_output_tokens").is_none());
    assert!(body.get("tools").is_none());
    sse(&[
        json!({"type":"response.created"}),
        json!({"type":"response.output_text.delta","delta":"Hello "}),
        json!({"type":"response.output_text.delta","delta":"back"}),
        json!({"type":"response.completed","response":{"status":"completed",
            "output":[{"type":"message","content":[{"type":"output_text","text":"Hello back"}]}],
            "usage":{"input_tokens":5,"output_tokens":2}}}),
    ])
}

#[tokio::test]
async fn streams_text_deltas_then_the_completed_reply_without_an_output_cap() {
    let (origin, task) = serve(Router::new().route("/responses", post(reply))).await;
    let provider = OpenAi::for_loopback_tests(&origin).unwrap();
    let messages = [Message::User("Hello".into())];
    let events = collect(&provider, &request(&messages, &[])).await;
    let [
        Event::Text(first),
        Event::Text(second),
        Event::Done { reply, usage },
    ] = &events[..]
    else {
        panic!("unexpected events");
    };
    assert_eq!(format!("{first}{second}"), "Hello back");
    assert_eq!(reply.text, "Hello back");
    assert!(reply.calls.is_empty());
    assert_eq!(
        *usage,
        Some(Usage {
            input_tokens: 5,
            output_tokens: 2
        })
    );
    assert!(OpenAi::for_loopback_tests("https://elsewhere.example/").is_err());
    task.abort();
}

#[tokio::test]
async fn provider_failure_does_not_expose_remote_payloads_and_carries_retry_after() {
    let router = Router::new().route(
        "/responses",
        post(|Json(body): Json<Value>| async move {
            if body["model"] == "limited" {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    [("retry-after", "3")],
                    "slow down",
                )
                    .into_response()
            } else {
                (StatusCode::UNAUTHORIZED, "sensitive upstream diagnostics").into_response()
            }
        }),
    );
    let (origin, task) = serve(router).await;
    let provider = OpenAi::for_loopback_tests(&origin).unwrap();
    let mut rejected = request(&[], &[]);
    rejected.model = "rejected";
    let error = std::pin::pin!(provider.stream(&rejected))
        .next()
        .await
        .unwrap()
        .err()
        .unwrap();
    assert_eq!(error.code(), "provider_rejected");
    assert!(!error.retryable());
    assert!(!format!("{error:?}").contains("sensitive"));
    let mut limited = request(&[], &[]);
    limited.model = "limited";
    let error = std::pin::pin!(provider.stream(&limited))
        .next()
        .await
        .unwrap()
        .err()
        .unwrap();
    assert!(error.retryable());
    assert_eq!(error.retry_after(), Some(std::time::Duration::from_secs(3)));
    task.abort();
}

#[tokio::test]
async fn a_stream_that_ends_before_completion_is_never_a_partial_reply() {
    let router = Router::new().route(
        "/responses",
        post(|| async { sse(&[json!({"type":"response.output_text.delta","delta":"Hal"})]) }),
    );
    let (origin, task) = serve(router).await;
    let provider = OpenAi::for_loopback_tests(&origin).unwrap();
    let events: Vec<_> = provider.stream(&request(&[], &[])).collect().await;
    assert!(matches!(events[0], Ok(Event::Text(_))));
    assert_eq!(
        events[1].as_ref().err().unwrap().code(),
        "provider_unavailable"
    );
    assert_eq!(events.len(), 2);
    task.abort();
}

#[tokio::test]
async fn parallel_calls_arrive_together_and_replay_native_output_with_their_result() {
    async fn tool_reply(Json(body): Json<Value>) -> Response {
        assert_eq!(body["tools"][0]["name"], "terminal");
        assert_eq!(body["tools"][0]["strict"], true);
        assert_eq!(body["tools"][1]["strict"], false);
        assert!(body.get("parallel_tool_calls").is_none());
        // The earlier reasoning and call are replayed verbatim before their output.
        assert_eq!(body["input"][2]["type"], "reasoning");
        assert_eq!(body["input"][3]["call_id"], "call_0");
        assert_eq!(
            body["input"][4],
            json!({"type":"function_call_output","call_id":"call_0","output":"done"})
        );
        sse(&[
            json!({"type":"response.completed","response":{"status":"completed","output":[
            {"type":"function_call","name":"terminal","call_id":"call_1","arguments":"{\"command\":\"printf hello\"}"},
            {"type":"function_call","name":"terminal","call_id":"call_2","arguments":"{\"command\":\"true\"}"}]}}),
        ])
    }
    let (origin, task) = serve(Router::new().route("/responses", post(tool_reply))).await;
    let provider = OpenAi::for_loopback_tests(&origin).unwrap();
    let earlier = cubebot_providers::Reply {
        text: String::new(),
        calls: Vec::new(),
        native: json!([{"type":"reasoning","encrypted_content":"opaque"},
            {"type":"function_call","name":"terminal","call_id":"call_0","arguments":"{}"}]),
    };
    let messages = [
        Message::User("Run it".into()),
        Message::Assistant(earlier),
        Message::ToolResult {
            call_id: "call_0".into(),
            output: "done".into(),
        },
    ];
    let tools = [
        Tool {
            name: "terminal".into(),
            description: "Run".into(),
            parameters: json!({"type":"object"}),
            strict: true,
        },
        Tool {
            name: "external".into(),
            description: "Anything".into(),
            parameters: json!({"type":"object"}),
            strict: false,
        },
    ];
    let events = collect(&provider, &request(&messages, &tools)).await;
    let [Event::Done { reply, usage: None }] = &events[..] else {
        panic!("missing usage must stay unknown");
    };
    let [call, second] = &reply.calls[..] else {
        panic!("expected two calls");
    };
    assert_eq!(second.call_id, "call_2");
    assert_eq!(call.call_id, "call_1");
    assert_eq!(call.name, "terminal");
    assert!(call.arguments.contains("printf hello"));
    task.abort();
}

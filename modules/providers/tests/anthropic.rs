use axum::{
    Json, Router,
    body::Body,
    extract::Path,
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cubebot_providers::{Anthropic, Event, Limits, Message, Provider, Request, Tool, Usage};
use futures_util::StreamExt;
use serde_json::{Value, json};

fn sse(events: &[Value]) -> Response {
    let text: String = events
        .iter()
        .map(|event| {
            format!(
                "event: {}\r\ndata: {event}\r\n\r\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect();
    let chunks: Vec<Result<Vec<u8>, std::io::Error>> = text
        .into_bytes()
        .chunks(5)
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

fn delta(index: u64, delta: Value) -> Value {
    json!({"type":"content_block_delta","index":index,"delta":delta})
}

async fn messages(headers: HeaderMap, Json(body): Json<Value>) -> Response {
    assert_eq!(headers["x-api-key"], "controlled-key");
    assert_eq!(headers["anthropic-version"], "2026-10-21");
    assert_eq!(body["stream"], true);
    assert_eq!(body["max_tokens"], 128000);
    assert_eq!(body["cache_control"], json!({"type":"ephemeral"}));
    assert_eq!(body["system"], "Be brief.");
    assert_eq!(body["tools"][0]["input_schema"], json!({"type":"object"}));
    assert_eq!(
        body["tool_choice"],
        json!({"type":"auto","disable_parallel_tool_use":true})
    );
    assert_eq!(
        body["messages"][0],
        json!({"role":"user","content":"Hello"})
    );
    assert_eq!(
        body["messages"][1],
        json!({"role":"assistant","content":"Before"})
    );
    sse(&[
        json!({"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":90,"output_tokens":1}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
        delta(0, json!({"type":"thinking_delta","thinking":"plan"})),
        delta(0, json!({"type":"signature_delta","signature":"sig"})),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"ping"}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
        delta(1, json!({"type":"text_delta","text":"Running "})),
        delta(1, json!({"type":"text_delta","text":"it."})),
        json!({"type":"content_block_stop","index":1}),
        json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_1","name":"terminal","input":{}}}),
        delta(
            2,
            json!({"type":"input_json_delta","partial_json":"{\"command\":"}),
        ),
        delta(
            2,
            json!({"type":"input_json_delta","partial_json":"\"true\"}"}),
        ),
        json!({"type":"content_block_stop","index":2}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":20}}),
        json!({"type":"message_stop"}),
    ])
}

#[tokio::test]
async fn streams_text_and_assembles_a_replayable_tool_call_with_signed_thinking() {
    let router = Router::new().route("/messages", post(messages)).route(
        "/models/{id}",
        get(|Path(id): Path<String>| async move {
            assert_eq!(id, "test/model");
            Json(json!({"id":id,"max_input_tokens":1000000,"max_tokens":128000}))
        }),
    );
    let (origin, task) = serve(router).await;
    let provider = Anthropic::for_loopback_tests(&origin).unwrap();
    assert!(provider.capabilities().model_limits);
    assert_eq!(
        provider
            .limits("test/model", "controlled-key")
            .await
            .unwrap(),
        Limits {
            context_window: Some(1_000_000),
            max_output_tokens: Some(128_000)
        }
    );
    let history = [
        Message::User("Hello".into()),
        Message::Assistant(cubebot_providers::Reply {
            text: "Before".into(),
            call: None,
            native: Value::Null,
        }),
    ];
    let tools = [Tool {
        name: "terminal".into(),
        description: "Run".into(),
        parameters: json!({"type":"object"}),
    }];
    let request = Request {
        model: "test-model",
        key: "controlled-key",
        system: "Be brief.",
        messages: &history,
        tools: &tools,
        tools_disabled: false,
        max_output_tokens: Some(128_000),
    };
    let events: Vec<Event> = provider
        .stream(&request)
        .map(Result::unwrap)
        .collect()
        .await;
    let [
        Event::Text(first),
        Event::Text(second),
        Event::Done { reply, usage },
    ] = &events[..]
    else {
        panic!("unexpected events");
    };
    assert_eq!(format!("{first}{second}"), "Running it.");
    assert_eq!(reply.text, "Running it.");
    assert_eq!(
        *usage,
        Some(Usage {
            input_tokens: 100,
            output_tokens: 20
        })
    );
    let call = reply.call.as_ref().unwrap();
    assert_eq!(
        (call.call_id.as_str(), call.name.as_str()),
        ("toolu_1", "terminal")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&call.arguments).unwrap(),
        json!({"command":"true"})
    );
    assert_eq!(
        reply.native[0],
        json!({"type":"thinking","thinking":"plan","signature":"sig"})
    );
    assert_eq!(reply.native[2]["input"], json!({"command":"true"}));
    task.abort();
}

#[tokio::test]
async fn truncated_and_overloaded_streams_are_errors_not_replies() {
    let router = Router::new().route(
        "/messages",
        post(|Json(body): Json<Value>| async move {
            if body["model"] == "overloaded" {
                return sse(&[json!({"type":"error","error":{"type":"overloaded_error"}})]);
            }
            sse(&[
                json!({"type":"message_start","message":{"usage":{"input_tokens":1}}}),
                json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
                delta(0, json!({"type":"text_delta","text":"cut"})),
                json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":1}}),
                json!({"type":"message_stop"}),
            ])
        }),
    );
    let (origin, task) = serve(router).await;
    let provider = Anthropic::for_loopback_tests(&origin).unwrap();
    let mut request = Request {
        model: "truncated",
        key: "k",
        system: "",
        messages: &[],
        tools: &[],
        tools_disabled: false,
        max_output_tokens: Some(1),
    };
    let events: Vec<_> = provider.stream(&request).collect().await;
    assert_eq!(
        events.last().unwrap().as_ref().err().unwrap().code(),
        "provider_invalid_response"
    );
    request.model = "overloaded";
    let events: Vec<_> = provider.stream(&request).collect().await;
    assert!(events[0].as_ref().err().unwrap().retryable());
    request.max_output_tokens = None;
    let events: Vec<_> = provider.stream(&request).collect().await;
    assert_eq!(
        events[0].as_ref().err().unwrap().code(),
        "provider_unavailable"
    );
    task.abort();
}

use axum::extract::RawQuery;
use axum::{
    Json, Router,
    body::Body,
    extract::Path,
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cubebot_providers::{Anthropic, Event, Limits, Message, Model, Provider, Request, Tool, Usage};
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
    assert_eq!(body["tool_choice"], json!({"type":"auto"}));
    // Parallel results share the user message that answers their assistant message.
    assert_eq!(
        body["messages"][3]["content"],
        json!([{"type":"tool_result","tool_use_id":"toolu_a","content":"one"},
            {"type":"tool_result","tool_use_id":"toolu_b","content":"two"}])
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
            calls: Vec::new(),
            native: Value::Null,
        }),
        Message::Assistant(cubebot_providers::Reply {
            text: String::new(),
            calls: Vec::new(),
            native: json!([{"type":"tool_use","id":"toolu_a"},{"type":"tool_use","id":"toolu_b"}]),
        }),
        Message::ToolResult {
            call_id: "toolu_a".into(),
            output: "one".into(),
            image: None,
        },
        Message::ToolResult {
            call_id: "toolu_b".into(),
            output: "two".into(),
            image: None,
        },
    ];
    let tools = [Tool {
        name: "terminal".into(),
        description: "Run".into(),
        parameters: json!({"type":"object"}),
        strict: true,
        computer: false,
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
            output_tokens: 20,
            cached_input_tokens: 90,
            cost_usd: None,
        })
    );
    let [call] = &reply.calls[..] else {
        panic!("expected one call");
    };
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

#[tokio::test]
async fn computer_toolset_calls_become_computer_calls_and_echo_their_toolset() {
    async fn computer(Json(body): Json<Value>) -> Response {
        let tool = &body["tools"][0];
        assert_eq!(tool["type"], "computer_toolset_20260801");
        assert_eq!(tool["configs"]["zoom"], json!({"enabled":false}));
        let result = &body["messages"][2]["content"][0];
        assert_eq!(result["toolset_name"], "computer");
        assert_eq!(result["content"][1]["source"]["data"], "iVBORw0KGgo=");
        sse(&[
            json!({"type":"message_start","message":{"usage":{"input_tokens":10}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_2","name":"left_click","toolset_name":"computer","input":{}}}),
            delta(
                0,
                json!({"type":"input_json_delta","partial_json":"{\"coordinate\":[5,6]}"}),
            ),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}),
            json!({"type":"message_stop"}),
        ])
    }
    let (origin, task) = serve(Router::new().route("/messages", post(computer))).await;
    let provider = Anthropic::for_loopback_tests(&origin).unwrap();
    let history = [
        Message::User("Open the browser".into()),
        Message::Assistant(cubebot_providers::Reply {
            text: String::new(),
            calls: Vec::new(),
            native: json!([{"type":"tool_use","id":"toolu_1","name":"screenshot","toolset_name":"computer","input":{}}]),
        }),
        Message::ToolResult {
            call_id: "toolu_1".into(),
            output: String::new(),
            image: Some("iVBORw0KGgo=".into()),
        },
    ];
    let tools = [Tool {
        name: "computer".into(),
        description: String::new(),
        parameters: json!({}),
        strict: false,
        computer: true,
    }];
    let request = Request {
        model: "test-model",
        key: "controlled-key",
        system: "",
        messages: &history,
        tools: &tools,
        tools_disabled: false,
        max_output_tokens: Some(1000),
    };
    let events: Vec<Event> = provider
        .stream(&request)
        .map(Result::unwrap)
        .collect()
        .await;
    let [Event::Done { reply, .. }] = &events[..] else {
        panic!("unexpected events");
    };
    let [call] = &reply.calls[..] else {
        panic!("expected one call");
    };
    assert_eq!(
        (call.call_id.as_str(), call.name.as_str()),
        ("toolu_2", "computer")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&call.arguments).unwrap(),
        json!({"action":"left_click","coordinate":[5,6]})
    );
    task.abort();
}

#[tokio::test]
async fn lists_models_with_their_reported_context_windows() {
    async fn models(headers: HeaderMap, RawQuery(query): RawQuery) -> Json<Value> {
        assert_eq!(headers["x-api-key"], "controlled-key");
        assert_eq!(query.as_deref(), Some("limit=1000"));
        Json(json!({"data":[
            {"id":"claude-new","max_input_tokens":1000000},
            {"id":"claude-old"}
        ],"has_more":false}))
    }
    let (origin, task) = serve(Router::new().route("/models", get(models))).await;
    let provider = Anthropic::for_loopback_tests(&origin).unwrap();
    assert_eq!(
        provider.models("controlled-key").await.unwrap(),
        [
            Model {
                id: "claude-new".into(),
                context_window: Some(1_000_000),
                tools: true,
            },
            Model {
                id: "claude-old".into(),
                context_window: None,
                tools: true,
            },
        ]
    );
    task.abort();
}

/// The table Anthropic's pricing page used on 2026-11-04.
const PRICING: &str = "## Model pricing\n\n\
| Model | Base input tokens | 5m cache writes | 1h cache writes | Cache hits and refreshes | Output tokens |\n\
| :---- | :---- | :---- | :---- | :---- | :---- |\n\
| Claude Opus 5.5 | $4 / MTok | $5 / MTok | $8 / MTok | $0.20 / MTok<sup>2</sup> | $20 / MTok |\n\
| Claude Mythos 5 ([limited availability](https://example.com/a)) | $10 / MTok | $12.50 / MTok | $20 / MTok | $1 / MTok | $50 / MTok |\n\
| Claude Haiku 5.5 (for prompts up to 100,000 tokens) | $0.10 / MTok | $0.125 / MTok | $0.20 / MTok | $0.01 / MTok | $0.50 / MTok |\n\
| Claude Haiku 5.5 (for prompts over 100,000 tokens) | $0.50 / MTok | $0.625 / MTok | $1 / MTok | $0.05 / MTok | $2.50 / MTok |\n\n\
## Other\n\n| Concept | Details |\n| --- | --- |\n| **CCU price** | $0.01 USD |\n";

#[tokio::test]
async fn prices_come_from_its_own_page_under_the_ids_its_api_uses() {
    let router = axum::Router::new().route(
        "/pricing.md",
        axum::routing::get(|headers: axum::http::HeaderMap| async move {
            assert!(!headers.contains_key("x-api-key"));
            PRICING
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let prices = cubebot_providers::Anthropic::for_loopback_tests(&origin)
        .unwrap()
        .prices()
        .await
        .unwrap();
    server.abort();
    let names: Vec<&str> = prices.iter().map(|price| price.model.as_str()).collect();
    assert_eq!(
        names,
        ["claude-opus-5.5", "claude-mythos-5", "claude-haiku-5.5"]
    );
    assert!((prices[0].cached_per_token - 0.2e-6).abs() < 1e-12);
    assert!((prices[0].output_per_token - 20e-6).abs() < 1e-12);
    assert!(
        (prices[2].input_per_token - 0.1e-6).abs() < 1e-12,
        "the lowest tier of a tiered model"
    );
}

/// Reads the real page; `cargo test -- --ignored live_page` tells whether the parser still fits.
#[tokio::test]
#[ignore = "reaches platform.claude.com"]
async fn live_page_still_parses() {
    let prices = cubebot_providers::Anthropic::new()
        .unwrap()
        .prices()
        .await
        .unwrap();
    println!(
        "{} Anthropic prices, e.g. {:?}",
        prices.len(),
        prices.first()
    );
    assert!(prices.len() >= 5, "{prices:?}");
    assert!(
        prices.iter().all(|p| p.output_per_token > 0.0),
        "{prices:?}"
    );
}

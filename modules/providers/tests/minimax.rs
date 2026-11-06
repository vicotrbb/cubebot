//! MiniMax's Anthropic-compatible API against a loopback server shaped like its documented
//! responses: https://platform.minimax.io/docs/api-reference/text-chat-anthropic.md
use axum::{
    Json, Router,
    body::Body,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cubebot_providers::{
    Anthropic, Error, Event, Limits, Message, Provider, Reply, Request, Tool, Usage,
};
use futures_util::StreamExt;
use serde_json::{Value, json};

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
    ([("content-type", "text/event-stream")], Body::from(text)).into_response()
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

const THINKING: &str = "The user wants the date; run it.";

/// The first request is answered with thinking and a tool call; the second must replay them.
async fn messages(headers: HeaderMap, Json(body): Json<Value>) -> Response {
    assert_eq!(headers["x-api-key"], "plan-key");
    assert_eq!(body["model"], "MiniMax-M3.1-Flash");
    assert_eq!(body["max_tokens"], 131_072);
    // MiniMax accepts only `auto`, ignores cache breakpoints and has no computer toolset.
    assert!(body.get("cache_control").is_none());
    assert_eq!(body["tool_choice"], json!({"type":"auto"}));
    let tools = body["tools"].as_array().unwrap();
    assert_eq!(
        tools[0],
        json!({"name":"terminal","description":"Run","input_schema":{"type":"object"}})
    );
    assert_eq!(tools[1]["name"], "computer");
    assert_eq!(tools[1]["input_schema"]["required"], json!(["action"]));
    assert!(tools[1].get("type").is_none());
    let messages = body["messages"].as_array().unwrap();
    if messages.len() == 1 {
        return sse(&[
            json!({"type":"message_start","message":{"usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
            delta(0, json!({"type":"thinking_delta","thinking":THINKING})),
            delta(0, json!({"type":"signature_delta","signature":"6d03"})),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call_function_1","name":"terminal","input":{}}}),
            delta(
                1,
                json!({"type":"input_json_delta","partial_json":"{\"command\":\"date\"}"}),
            ),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"input_tokens":14,"output_tokens":91,"cache_creation_input_tokens":0,"cache_read_input_tokens":404}}),
            json!({"type":"message_stop"}),
        ]);
    }
    assert_eq!(
        messages[1],
        json!({"role":"assistant","content":[
            {"type":"thinking","thinking":THINKING,"signature":"6d03"},
            {"type":"tool_use","id":"call_function_1","name":"terminal","input":{"command":"date"}},
        ]})
    );
    assert_eq!(
        messages[2],
        json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"call_function_1","content":"Thu"}]})
    );
    sse(&[
        json!({"type":"message_start","message":{"usage":{"input_tokens":0}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        delta(0, json!({"type":"text_delta","text":"Thursday."})),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":20,"output_tokens":3}}),
        json!({"type":"message_stop"}),
    ])
}

fn tools() -> [Tool; 2] {
    let tool = |name: &str, computer| Tool {
        name: name.into(),
        description: "Run".into(),
        parameters: json!({"type":"object"}),
        strict: true,
        computer,
    };
    [tool("terminal", false), tool("computer", true)]
}

async fn done(provider: &Anthropic, request: &Request<'_>) -> (Reply, Option<Usage>) {
    let events: Vec<Event> = provider.stream(request).map(Result::unwrap).collect().await;
    match events.into_iter().last() {
        Some(Event::Done { reply, usage }) => (reply, usage),
        _ => panic!("no reply"),
    }
}

#[tokio::test]
async fn a_tool_call_round_trip_replays_the_thinking_unchanged() {
    let router = Router::new().route("/messages", post(messages)).route(
        "/models",
        get(|| async {
            Json(json!({"data":[{"id":"MiniMax-M3.1-Flash","type":"model"},{"id":"MiniMax-M3","type":"model"}],"has_more":false}))
        }),
    );
    let (origin, task) = serve(router).await;
    let provider = Anthropic::minimax_for_loopback_tests(&origin).unwrap();
    let models = provider.models("plan-key").await.unwrap();
    assert_eq!(models[0].id, "MiniMax-M3.1-Flash");
    assert_eq!(models[1].context_window, Some(1_000_000));
    let limits = provider.limits("MiniMax-M3.1-Flash", "plan-key").await;
    assert_eq!(
        limits.unwrap(),
        Limits {
            context_window: Some(1_000_000),
            max_output_tokens: Some(131_072)
        }
    );
    let tools = tools();
    let mut history = vec![Message::User("What day is it?".into())];
    fn request<'a>(history: &'a [Message], tools: &'a [Tool]) -> Request<'a> {
        Request {
            model: "MiniMax-M3.1-Flash",
            key: "plan-key",
            system: "Be brief.",
            messages: history,
            tools,
            // Compaction disables tools; MiniMax still gets `auto`.
            tools_disabled: true,
            max_output_tokens: Some(131_072),
        }
    }
    let (reply, usage) = done(&provider, &request(&history, &tools)).await;
    assert_eq!(reply.calls[0].arguments, "{\"command\":\"date\"}");
    assert_eq!(usage.unwrap().input_tokens, 418);
    assert_eq!(usage.unwrap().cached_input_tokens, 404);
    let call = reply.calls[0].call_id.clone();
    history.push(Message::Assistant(reply));
    history.push(Message::ToolResult {
        call_id: call,
        output: "Thu".into(),
        image: None,
    });
    let (reply, _) = done(&provider, &request(&history, &tools)).await;
    assert_eq!(reply.text, "Thursday.");
    task.abort();
}

#[tokio::test]
async fn a_rate_limit_waits_and_retries() {
    let router = Router::new().route(
        "/messages",
        post(|| async {
            (
                StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", "7")],
                Json(json!({"type":"error","error":{"type":"rate_limit_error","message":"usage limit exceeded (2056)"}})),
            )
        }),
    );
    let (origin, task) = serve(router).await;
    let provider = Anthropic::minimax_for_loopback_tests(&origin).unwrap();
    let request = Request {
        model: "MiniMax-M3.1-Flash",
        key: "k",
        system: "",
        messages: &[Message::User("Hi".into())],
        tools: &[],
        tools_disabled: false,
        max_output_tokens: Some(1),
    };
    let events: Vec<_> = provider.stream(&request).collect().await;
    let Some(Err(error @ Error::Transient(_))) = events.first() else {
        panic!("expected a transient error");
    };
    assert!(error.retryable());
    assert_eq!(error.retry_after(), Some(std::time::Duration::from_secs(7)));
    task.abort();
}

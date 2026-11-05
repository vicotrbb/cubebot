use axum::{
    Json, Router,
    body::Body,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cubebot_providers::{Event, Message, Model, OpenAi, Provider, Request, Tool, Usage};
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
            output_tokens: 2,
            cached_input_tokens: 0,
            cost_usd: None,
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
            image: None,
        },
    ];
    let tools = [
        Tool {
            name: "terminal".into(),
            description: "Run".into(),
            parameters: json!({"type":"object"}),
            strict: true,
            computer: false,
        },
        Tool {
            name: "external".into(),
            description: "Anything".into(),
            parameters: json!({"type":"object"}),
            strict: false,
            computer: false,
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

#[tokio::test]
async fn computer_calls_arrive_as_action_batches_and_answer_with_screenshots() {
    async fn computer(Json(body): Json<Value>) -> Response {
        assert_eq!(body["tools"], json!([{"type":"computer"}]));
        assert_eq!(body["input"][3]["type"], "computer_call_output");
        assert_eq!(
            body["input"][3]["output"]["image_url"],
            "data:image/png;base64,iVBORw0KGgo="
        );
        assert_eq!(
            body["input"][4],
            json!({"role":"developer","content":"The computer tool reported: a person has control"})
        );
        assert_eq!(
            body["input"][5]["output"],
            json!([{"type":"input_text","text":"OK"},{"type":"input_image","image_url":"data:image/png;base64,iVBORw0KGgo="}])
        );
        sse(&[
            json!({"type":"response.completed","response":{"status":"completed","output":[
            {"type":"computer_call","call_id":"call_2","status":"completed","actions":[
                {"type":"click","button":"left","x":405,"y":157},{"type":"type","text":"penguin"}]}]}}),
        ])
    }
    let (origin, task) = serve(Router::new().route("/responses", post(computer))).await;
    let provider = OpenAi::for_loopback_tests(&origin).unwrap();
    let earlier = cubebot_providers::Reply {
        text: String::new(),
        calls: Vec::new(),
        native: json!([{"type":"computer_call","call_id":"call_1","actions":[{"type":"screenshot"}]}]),
    };
    let messages = [
        Message::User("Search for penguins".into()),
        Message::Assistant(earlier),
        Message::ToolResult {
            call_id: "call_1".into(),
            output: "a person has control".into(),
            image: Some("iVBORw0KGgo=".into()),
        },
        Message::ToolResult {
            call_id: "call_9".into(),
            output: "OK".into(),
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
    let events = collect(&provider, &request(&messages, &tools)).await;
    let [Event::Done { reply, .. }] = &events[..] else {
        panic!("expected one reply");
    };
    let [call] = &reply.calls[..] else {
        panic!("expected one call");
    };
    assert_eq!(
        (call.call_id.as_str(), call.name.as_str()),
        ("call_2", "computer")
    );
    let arguments: Value = serde_json::from_str(&call.arguments).unwrap();
    assert_eq!(
        arguments["actions"][1],
        json!({"type":"type","text":"penguin"})
    );
    task.abort();
}

#[tokio::test]
async fn only_server_faults_and_unreached_providers_are_retryable() {
    let router = Router::new().route(
        "/responses",
        post(|Json(body): Json<Value>| async move {
            let code = body["model"].as_str().unwrap().to_owned();
            sse(&[
                json!({"type":"response.failed","response":{"status":"failed","error":{"code":code}}}),
            ])
        }),
    );
    let (origin, task) = serve(router).await;
    let provider = OpenAi::for_loopback_tests(&origin).unwrap();
    for (code, retryable) in [("server_error", true), ("invalid_prompt", false)] {
        let mut failed = request(&[], &[]);
        failed.model = code;
        let events: Vec<_> = provider.stream(&failed).collect().await;
        assert_eq!(
            events[0].as_ref().err().unwrap().retryable(),
            retryable,
            "{code}"
        );
    }
    task.abort();
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/", closed.local_addr().unwrap());
    drop(closed);
    let provider = OpenAi::for_loopback_tests(&origin).unwrap();
    let events: Vec<_> = provider.stream(&request(&[], &[])).collect().await;
    assert!(events[0].as_ref().err().unwrap().retryable());
}

#[tokio::test]
async fn lists_conversational_models_newest_first() {
    async fn models(headers: HeaderMap) -> Json<Value> {
        assert_eq!(headers["authorization"], "Bearer controlled-key");
        Json(json!({"object":"list","data":[
            {"id":"gpt-old","created":1},
            {"id":"text-embedding-3-small","created":3},
            {"id":"gpt-new","created":2},
            {"id":"whisper-1","created":4}
        ]}))
    }
    let (origin, task) = serve(Router::new().route("/models", get(models))).await;
    let provider = OpenAi::for_loopback_tests(&origin).unwrap();
    let ids: Vec<Model> = provider.models("controlled-key").await.unwrap();
    assert_eq!(
        ids.iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["gpt-new", "gpt-old"]
    );
    assert!(ids.iter().all(|model| model.context_window.is_none()));
    task.abort();
}

#[tokio::test]
async fn a_rejected_key_lists_no_models() {
    let (origin, task) =
        serve(Router::new().route("/models", get(|| async { StatusCode::UNAUTHORIZED }))).await;
    let provider = OpenAi::for_loopback_tests(&origin).unwrap();
    assert!(matches!(
        provider.models("bad-key").await,
        Err(cubebot_providers::Error::Rejected)
    ));
    task.abort();
}

const PRICING: &str = "# Pricing\n\n### Standard pricing data\n\n\
| Model | Short context input | Short context output | Short context cached input | Short context 30m write | Long context input |\n\
| --- | --- | --- | --- | --- | --- |\n\
| gpt-5.5 (<272K context length) | $5.00 | $30.00 | $0.50 | - | $10.00 |\n\
| gpt-5.5-pro (<272K context length) | $30.00 | $180.00 | - | - | $60.00 |\n\n\
| Model | Input | Output | Cached input |\n| --- | --- | --- | --- |\n\
| gpt-4.1-mini | $0.40 | $1.60 | $0.10 |\n\n\
### Batch pricing data\n\n#### Batch\n\n\
| Model | Input | Output | Cached input |\n| --- | --- | --- | --- |\n\
| gpt-4.1-mini | $0.20 | $0.80 | - |\n";

#[tokio::test]
async fn prices_come_from_the_standard_tier_of_its_own_page_without_a_key() {
    let router = axum::Router::new().route(
        "/pricing.md",
        axum::routing::get(|headers: axum::http::HeaderMap| async move {
            assert!(!headers.contains_key("authorization"));
            PRICING
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let prices = cubebot_providers::OpenAi::for_loopback_tests(&origin)
        .unwrap()
        .prices()
        .await
        .unwrap();
    server.abort();
    let names: Vec<&str> = prices.iter().map(|price| price.model.as_str()).collect();
    assert_eq!(names, ["gpt-5.5", "gpt-5.5-pro", "gpt-4.1-mini"]);
    assert!((prices[0].input_per_token - 5e-6).abs() < 1e-12);
    assert!((prices[0].cached_per_token - 0.5e-6).abs() < 1e-12);
    assert!((prices[0].output_per_token - 30e-6).abs() < 1e-12);
    assert!(
        (prices[1].cached_per_token - 30e-6).abs() < 1e-12,
        "no cached rate bills as input"
    );
    assert!(
        (prices[2].output_per_token - 1.6e-6).abs() < 1e-12,
        "standard, not batch"
    );
}

#[tokio::test]
#[ignore = "reaches developers.openai.com"]
async fn live_page_still_parses() {
    let prices = cubebot_providers::OpenAi::new()
        .unwrap()
        .prices()
        .await
        .unwrap();
    println!("{} OpenAI prices, e.g. {:?}", prices.len(), prices.first());
    assert!(prices.len() >= 5, "{prices:?}");
    assert!(
        prices.iter().all(|p| p.output_per_token > 0.0),
        "{prices:?}"
    );
}

fn desktop() -> Tool {
    Tool {
        name: "computer".into(),
        description: "desktop".into(),
        parameters: json!({}),
        strict: false,
        computer: true,
    }
}

#[tokio::test]
async fn a_chatgpt_plan_sends_its_system_prompt_as_instructions_without_refused_fields() {
    async fn plan(Json(body): Json<Value>) -> Response {
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["instructions"], "Be brief.");
        assert_eq!(body["input"], json!([{"role":"user","content":"Hello"}]));
        for refused in ["max_output_tokens", "temperature", "previous_response_id"] {
            assert!(body.get(refused).is_none(), "{refused}");
        }
        // Hosted computer use is unsupported on plans, so the desktop is a function.
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "computer");
        assert!(body["tools"][0]["parameters"]["properties"]["action"].is_object());
        sse(&[
            json!({"type":"response.completed","response":{"status":"completed",
                "output":[{"type":"message","content":[{"type":"output_text","text":"Hi"}]}]}}),
        ])
    }
    let (origin, task) = serve(Router::new().route("/responses", post(plan))).await;
    let provider = OpenAi::chatgpt_for_loopback_tests(&origin).unwrap();
    let messages = [Message::User("Hello".into())];
    let tools = [desktop()];
    let mut capped = request(&messages, &tools);
    capped.max_output_tokens = Some(512);
    let events = collect(&provider, &capped).await;
    assert!(matches!(&events[..], [Event::Done { reply, .. }] if reply.text == "Hi"));
    task.abort();
}

#[tokio::test]
async fn a_plan_usage_limit_is_neither_retried_nor_reported_as_a_rejected_key() {
    // The shape ChatGPT sent on 2026-11-05 for an exhausted plan: an `error` event, then `response.failed`.
    let limited = json!({"type":"invalid_request_error","code":"subscription_sharing_usage_limit_exceeded","message":"The ChatGPT user has reached their Subscription Sharing usage limit.","param":null});
    let mid_stream = limited.clone();
    let router = Router::new()
        .route(
            "/responses",
            post(move |Json(body): Json<Value>| {
                let limited = mid_stream.clone();
                async move {
                    if body["model"] == "admission" {
                        return (
                            StatusCode::TOO_MANY_REQUESTS,
                            Json(json!({"error": limited})),
                        )
                            .into_response();
                    }
                    sse(&[
                        json!({"type":"response.created"}),
                        json!({"type":"error","error":limited,"sequence_number":3}),
                        json!({"type":"response.failed","response":{"status":"failed","error":limited}}),
                    ])
                }
            }),
        );
    let (origin, task) = serve(router).await;
    let provider = OpenAi::chatgpt_for_loopback_tests(&origin).unwrap();
    for model in ["streamed", "admission"] {
        let mut asked = request(&[], &[]);
        asked.model = model;
        let error = std::pin::pin!(provider.stream(&asked))
            .next()
            .await
            .unwrap()
            .err()
            .unwrap();
        assert_eq!(error.code(), "provider_usage_limit", "{model}");
        assert!(!error.retryable());
    }
    task.abort();
}

#[tokio::test]
async fn a_chatgpt_plan_lists_only_its_listed_models_in_server_order() {
    async fn models(headers: HeaderMap) -> Json<Value> {
        assert_eq!(headers["authorization"], "Bearer controlled-key");
        Json(json!({"models":[
            {"slug":"gpt-6.1-sol","visibility":"list","supported_in_api":true,"context_window":500000},
            {"slug":"gpt-reserve","visibility":"hide","supported_in_api":true,"context_window":272000},
            {"slug":"gpt-6-luna","visibility":"list","supported_in_api":true,"context_window":272000},
            {"slug":"gpt-internal","visibility":"list","supported_in_api":false},
        ]}))
    }
    let (origin, task) = serve(Router::new().route("/models", get(models))).await;
    let provider = OpenAi::chatgpt_for_loopback_tests(&origin).unwrap();
    let listed = provider.models("controlled-key").await.unwrap();
    let ids: Vec<_> = listed.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(ids, ["gpt-6.1-sol", "gpt-6-luna"]);
    assert_eq!(listed[0].context_window, Some(500_000));
    task.abort();
}

#[tokio::test]
async fn supergrok_passes_the_grok_cli_version_gate_and_keeps_the_api_body() {
    async fn proxy(headers: HeaderMap, Json(body): Json<Value>) -> Response {
        if headers
            .get("x-grok-client-version")
            .is_none_or(|v| v != "1.0.13")
        {
            return (StatusCode::UPGRADE_REQUIRED, "outdated").into_response();
        }
        assert_eq!(
            body["input"][0],
            json!({"role":"system","content":"Be brief."})
        );
        assert_eq!(body["max_output_tokens"], 512);
        assert_eq!(body["tools"][0]["type"], "function");
        sse(&[
            json!({"type":"response.completed","response":{"status":"completed",
                "output":[{"type":"function_call","call_id":"call-1","name":"computer","arguments":"{\"action\":\"screenshot\"}"}]}}),
        ])
    }
    async fn models(headers: HeaderMap) -> Json<Value> {
        assert_eq!(headers["x-grok-client-version"], "1.0.13");
        Json(
            json!({"object":"list","data":[{"id":"grok-4.7","object":"model","context_window":256000}]}),
        )
    }
    let router = Router::new()
        .route("/responses", post(proxy))
        .route("/models", get(models));
    let (origin, task) = serve(router).await;
    let provider = OpenAi::supergrok_for_loopback_tests(&origin).unwrap();
    let tools = [desktop()];
    let mut capped = request(&[], &tools);
    capped.max_output_tokens = Some(512);
    let events = collect(&provider, &capped).await;
    assert!(
        matches!(&events[..], [Event::Done { reply, .. }] if reply.calls[0].name == "computer")
    );
    let listed = provider.models("controlled-key").await.unwrap();
    assert_eq!(
        listed,
        [Model {
            id: "grok-4.7".into(),
            context_window: Some(256_000),
            tools: true
        }]
    );
    task.abort();
}

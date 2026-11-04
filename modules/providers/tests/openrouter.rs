use axum::{
    Json, Router,
    body::Body,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cubebot_providers::{Error, Event, Message, Model, OpenRouter, Provider, Reply, Request, Tool};
use futures_util::StreamExt;
use serde_json::{Value, json};

/// Streams chunks split at seven bytes, an SSE comment first and the sentinel last, as OpenRouter does.
fn sse(chunks: &[Value], done: bool) -> Response {
    let mut text = ": OPENROUTER PROCESSING\n\n".to_owned();
    for chunk in chunks {
        text.push_str(&format!("data: {chunk}\n\n"));
    }
    if done {
        text.push_str("data: [DONE]\n\n");
    }
    let parts: Vec<Result<Vec<u8>, std::io::Error>> = text
        .into_bytes()
        .chunks(7)
        .map(|part| Ok(part.to_vec()))
        .collect();
    (
        [("content-type", "text/event-stream")],
        Body::from_stream(futures_util::stream::iter(parts)),
    )
        .into_response()
}

async fn serve(router: Router) -> (OpenRouter, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (OpenRouter::for_loopback_tests(&origin).unwrap(), task)
}

fn request<'a>(messages: &'a [Message], tools: &'a [Tool]) -> Request<'a> {
    Request {
        model: "vendor/model:free",
        key: "controlled-key",
        system: "Be brief.",
        messages,
        tools,
        tools_disabled: false,
        max_output_tokens: None,
    }
}

async fn tool_turn(headers: HeaderMap, Json(body): Json<Value>) -> Response {
    assert_eq!(headers["authorization"], "Bearer controlled-key");
    assert_eq!(body["model"], "vendor/model:free");
    assert_eq!(body["stream"], true);
    assert_eq!(body["messages"][0]["role"], "system");
    assert!(body.get("max_tokens").is_none());
    // The computer tool is an ordinary function here.
    assert_eq!(body["tools"][0]["function"]["name"], "read");
    assert_eq!(body["tools"][1]["function"]["name"], "computer");
    let required = &body["tools"][1]["function"]["parameters"]["required"];
    assert_eq!(*required, json!(["action"]));
    let delta = |delta: Value| json!({"choices":[{"index":0,"delta":delta,"finish_reason":null}]});
    sse(
        &[
            delta(json!({"content":"Let "})),
            delta(
                json!({"content":"me.","reasoning_details":[{"type":"reasoning.encrypted","data":"abc"}]}),
            ),
            delta(
                json!({"tool_calls":[{"index":0,"id":"c1","function":{"name":"read","arguments":"{\"pa"}}]}),
            ),
            delta(json!({"tool_calls":[{"index":0,"function":{"arguments":"th\":\"a\"}"}}]})),
            delta(json!({"tool_calls":[{"index":1,"id":"c2","function":{"name":"computer"}}]})),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":9,"completion_tokens":4,"cost":0.0012,"prompt_tokens_details":{"cached_tokens":3}}}),
        ],
        true,
    )
}

#[tokio::test]
async fn pieced_tool_calls_become_a_reply_that_replays_its_reasoning() {
    let (provider, task) = serve(Router::new().route("/chat/completions", post(tool_turn))).await;
    let tools = [
        Tool::strict("read", "d", json!({"path":{"type":"string"}})),
        Tool {
            name: "computer".into(),
            description: String::new(),
            parameters: json!({}),
            strict: false,
            computer: true,
        },
    ];
    let messages = [Message::User("Hello".into())];
    let request = request(&messages, &tools);
    let events: Vec<Event> = provider
        .stream(&request)
        .map(|event| event.unwrap())
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
    assert_eq!(format!("{first}{second}"), "Let me.");
    assert_eq!(reply.text, "Let me.");
    assert_eq!(reply.calls[0].arguments, "{\"path\":\"a\"}");
    // A call that streamed no arguments still parses.
    assert_eq!(reply.calls[1].arguments, "{}");
    let usage = usage.unwrap();
    assert_eq!(usage.input_tokens, 9);
    assert_eq!(usage.cached_input_tokens, 3);
    assert_eq!(usage.cost_usd, Some(0.0012));
    assert_eq!(
        reply.native["reasoning_details"],
        json!([{"type":"reasoning.encrypted","data":"abc"}])
    );
    assert_eq!(reply.native["tool_calls"][1]["id"], "c2");
    task.abort();
}

async fn replayed(Json(body): Json<Value>) -> Response {
    let messages = &body["messages"];
    assert_eq!(messages[2]["reasoning_details"][0]["data"], "abc");
    // Images wait for the whole run of tool results.
    assert_eq!(messages[3]["role"], "tool");
    assert_eq!(messages[3]["content"], "shot");
    assert_eq!(messages[4]["role"], "tool");
    assert_eq!(messages[5]["role"], "user");
    assert_eq!(
        messages[5]["content"][0]["image_url"]["url"],
        "data:image/png;base64,AAAA"
    );
    assert_eq!(body["max_tokens"], 77);
    sse(
        &[json!({"choices":[{"delta":{"content":"Done"},"finish_reason":"stop"}]})],
        true,
    )
}

#[tokio::test]
async fn native_messages_and_screenshots_go_back_in_chat_completions_form() {
    let (provider, task) = serve(Router::new().route("/chat/completions", post(replayed))).await;
    let messages = [
        Message::User("Hello".into()),
        Message::Assistant(Reply {
            text: "ok".into(),
            calls: Vec::new(),
            native: json!({"role":"assistant","content":"ok","reasoning_details":[{"data":"abc"}]}),
        }),
        Message::ToolResult {
            call_id: "c1".into(),
            output: "shot".into(),
            image: Some("AAAA".into()),
        },
        Message::ToolResult {
            call_id: "c2".into(),
            output: "text".into(),
            image: None,
        },
    ];
    let mut request = request(&messages, &[]);
    request.max_output_tokens = Some(77);
    let last = provider.stream(&request).collect::<Vec<_>>().await.pop();
    assert!(matches!(last, Some(Ok(Event::Done { .. }))));
    task.abort();
}

async fn outcome(chunks: Vec<Value>, done: bool) -> Option<Error> {
    let chunks = std::sync::Arc::new(chunks);
    let route = post(move || {
        let chunks = chunks.clone();
        async move { sse(&chunks, done) }
    });
    let (provider, task) = serve(Router::new().route("/chat/completions", route)).await;
    let messages = [Message::User("x".into())];
    let request = request(&messages, &[]);
    let last = provider.stream(&request).collect::<Vec<_>>().await.pop();
    task.abort();
    last.unwrap().err()
}

#[tokio::test]
async fn failures_inside_the_stream_are_errors_never_partial_replies() {
    let error = |code: u64| json!({"error":{"code":code,"message":"m"},"choices":[{"delta":{"content":""},"finish_reason":"error"}]});
    assert!(matches!(
        outcome(vec![error(502)], true).await,
        Some(Error::Transient(_))
    ));
    assert!(matches!(
        outcome(vec![error(400)], true).await,
        Some(Error::Rejected)
    ));
    let cut = json!({"choices":[{"delta":{"content":"par"},"finish_reason":"length"}]});
    assert!(matches!(
        outcome(vec![cut], true).await,
        Some(Error::InvalidResponse(_))
    ));
    let open = json!({"choices":[{"delta":{"content":"par"},"finish_reason":null}]});
    assert!(matches!(
        outcome(vec![open], false).await,
        Some(Error::Unavailable)
    ));
}

#[tokio::test]
async fn every_model_is_listed_newest_first_after_the_key_is_checked() {
    let mut data = vec![
        json!({"id":"old/one","created":1,"context_length":8000,"supported_parameters":[]}),
        json!({"id":"new/two:free","created":9,"context_length":200000,"supported_parameters":["tools"]}),
    ];
    // Far past the bound on small JSON bodies the other providers' listings fit in.
    for number in 0..4000 {
        data.push(
            json!({"id":format!("x/{number}"),"created":0,"description":"padding padding padding"}),
        );
    }
    let listing = json!({ "data": data });
    let router = Router::new()
        .route(
            "/key",
            get(|headers: HeaderMap| async move {
                if headers["authorization"] == "Bearer controlled-key" {
                    StatusCode::OK
                } else {
                    StatusCode::UNAUTHORIZED
                }
            }),
        )
        .route(
            "/models",
            get(move || {
                let listing = listing.clone();
                async move { Json(listing) }
            }),
        );
    let (provider, task) = serve(router).await;
    let models = provider.models("controlled-key").await.unwrap();
    assert_eq!(models.len(), 4002);
    assert_eq!(
        models[0],
        Model {
            id: "new/two:free".into(),
            context_window: Some(200_000),
            tools: true
        }
    );
    assert!(!models[1].tools);
    assert!(matches!(
        provider.models("bad-key").await,
        Err(Error::Rejected)
    ));
    task.abort();
}

#[tokio::test]
async fn prices_come_from_the_public_listing_without_a_key_and_skip_routers() {
    let router = Router::new().route(
        "/models",
        get(|headers: HeaderMap| async move {
            assert!(!headers.contains_key("authorization"));
            Json(json!({"data":[
                {"id":"openai/gpt-x","pricing":{"prompt":"0.000005","completion":"0.000015","input_cache_read":"0.0000005"}},
                {"id":"vendor/no-cache","pricing":{"prompt":"0.000001","completion":"0.000002"}},
                {"id":"openrouter/auto","pricing":{"prompt":"-1","completion":"-1"}},
                {"id":"vendor/unpriced"},
            ]}))
        }),
    );
    let (provider, task) = serve(router).await;
    let prices = provider.prices().await.unwrap();
    task.abort();
    assert_eq!(prices.len(), 2, "{prices:?}");
    assert_eq!(prices[0].model, "openai/gpt-x");
    assert!((prices[0].cached_per_token - 0.0000005).abs() < 1e-12);
    assert_eq!(prices[1].cached_per_token, prices[1].input_per_token);
}

mod other_completions_providers {
    use super::*;
    use cubebot_providers::{Gemini, Grok, Usage};

    async fn origin(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (origin, task)
    }

    async fn done<P: Provider>(provider: &P, request: &Request<'_>) -> (Reply, Option<Usage>) {
        let stream = provider.stream(request);
        let mut stream = std::pin::pin!(stream);
        while let Some(event) = stream.next().await {
            if let Event::Done { reply, usage } = event.unwrap() {
                return (reply, usage);
            }
        }
        panic!("no final event");
    }

    /// Parallel calls with index 0 each and no id on one, as Gemini sends, with a thought signature.
    async fn gemini_turn(headers: HeaderMap, Json(body): Json<Value>) -> Response {
        assert_eq!(headers["authorization"], "Bearer controlled-key");
        assert_eq!(body["stream_options"]["include_usage"], true);
        let declared = &body["tools"][0]["function"];
        assert!(declared["parameters"].get("additionalProperties").is_none());
        sse(
            &[
                json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","type":"function","function":{"name":"read","arguments":"{\"path\":\"x\"}"},"extra_content":{"google":{"thought_signature":"sig"}}}]},"finish_reason":null}]}),
                json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"b","type":"function","function":{"name":"read","arguments":"{\"path\":\"y\"}"}}]},"finish_reason":null}]}),
                json!({"choices":[{"delta":{"tool_calls":[{"id":"","type":"function","function":{"name":"read","arguments":"{}"}}]},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":2}}),
            ],
            true,
        )
    }

    #[tokio::test]
    async fn gemini_tool_calls_stay_apart_and_keep_their_thought_signature() {
        let router = Router::new().route("/openai/chat/completions", post(gemini_turn));
        let (origin, _server) = origin(router).await;
        let gemini = Gemini::for_loopback_tests(&origin).unwrap();
        let messages = [Message::User("go".into())];
        let tools = [Tool::strict(
            "read",
            "Reads.",
            json!({"path":{"type":"string"}}),
        )];
        let (reply, usage) = done(&gemini, &request(&messages, &tools)).await;
        assert_eq!(reply.calls.len(), 3);
        assert_eq!(reply.calls[0].call_id, "a");
        assert_eq!(reply.calls[1].call_id, "b");
        assert_eq!(reply.calls[2].call_id, "call_2", "an empty id is filled in");
        assert_eq!(
            reply.native["tool_calls"][0]["extra_content"]["google"]["thought_signature"],
            "sig"
        );
        assert!(reply.native["tool_calls"][1].get("extra_content").is_none());
        assert_eq!(usage.unwrap().input_tokens, 10);
    }

    async fn grok_turn(Json(body): Json<Value>) -> Response {
        assert_eq!(body["stream_options"]["include_usage"], true);
        sse(
            &[
                json!({"choices":[{"delta":{"content":"Hi"},"finish_reason":"stop"}]}),
                json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":2,"cost_in_usd_ticks":5_000_000_000u64}}),
            ],
            true,
        )
    }

    #[tokio::test]
    async fn grok_reports_each_responses_cost_in_ticks() {
        let router = Router::new().route("/chat/completions", post(grok_turn));
        let (origin, _server) = origin(router).await;
        let grok = Grok::for_loopback_tests(&origin).unwrap();
        let messages = [Message::User("go".into())];
        let (reply, usage) = done(&grok, &request(&messages, &[])).await;
        assert_eq!(reply.text, "Hi");
        assert_eq!(usage.unwrap().cost_usd, Some(0.5));
    }

    #[tokio::test]
    async fn listings_report_windows_and_keep_only_what_converses() {
        let router = Router::new()
            .route(
                "/models",
                get(|headers: HeaderMap| async move {
                    if headers.get("x-goog-api-key").is_some() {
                        return Json(json!({"models":[
                            {"name":"models/gemini-3.8-flash","inputTokenLimit":1048576,"supportedGenerationMethods":["generateContent"]},
                            {"name":"models/embedding-1","inputTokenLimit":2048,"supportedGenerationMethods":["embedContent"]},
                        ]}));
                    }
                    Json(json!({"data":[
                        {"id":"grok-4.7","created":2,"context_length":2000000,"completion_text_token_price":60000},
                        {"id":"grok-imagine","created":3,"completion_text_token_price":0},
                    ]}))
                }),
            );
        let (origin, _server) = origin(router).await;
        let gemini = Gemini::for_loopback_tests(&origin).unwrap();
        assert_eq!(
            gemini.models("key").await.unwrap(),
            [Model {
                id: "gemini-3.8-flash".into(),
                context_window: Some(1048576),
                tools: true
            }]
        );
        let grok = Grok::for_loopback_tests(&origin).unwrap();
        assert_eq!(
            grok.models("key").await.unwrap(),
            [Model {
                id: "grok-4.7".into(),
                context_window: Some(2000000),
                tools: true
            }]
        );
    }
}

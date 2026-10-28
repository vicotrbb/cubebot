use axum::{Json, Router, http::HeaderMap, routing::post};
use cubebot_providers::OpenAi;
use serde_json::{Value, json};

async fn reply(headers: HeaderMap, Json(body): Json<Value>) -> Json<Value> {
    assert_eq!(headers["authorization"], "Bearer controlled-key");
    assert_eq!(body["store"], false);
    assert_eq!(body["input"][0]["content"], "Hello");
    assert!(body.get("tools").is_none());
    assert!(body.get("parallel_tool_calls").is_none());
    Json(
        json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"Hello back"}]}],"usage":{"input_tokens":5,"output_tokens":2}}),
    )
}

#[tokio::test]
async fn sends_a_bounded_stateless_text_request_and_extracts_output() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/responses", post(reply)))
            .await
            .unwrap();
    });
    let provider = OpenAi::for_loopback_tests(&origin).unwrap();
    let step = provider
        .tool_step(
            "test-model",
            "controlled-key",
            &[json!({"role":"user","content":"Hello"})],
            &[],
        )
        .await
        .unwrap();
    assert_eq!(step.text, "Hello back");
    assert!(step.call.is_none());
    assert_eq!(step.usage, (5, 2));
    assert!(OpenAi::for_loopback_tests("https://elsewhere.example/").is_err());
    task.abort();
}

#[tokio::test]
async fn provider_failure_does_not_expose_remote_payloads_or_accept_partial_text() {
    use axum::http::StatusCode;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/responses",
                post(|| async { (StatusCode::UNAUTHORIZED, "sensitive upstream diagnostics") }),
            ),
        )
        .await
        .unwrap();
    });
    let Err(error) = OpenAi::for_loopback_tests(&origin)
        .unwrap()
        .tool_step("test", "test", &[], &[])
        .await
    else {
        panic!("provider rejection was accepted");
    };
    assert_eq!(error.code(), "provider_rejected");
    assert!(!format!("{error:?}").contains("sensitive"));
    task.abort();
}

#[tokio::test]
async fn terminal_calls_are_native_bounded_and_return_their_provider_identity() {
    async fn tool_reply(Json(body): Json<Value>) -> Json<Value> {
        assert_eq!(body["tools"][0]["name"], "terminal");
        assert_eq!(body["parallel_tool_calls"], false);
        Json(
            json!({"status":"completed","output":[{"type":"function_call","name":"terminal","call_id":"call_1","arguments":"{\"command\":\"printf hello\"}"}]}),
        )
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/responses", post(tool_reply)),
        )
        .await
        .unwrap();
    });
    let provider = OpenAi::for_loopback_tests(&origin).unwrap();
    let step = provider
        .tool_step(
            "test",
            "test",
            &[],
            &[json!({"type":"function","name":"terminal"})],
        )
        .await
        .unwrap();
    let call = step.call.unwrap();
    assert_eq!(call.call_id, "call_1");
    assert_eq!(call.name, "terminal");
    assert!(call.arguments.contains("printf hello"));
    task.abort();
}

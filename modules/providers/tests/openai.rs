use axum::{Json, Router, http::HeaderMap, routing::post};
use cubebot_protocol::chat::Message;
use cubebot_providers::OpenAi;
use serde_json::{Value, json};

async fn reply(headers: HeaderMap, Json(body): Json<Value>) -> Json<Value> {
    assert_eq!(headers["authorization"], "Bearer controlled-key");
    assert_eq!(body["store"], false);
    assert_eq!(body["input"][0]["content"], "Hello");
    assert!(body.get("tools").is_none());
    Json(
        json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"Hello back"}]}]}),
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
    let result = provider
        .complete(
            "test-model",
            "controlled-key",
            &[Message {
                role: "user".into(),
                content: "Hello".into(),
            }],
        )
        .await
        .unwrap();
    assert_eq!(result, "Hello back");
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
    let error = OpenAi::for_loopback_tests(&origin)
        .unwrap()
        .complete("test", "test", &[])
        .await
        .unwrap_err();
    assert_eq!(error.code(), "provider_rejected");
    assert!(!format!("{error:?}").contains("sensitive"));
    task.abort();
}

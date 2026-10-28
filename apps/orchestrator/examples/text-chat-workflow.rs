//! Local qualification only: real PostgreSQL, HTTP and native runtime; controlled Kubernetes API.
#[path = "../tests/support/registry.rs"]
mod registry_support;
#[path = "../tests/support/runtime.rs"]
mod runtime_support;
#[path = "../tests/support/mod.rs"]
mod support;
use axum::{Json, Router, http::StatusCode, routing::post};
use cubebot_orchestrator::{
    connections::{ConfigureProvider, Connections, Credentials},
    identity::Identity,
};
use serde_json::{Value, json};
use std::{os::unix::fs::DirBuilderExt, path::PathBuf};
use uuid::Uuid;

#[tokio::main]
async fn main() {
    let f = runtime_support::fixture().await;
    let credentials = Credentials::new([7; 32]);
    let identity = Identity::new(f.pool.clone());
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    identity
        .create_account(&admin, "member", "member password", false, Uuid::new_v4())
        .await
        .unwrap();
    let key = std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "controlled-provider-key".into());
    let model = std::env::var("CUBEBOT_TEST_MODEL").unwrap_or_else(|_| "controlled-model".into());
    let connections = Connections::new(f.pool.clone(), credentials.clone());
    connections
        .configure(
            &admin,
            &ConfigureProvider {
                request_id: Uuid::new_v4(),
                expected_revision: 0,
                provider: cubebot_protocol::runtime::ProviderKind::Openai,
                model,
                context_window: Some(128_000),
                api_key: key,
                access: None,
            },
        )
        .await
        .unwrap();
    let auth = f.auth.with_credentials(credentials.clone());
    let router = cubebot_orchestrator::api::configured_router_with_credentials(
        f.pool.clone(),
        Some("http://127.0.0.1:3300".into()),
        None,
        Some(credentials),
    )
    .await
    .unwrap()
    .merge(cubebot_orchestrator::api::runtime_router(auth));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080")
        .await
        .unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let (provider_origin, provider_task) = controlled_provider().await;
    let root = private_directory();
    let token = root.join("token");
    std::fs::write(&token, "projected-qualification-token").unwrap();
    let mut runtime = std::process::Command::new("target/debug/cubebot-agent");
    runtime
        .env_remove("OPENAI_API_KEY")
        .env("CUBEBOT_ORCHESTRATOR_ORIGIN", "http://127.0.0.1:8080/")
        .env("CUBEBOT_BOOTSTRAP_TOKEN_FILE", token)
        .env("CUBEBOT_JOURNAL_DIRECTORY", &root);
    if std::env::var_os("OPENAI_API_KEY").is_none() {
        runtime.env("CUBEBOT_TEST_PROVIDER_ORIGIN", provider_origin);
    }
    let mut child = runtime.spawn().unwrap();
    // Safe metadata only. Passwords and projected identity here are disposable controlled fixtures.
    println!(
        "Local text-chat workflow ready; controlled Kubernetes; agent {}",
        f.agent.0
    );
    let _ = (&f.api, f.attempt);
    tokio::signal::ctrl_c().await.unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    server.abort();
    provider_task.abort();
    f.task.abort();
    std::fs::remove_dir_all(root).unwrap();
}

fn private_directory() -> PathBuf {
    let root = std::env::temp_dir().join(format!("cubebot-chat-workflow-{}", Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    root
}
async fn controlled_provider() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/responses", post(reply)))
            .await
            .unwrap();
    });
    (origin, task)
}
async fn reply(Json(body): Json<Value>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let text = body["input"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap();
    if text.contains("FAIL_PROVIDER") {
        return (StatusCode::SERVICE_UNAVAILABLE, "controlled outage").into_response();
    }
    let completed = json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":format!("Controlled reply: {text}")}]}]}});
    (
        [("content-type", "text/event-stream")],
        format!("data: {completed}\n\n"),
    )
        .into_response()
}

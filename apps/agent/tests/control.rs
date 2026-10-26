use axum::{
    Json, Router,
    extract::State,
    http::HeaderMap,
    routing::{get, post},
};
use cubebot_agent::control::{Config, RuntimeClient};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

struct Api {
    instance: Option<Uuid>,
    version: u64,
    request: Uuid,
    token: String,
    wrong_actor: bool,
    acknowledgements: usize,
    controls: usize,
}
async fn bootstrap(State(state): State<Arc<Mutex<Api>>>, Json(body): Json<Value>) -> Json<Value> {
    let mut state = state.lock().unwrap();
    assert_eq!(body["token"], state.token);
    let instance: Uuid = serde_json::from_value(body["instance"].clone()).unwrap();
    assert!(!instance.is_nil());
    if let Some(previous) = state.instance {
        assert_eq!(instance, previous);
    }
    state.instance = Some(instance);
    state.version += 1;
    Json(
        json!({"token":format!("cb_runtime_{}", "a".repeat(64)),"agent":Uuid::from_u128(1),"attempt":Uuid::from_u128(2),"epoch":1,"version":state.version,"expires_in_seconds":120}),
    )
}
async fn control(State(state): State<Arc<Mutex<Api>>>, headers: HeaderMap) -> Json<Value> {
    assert_eq!(
        headers["authorization"],
        format!("Bearer cb_runtime_{}", "a".repeat(64))
    );
    let mut state = state.lock().unwrap();
    state.controls += 1;
    Json(
        json!({"actor":{"agent":Uuid::from_u128(1),"attempt":Uuid::from_u128(if state.wrong_actor {3} else {2}),"epoch":1,"version":state.version},"disposition":"drain","drain":{"request_id":state.request,"acknowledged":false}}),
    )
}
async fn drained(
    State(state): State<Arc<Mutex<Api>>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    assert_eq!(
        headers["authorization"],
        format!("Bearer cb_runtime_{}", "a".repeat(64))
    );
    let mut state = state.lock().unwrap();
    assert_eq!(body["request_id"], json!(state.request));
    state.acknowledgements += 1;
    Json(json!({"request_id":state.request,"acknowledged":true}))
}

#[tokio::test]
async fn native_client_bootstraps_renews_and_reports_bound_drain() {
    let path = std::env::temp_dir().join(format!("cubebot-control-{}", Uuid::new_v4()));
    std::fs::write(&path, "projected-first").unwrap();
    let state = Arc::new(Mutex::new(Api {
        instance: None,
        version: 0,
        request: Uuid::new_v4(),
        token: "projected-first".into(),
        wrong_actor: false,
        acknowledgements: 0,
        controls: 0,
    }));
    let router = Router::new()
        .route("/api/runtime/bootstrap", post(bootstrap))
        .route("/api/runtime/control", get(control))
        .route("/api/runtime/drained", post(drained))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let config = Config::new(&origin, path.clone()).unwrap();
    let mut client = RuntimeClient::new(config).unwrap();
    assert!(client.control().await.is_err());
    client.bootstrap().await.unwrap();
    let first = client.control().await.unwrap();
    assert_eq!(first.actor.version, 1);
    let request = first.drain.unwrap().request_id;
    assert!(
        client
            .acknowledge_drain(request)
            .await
            .unwrap()
            .acknowledged
    );
    std::fs::write(&path, "projected-second").unwrap();
    state.lock().unwrap().token = "projected-second".into();
    client.bootstrap().await.unwrap();
    assert_eq!(client.control().await.unwrap().actor.version, 2);
    state.lock().unwrap().wrong_actor = true;
    assert!(client.control().await.is_err());
    task.abort();
    std::fs::remove_file(path).unwrap();
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

#[tokio::test]
async fn native_process_completes_empty_journal_drain_and_retains_checkpoint() {
    use std::os::unix::fs::DirBuilderExt;
    let root = std::env::temp_dir().join(format!("cubebot-native-{}", Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    let token = root.join("token");
    std::fs::write(&token, "projected-process").unwrap();
    let state = Arc::new(Mutex::new(Api {
        instance: None,
        version: 0,
        request: Uuid::new_v4(),
        token: "projected-process".into(),
        wrong_actor: false,
        acknowledgements: 0,
        controls: 0,
    }));
    let router = Router::new()
        .route("/api/runtime/bootstrap", post(bootstrap))
        .route("/api/runtime/control", get(control))
        .route("/api/runtime/drained", post(drained))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_cubebot-agent"))
        .env("CUBEBOT_ORCHESTRATOR_ORIGIN", origin)
        .env("CUBEBOT_BOOTSTRAP_TOKEN_FILE", token)
        .env("CUBEBOT_JOURNAL_DIRECTORY", &root)
        .spawn()
        .unwrap();
    let status = child_exit(&mut child).await;
    assert!(status.success());
    assert!(root.join("journal/state").is_file());
    task.abort();
    std::fs::remove_dir_all(root).unwrap();
}
async fn child_exit(child: &mut std::process::Child) -> std::process::ExitStatus {
    for _ in 0..100 {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    child.kill().unwrap();
    child.wait().unwrap();
    panic!("native runtime did not finish the drain");
}

#[derive(Clone, Copy)]
enum BadReply {
    Oversized,
    Redirect,
    Expired,
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
        BadReply::Expired => {
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
            Json(issued(1)).into_response()
        }
    }
}
fn issued(expires: u64) -> Value {
    json!({"token":format!("cb_runtime_{}", "a".repeat(64)),"agent":Uuid::from_u128(1),"attempt":Uuid::from_u128(2),"epoch":1,"version":1,"expires_in_seconds":expires})
}
#[tokio::test]
async fn bootstrap_rejects_oversized_redirected_and_already_expired_responses() {
    use cubebot_agent::control::Error;
    for case in [BadReply::Oversized, BadReply::Redirect, BadReply::Expired] {
        let router = Router::new()
            .route("/api/runtime/bootstrap", post(bad_bootstrap))
            .route("/leak", axum::routing::any(|| async { Json(issued(120)) }))
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
                | (BadReply::Expired, Err(Error::Expired))
        ));
        assert!(client.requires_bootstrap());
        assert!(client.control().await.is_err());
        task.abort();
        std::fs::remove_file(path).unwrap();
    }
}

#[tokio::test]
async fn native_process_retains_pending_evidence_and_never_acknowledges_clean_drain() {
    use std::os::unix::fs::DirBuilderExt;
    for signal in ["-INT", "-TERM"] {
        let root = std::env::temp_dir().join(format!("cubebot-pending-{}", Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let mut journal = cubebot_agent::journal::Journal::open(&root.join("journal")).unwrap();
        journal.append(b"unsynchronized outcome").unwrap();
        drop(journal);
        let token = root.join("token");
        std::fs::write(&token, "projected-pending").unwrap();
        let state = Arc::new(Mutex::new(Api {
            instance: None,
            version: 0,
            request: Uuid::new_v4(),
            token: "projected-pending".into(),
            wrong_actor: false,
            acknowledgements: 0,
            controls: 0,
        }));
        let router = Router::new()
            .route("/api/runtime/bootstrap", post(bootstrap))
            .route("/api/runtime/control", get(control))
            .route("/api/runtime/drained", post(drained))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_cubebot-agent"))
            .env("CUBEBOT_ORCHESTRATOR_ORIGIN", origin)
            .env("CUBEBOT_BOOTSTRAP_TOKEN_FILE", token)
            .env("CUBEBOT_JOURNAL_DIRECTORY", &root)
            .spawn()
            .unwrap();
        for _ in 0..100 {
            if state.lock().unwrap().controls > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let bootstrapped = state.lock().unwrap().controls > 0;
        let running = child.try_wait().unwrap().is_none();
        if running {
            assert!(
                std::process::Command::new("kill")
                    .arg(signal)
                    .arg(child.id().to_string())
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let status = child_exit(&mut child).await;
        let acknowledgements = state.lock().unwrap().acknowledgements;
        let journal = cubebot_agent::journal::Journal::open(&root.join("journal")).unwrap();
        assert!(bootstrapped);
        assert!(running);
        assert!(status.success());
        assert_eq!(acknowledgements, 0);
        assert_eq!(journal.pending()[0].payload, b"unsynchronized outcome");
        drop(journal);
        task.abort();
        std::fs::remove_dir_all(root).unwrap();
    }
}

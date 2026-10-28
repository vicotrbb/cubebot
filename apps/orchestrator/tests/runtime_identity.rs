use crate::runtime_support;
use axum::Router;
use cubebot_orchestrator::{
    agents::{
        kubernetes::{Registry, ResourceObject},
        runtime::{RuntimeAccess, RuntimeIdentity},
    },
    identity::Identity,
};
use runtime_support::{Fixture, fixture};
use serde_json::{Value, json};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn runtime_sessions_last_for_the_incarnation_rotate_and_stay_separate_from_human_credentials()
{
    let f = fixture().await;
    let instance = Uuid::new_v4();
    let first = f.auth.bootstrap("projected-token", instance).await.unwrap();
    let actor = f
        .auth
        .authenticate(&first.token, RuntimeAccess::Active)
        .await
        .unwrap();
    assert_eq!(actor.agent, f.agent);
    assert_eq!(actor.attempt, f.attempt);
    let stored: String =
        sqlx::query_scalar("SELECT to_jsonb(s)::text FROM agent_runtime_sessions s")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert!(!stored.contains(&first.token));
    assert!(!stored.contains("projected-token"));
    assert!(
        Identity::new(f.pool.clone())
            .authenticate(&first.token)
            .await
            .is_err()
    );
    assert!(
        f.auth
            .authenticate(&"a".repeat(64), RuntimeAccess::Active)
            .await
            .is_err()
    );
    let second = f.auth.bootstrap("projected-token", instance).await.unwrap();
    assert!(
        f.auth
            .authenticate(&first.token, RuntimeAccess::Recovery)
            .await
            .is_err()
    );
    assert_eq!(second.version, first.version + 1);
    // No clock ends the credential; only the incarnation ending revokes it.
    sqlx::query(
        "UPDATE agent_runtime_sessions SET created_at = clock_timestamp() - interval '30 days'",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(
        f.auth
            .authenticate(&second.token, RuntimeAccess::Active)
            .await
            .is_ok()
    );
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_different_process_or_recreated_service_account_cannot_take_over_the_incarnation() {
    let f = fixture().await;
    let instance = Uuid::new_v4();
    let first = f.auth.bootstrap("projected-token", instance).await.unwrap();
    assert!(
        f.auth
            .bootstrap("projected-token", Uuid::new_v4())
            .await
            .is_err()
    );
    f.api.lock().unwrap().service_uid = "recreated-service-account".into();
    assert!(f.auth.bootstrap("projected-token", instance).await.is_err());
    assert!(
        f.auth
            .authenticate(&first.token, RuntimeAccess::Active)
            .await
            .is_ok()
    );
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn lifecycle_revokes_active_access_while_preserving_bounded_recovery_identity() {
    use cubebot_orchestrator::agents::{Agents, LifecycleAction, LifecycleCommand};
    let f = fixture().await;
    let session = f
        .auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    let identity = Identity::new(f.pool.clone());
    let human = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let actor = identity.authenticate(&human.token).await.unwrap();
    Agents::new(f.pool.clone())
        .lifecycle(
            &actor,
            f.agent,
            &LifecycleCommand {
                request_id: Uuid::new_v4(),
                expected_revision: 2,
                action: LifecycleAction::Restart,
            },
        )
        .await
        .unwrap();
    assert!(
        f.auth
            .authenticate(&session.token, RuntimeAccess::Active)
            .await
            .is_err()
    );
    assert!(
        f.auth
            .authenticate(&session.token, RuntimeAccess::Recovery)
            .await
            .is_ok()
    );
    sqlx::query(
        "UPDATE agent_resources SET phase = 'stopped',termination_proof = '{}' WHERE id = $1",
    )
    .bind(f.attempt)
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(
        f.auth
            .authenticate(&session.token, RuntimeAccess::Recovery)
            .await
            .is_err()
    );
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn wrong_pod_identity_and_specification_cannot_obtain_runtime_credentials() {
    let f = fixture().await;
    f.api.lock().unwrap().pod.metadata.uid = Some("different-pod".into());
    assert!(
        f.auth
            .bootstrap("projected-token", Uuid::new_v4())
            .await
            .is_err()
    );
    f.api.lock().unwrap().pod.metadata.uid = Some("pod-uid".into());
    f.api
        .lock()
        .unwrap()
        .pod
        .spec
        .as_mut()
        .unwrap()
        .host_network = Some(true);
    assert!(
        f.auth
            .bootstrap("projected-token", Uuid::new_v4())
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_runtime_sessions")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn http_exchange_uses_runtime_credentials_and_never_accepts_a_human_bearer() {
    let f = fixture().await;
    let (origin, task) = runtime_server(f.auth.clone()).await;
    let client = openidconnect::reqwest::Client::new();
    let response = client
        .post(format!("{origin}/api/runtime/bootstrap"))
        .header("content-type", "application/json")
        .body(json!({"token":"projected-token","instance":Uuid::new_v4()}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let issued: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    let runtime = client
        .get(format!("{origin}/api/runtime/identity"))
        .bearer_auth(issued["token"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(runtime.status(), 200);
    let human = Identity::new(f.pool.clone())
        .login("admin", "administrator password")
        .await
        .unwrap();
    let denied = client
        .get(format!("{origin}/api/runtime/identity"))
        .bearer_auth(human.token)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);
    task.abort();
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn competing_processes_cannot_both_bootstrap_the_same_physical_attempt() {
    let f = fixture().await;
    let (left, right) = tokio::join!(
        f.auth.bootstrap("projected-token", Uuid::new_v4()),
        f.auth.bootstrap("projected-token", Uuid::new_v4())
    );
    assert_ne!(left.is_ok(), right.is_ok());
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn bootstrap_deadline_does_not_issue_a_session_after_waiting_on_lifecycle_lock() {
    use cubebot_orchestrator::agents::kubernetes::Error;
    let f = fixture().await;
    let mut blocker = f.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM agents WHERE id = $1 FOR UPDATE")
        .bind(f.agent)
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    let instance = Uuid::new_v4();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(25),
        f.auth.bootstrap("projected-token", instance),
    )
    .await
    .unwrap();
    assert!(matches!(outcome, Err(Error::DeadlineExceeded)));
    blocker.rollback().await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_runtime_sessions")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(f.auth.bootstrap("projected-token", instance).await.is_ok());
    f.task.abort();
}

async fn runtime_server(auth: RuntimeIdentity) -> (String, tokio::task::JoinHandle<()>) {
    http_server(cubebot_orchestrator::api::runtime_router(auth)).await
}
async fn http_server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (origin, task)
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn rejected_http_inputs_are_bounded_uncached_and_do_not_echo_input_secrets() {
    let f = fixture().await;
    let (origin, task) = runtime_server(f.auth.clone()).await;
    let client = openidconnect::reqwest::Client::new();
    for body in [
        json!({"secret-in-field-name":"private"}).to_string(),
        json!({"token":"a".repeat(30 * 1024),"instance":Uuid::new_v4()}).to_string(),
    ] {
        let response = client
            .post(format!("{origin}/api/runtime/bootstrap"))
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_client_error());
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(
            response.text().await.unwrap(),
            "{\"error\":\"runtime_identity_unavailable\"}"
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_runtime_sessions")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    task.abort();
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn uncertain_physical_ownership_denies_active_access_until_verified_again() {
    let f = fixture().await;
    let session = f
        .auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    let registry = Registry::new(f.pool.clone());
    registry.record_missing(f.attempt).await.unwrap();
    assert!(
        f.auth
            .authenticate(&session.token, RuntimeAccess::Active)
            .await
            .is_err()
    );
    assert!(
        f.auth
            .authenticate(&session.token, RuntimeAccess::Recovery)
            .await
            .is_ok()
    );
    let pod = f.api.lock().unwrap().pod.clone();
    registry
        .bind_object(f.attempt, &ResourceObject::Pod(Box::new(pod)))
        .await
        .unwrap();
    assert!(
        f.auth
            .authenticate(&session.token, RuntimeAccess::Active)
            .await
            .is_ok()
    );
    f.task.abort();
}

async fn lifecycle(
    f: &Fixture,
    action: cubebot_orchestrator::agents::LifecycleAction,
    revision: i64,
) {
    use cubebot_orchestrator::agents::{Agents, LifecycleCommand};
    let identity = Identity::new(f.pool.clone());
    let human = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let actor = identity.authenticate(&human.token).await.unwrap();
    Agents::new(f.pool.clone())
        .lifecycle(
            &actor,
            f.agent,
            &LifecycleCommand {
                request_id: Uuid::new_v4(),
                expected_revision: revision,
                action,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn drain_request_is_durable_idempotent_and_cannot_restore_active_authority() {
    use cubebot_orchestrator::agents::LifecycleAction;
    let f = fixture().await;
    let instance = Uuid::new_v4();
    let session = f.auth.bootstrap("projected-token", instance).await.unwrap();
    assert!(
        f.auth
            .control(&session.token)
            .await
            .unwrap()
            .drain
            .is_none()
    );
    assert!(
        f.auth
            .acknowledge_drain(&session.token, Uuid::new_v4())
            .await
            .is_err()
    );
    lifecycle(&f, LifecycleAction::Sleep, 2).await;
    let request = f.auth.control(&session.token).await.unwrap().drain.unwrap();
    assert!(!request.acknowledged);
    assert!(
        f.auth
            .acknowledge_drain(&session.token, Uuid::new_v4())
            .await
            .is_err()
    );
    let acknowledged = f
        .auth
        .acknowledge_drain(&session.token, request.request_id)
        .await
        .unwrap();
    assert!(acknowledged.acknowledged);
    let original: String = sqlx::query_scalar(
        "SELECT acknowledged_at::text FROM agent_runtime_drains WHERE attempt_id = $1",
    )
    .bind(f.attempt)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    let rotated = f.auth.bootstrap("projected-token", instance).await.unwrap();
    f.auth
        .acknowledge_drain(&rotated.token, request.request_id)
        .await
        .unwrap();
    let retained: String = sqlx::query_scalar(
        "SELECT acknowledged_at::text FROM agent_runtime_drains WHERE attempt_id = $1",
    )
    .bind(f.attempt)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(original, retained);
    lifecycle(&f, LifecycleAction::Wake, 3).await;
    let control = f.auth.control(&rotated.token).await.unwrap();
    assert_eq!(control.drain.unwrap().request_id, request.request_id);
    assert!(
        f.auth
            .authenticate(&rotated.token, RuntimeAccess::Active)
            .await
            .is_err()
    );
    let phase: String = sqlx::query_scalar("SELECT phase FROM agent_resources WHERE id = $1")
        .bind(f.attempt)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(phase, "bound");
    // Even an accidentally restored generation cannot reopen a latched incarnation.
    sqlx::query("UPDATE agents SET restart_generation = (SELECT target_generation FROM agent_resources WHERE id = $1) WHERE id = $2")
        .bind(f.attempt).bind(f.agent).execute(&f.pool).await.unwrap();
    assert!(
        f.auth
            .authenticate(&rotated.token, RuntimeAccess::Active)
            .await
            .is_err()
    );
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn drain_reports_recheck_revocation_after_waiting_for_lifecycle_lock() {
    use cubebot_orchestrator::agents::LifecycleAction;
    let f = fixture().await;
    let session = f
        .auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    lifecycle(&f, LifecycleAction::Restart, 2).await;
    let request = f.auth.control(&session.token).await.unwrap().drain.unwrap();
    let mut transaction = f.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM agents WHERE id = $1 FOR UPDATE")
        .bind(f.agent)
        .execute(&mut *transaction)
        .await
        .unwrap();
    let auth = f.auth.clone();
    let report = tokio::spawn(async move {
        auth.acknowledge_drain(&session.token, request.request_id)
            .await
    });
    // Wait until PostgreSQL proves the report is contending on the held lifecycle lock.
    wait_for_runtime_lock(&f.pool).await;
    sqlx::query(
        "UPDATE agent_resources SET phase = 'stopped',termination_proof = '{}' WHERE id = $1",
    )
    .bind(f.attempt)
    .execute(&mut *transaction)
    .await
    .unwrap();
    transaction.commit().await.unwrap();
    assert!(report.await.unwrap().is_err());
    let acknowledged: bool = sqlx::query_scalar(
        "SELECT acknowledged_at IS NOT NULL FROM agent_runtime_drains WHERE attempt_id = $1",
    )
    .bind(f.attempt)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert!(!acknowledged);
    f.task.abort();
}

async fn wait_for_runtime_lock(pool: &sqlx::PgPool) {
    for _ in 0..100 {
        let waiting: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' AND query LIKE 'SELECT id FROM agents%')")
            .fetch_one(pool).await.unwrap();
        if waiting {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("runtime did not wait on held lifecycle lock");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn drain_is_requested_and_acknowledged_on_the_channel() {
    use cubebot_orchestrator::agents::LifecycleAction;
    let f = fixture().await;
    let session = f
        .auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    lifecycle(&f, LifecycleAction::Retire, 2).await;
    let (origin, task) = runtime_server(f.auth.clone()).await;
    let human = Identity::new(f.pool.clone())
        .login("admin", "administrator password")
        .await
        .unwrap();
    assert!(try_channel(&origin, &human.token).await.is_err());
    let mut channel = open_channel(&origin, &session.token).await;
    let drain = delivery(&mut channel).await;
    assert_eq!(drain["type"], "drain");
    report(
        &mut channel,
        json!({"type":"drained","request_id":drain["request_id"]}),
    )
    .await;
    assert_eq!(delivery(&mut channel).await, json!({"type":"revoked"}));
    let acknowledged: bool = sqlx::query_scalar(
        "SELECT acknowledged_at IS NOT NULL FROM agent_runtime_drains WHERE attempt_id = $1",
    )
    .bind(f.attempt)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert!(acknowledged);
    task.abort();
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn actual_native_runtime_acknowledges_a_persisted_drain_and_exits() {
    use cubebot_orchestrator::agents::LifecycleAction;
    use std::os::unix::fs::DirBuilderExt;
    let f = fixture().await;
    lifecycle(&f, LifecycleAction::Sleep, 2).await;
    let (origin, task) = runtime_server(f.auth.clone()).await;
    let root = std::env::temp_dir().join(format!("cubebot-native-pg-{}", Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    let token = root.join("token");
    std::fs::write(&token, "projected-native-token").unwrap();
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("cubebot-agent");
    let mut child = std::process::Command::new(binary)
        .env("CUBEBOT_ORCHESTRATOR_ORIGIN", origin)
        .env("CUBEBOT_BOOTSTRAP_TOKEN_FILE", token)
        .env("CUBEBOT_JOURNAL_DIRECTORY", &root)
        .spawn()
        .expect("build the workspace before integration");
    let status = native_exit(&mut child).await;
    assert!(status.success());
    let reported: bool = sqlx::query_scalar(
        "SELECT acknowledged_at IS NOT NULL FROM agent_runtime_drains WHERE attempt_id = $1",
    )
    .bind(f.attempt)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert!(reported);
    let phase: String = sqlx::query_scalar("SELECT phase FROM agent_resources WHERE id = $1")
        .bind(f.attempt)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(phase, "bound");
    assert!(root.join("tools/state").is_file());
    std::fs::remove_dir_all(root).unwrap();
    task.abort();
    f.task.abort();
}
async fn native_exit(child: &mut std::process::Child) -> std::process::ExitStatus {
    for _ in 0..250 {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    child.kill().unwrap();
    child.wait().unwrap();
    panic!("native runtime did not complete persisted drain");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn incomplete_observation_holds_active_identity_without_destroying_recovery() {
    let f = fixture().await;
    let session = f
        .auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    let registry = Registry::new(f.pool.clone());
    let original = f.api.lock().unwrap().pod.clone();
    let mut incomplete = original.clone();
    incomplete.status = None;
    registry
        .bind_object(f.attempt, &ResourceObject::Pod(Box::new(incomplete)))
        .await
        .unwrap();
    assert!(
        f.auth
            .authenticate(&session.token, RuntimeAccess::Active)
            .await
            .is_err()
    );
    assert!(
        f.auth
            .authenticate(&session.token, RuntimeAccess::Recovery)
            .await
            .is_ok()
    );
    let control = f.auth.control(&session.token).await.unwrap();
    assert_eq!(
        serde_json::to_value(control).unwrap()["disposition"],
        "hold"
    );
    registry
        .bind_object(f.attempt, &ResourceObject::Pod(Box::new(original)))
        .await
        .unwrap();
    assert!(
        f.auth
            .authenticate(&session.token, RuntimeAccess::Active)
            .await
            .is_ok()
    );
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn partial_runtime_failure_latches_drain_even_when_the_pod_phase_still_says_running() {
    let f = fixture().await;
    let session = f
        .auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    let registry = Registry::new(f.pool.clone());
    let original = f.api.lock().unwrap().pod.clone();
    let mut failed = original.clone();
    failed.status = Some(serde_json::from_value(json!({"phase":"Running","containerStatuses":[{"name":"sandbox","image":"test","imageID":"test","ready":false,"restartCount":0,"state":{"terminated":{"exitCode":1,"reason":"Error"}}}]})).unwrap());
    registry
        .bind_object(f.attempt, &ResourceObject::Pod(Box::new(failed)))
        .await
        .unwrap();
    assert!(
        f.auth
            .authenticate(&session.token, RuntimeAccess::Active)
            .await
            .is_err()
    );
    assert!(
        f.auth
            .authenticate(&session.token, RuntimeAccess::Recovery)
            .await
            .is_ok()
    );
    assert!(
        f.auth
            .control(&session.token)
            .await
            .unwrap()
            .drain
            .is_some()
    );
    registry
        .bind_object(f.attempt, &ResourceObject::Pod(Box::new(original)))
        .await
        .unwrap();
    assert!(
        f.auth
            .authenticate(&session.token, RuntimeAccess::Active)
            .await
            .is_err()
    );
    assert!(
        f.auth
            .control(&session.token)
            .await
            .unwrap()
            .drain
            .is_some()
    );
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn human_status_http_reports_observed_facts_without_runtime_secrets_or_working_claims() {
    let f = fixture().await;
    let runtime = f
        .auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    let identity = Identity::new(f.pool.clone());
    let human = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let (origin, task) =
        http_server(cubebot_orchestrator::api::database_router(f.pool.clone())).await;
    let client = openidconnect::reqwest::Client::new();
    let path = format!("{origin}/api/agents/{}/status", f.agent.0);
    assert_eq!(client.get(&path).send().await.unwrap().status(), 401);
    let response = client
        .get(&path)
        .bearer_auth(human.token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = response.bytes().await.unwrap();
    let status: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status["desired_state"], "running");
    assert_eq!(status["resource_phase"], "bound");
    assert_eq!(status["kubernetes_phase"], "Running");
    assert_eq!(status["runtime_available"], true);
    assert_eq!(status["kubernetes_ready"], false);
    assert!(status["observed_at"].is_string());
    assert!(status["observed_seconds_ago"].is_number());
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(!body.contains(&runtime.token));
    assert!(!body.contains("token_hash"));
    assert!(!body.contains("manifest"));
    assert!(!body.contains("instructions"));
    task.abort();
    f.task.abort();
}

struct TextProvider {
    fail_first: bool,
    started: tokio::sync::Notify,
    finish: tokio::sync::Notify,
    calls: std::sync::atomic::AtomicUsize,
}
async fn text_reply(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<TextProvider>>,
    axum::Json(body): axum::Json<Value>,
) -> axum::response::Response {
    use axum::{http::StatusCode, response::IntoResponse};
    // An immediate retry-after keeps bounded retries fast without changing their count.
    let failure = |status| (status, [("retry-after", "0")], "controlled failure").into_response();
    assert_eq!(body["model"], "controlled-model");
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    let call = state
        .calls
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let input = body["input"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap();
    if input == "FAIL_PROVIDER" || (state.fail_first && call == 0) {
        return failure(StatusCode::SERVICE_UNAVAILABLE);
    }
    if input == "REJECT_PROVIDER" {
        return failure(StatusCode::UNAUTHORIZED);
    }
    state.started.notify_one();
    state.finish.notified().await;
    let completed = json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"Durable native reply"}]}]}});
    let delta = json!({"type":"response.output_text.delta","delta":"Durable native reply"});
    (
        [("content-type", "text/event-stream")],
        format!("data: {delta}\n\ndata: {completed}\n\n"),
    )
        .into_response()
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn native_text_turn_completes_through_the_channel_with_recorded_events() {
    native_text(false).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn native_transient_failure_retries_once_with_the_same_model() {
    native_text(true).await;
}

async fn native_text(first_failure: bool) {
    let f = fixture().await;
    let (auth, work, admin, conversation) = native_chat_setup(&f, "Hello").await;
    let (origin, server) = runtime_server(auth).await;
    let state = std::sync::Arc::new(TextProvider {
        fail_first: first_failure,
        started: tokio::sync::Notify::new(),
        finish: tokio::sync::Notify::new(),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let provider_router = Router::new()
        .route("/responses", axum::routing::post(text_reply))
        .with_state(state.clone());
    let (provider_origin, provider) = http_server(provider_router).await;
    let child = spawn_native_text(&origin, &provider_origin);
    tokio::time::timeout(std::time::Duration::from_secs(10), state.started.notified())
        .await
        .unwrap();
    assert_eq!(
        work.history(&admin, conversation, 0).await.unwrap()[0].status,
        "running"
    );
    state.finish.notify_one();
    wait_native_response(&work, &admin, conversation).await;
    assert_eq!(
        state.calls.load(std::sync::atomic::Ordering::SeqCst),
        1 + usize::from(first_failure)
    );
    drop(child);
    let history = work.history(&admin, conversation, 0).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].response.as_deref(), Some("Durable native reply"));
    let kinds: Vec<String> = sqlx::query_scalar(
        "SELECT payload->>'kind' FROM turn_events WHERE turn_id = $1 ORDER BY sequence",
    )
    .bind(history[0].id)
    .fetch_all(&f.pool)
    .await
    .unwrap();
    // The fixture reports no usage, which is never recorded as zero.
    assert_eq!(kinds, ["text_delta", "done"]);
    server.abort();
    provider.abort();
    f.task.abort();
}
async fn wait_native_response(
    work: &cubebot_orchestrator::work::Work,
    admin: &cubebot_orchestrator::identity::Actor,
    conversation: Uuid,
) {
    for _ in 0..150 {
        if work.history(admin, conversation, 0).await.unwrap()[0]
            .response
            .is_some()
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("native journal result was not committed after restart");
}

struct NativeChild {
    process: std::process::Child,
    root: std::path::PathBuf,
}
impl Drop for NativeChild {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

async fn native_chat_setup(
    f: &Fixture,
    message: &str,
) -> (
    RuntimeIdentity,
    cubebot_orchestrator::work::Work,
    cubebot_orchestrator::identity::Actor,
    Uuid,
) {
    use cubebot_orchestrator::{
        connections::{ConfigureProvider, Connections, Credentials},
        work::{SendMessage, Work},
    };
    let identity = Identity::new(f.pool.clone());
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    Connections::new(f.pool.clone(), Credentials::new([7; 32]))
        .configure(
            &admin,
            &ConfigureProvider {
                request_id: Uuid::new_v4(),
                expected_revision: 0,
                provider: cubebot_protocol::runtime::ProviderKind::Openai,
                model: "controlled-model".into(),
                context_window: Some(128_000),
                api_key: "controlled-key".into(),
                access: None,
            },
        )
        .await
        .unwrap();
    let work = Work::new(f.pool.clone());
    let conversation = work.conversation(&admin, f.agent).await.unwrap();
    work.send(
        &admin,
        conversation.id,
        &SendMessage {
            request_id: Uuid::new_v4(),
            message: message.into(),
        },
    )
    .await
    .unwrap();
    (
        f.auth.clone().with_credentials(Credentials::new([7; 32])),
        work,
        admin,
        conversation.id,
    )
}
fn spawn_native_text(origin: &str, provider_origin: &str) -> NativeChild {
    use std::os::unix::fs::DirBuilderExt;
    let root = std::env::temp_dir().join(format!("cubebot-native-text-{}", Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    let token = root.join("token");
    std::fs::write(&token, "projected-native-token").unwrap();
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("cubebot-agent");
    let process = std::process::Command::new(binary)
        .env("CUBEBOT_ORCHESTRATOR_ORIGIN", origin)
        .env("CUBEBOT_BOOTSTRAP_TOKEN_FILE", token)
        .env("CUBEBOT_JOURNAL_DIRECTORY", &root)
        .env(
            "CUBEBOT_TEST_PROVIDER_ORIGIN",
            format!("{provider_origin}/"),
        )
        .spawn()
        .unwrap();
    NativeChild { process, root }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn native_repeated_transient_failure_waits_after_bounded_attempts_and_manual_retry_is_idempotent()
 {
    native_failure("FAIL_PROVIDER", 4).await;
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn native_provider_authentication_rejection_does_not_retry_automatically() {
    native_failure("REJECT_PROVIDER", 1).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn native_lost_provider_response_waits_without_replaying_an_accepted_request() {
    let f = fixture().await;
    let (auth, work, admin, conversation) = native_chat_setup(&f, "Lost response").await;
    let (origin, server) = runtime_server(auth).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider_origin = format!("http://{}", listener.local_addr().unwrap());
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let accepted = calls.clone();
    let provider = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            receive_provider_request(socket).await;
            accepted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // The complete request was received; deliberately lose the response.
        }
    });
    let child = spawn_native_text(&origin, &provider_origin);
    let before = wait_native_waiting(&work, &admin, conversation).await;
    assert_eq!(before.error.as_deref(), Some("provider_unavailable"));
    assert!(before.response.is_none());
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    work.retry(
        &admin,
        conversation,
        before.id,
        &cubebot_orchestrator::work::RetryMessage {
            request_id: Uuid::new_v4(),
        },
    )
    .await
    .unwrap();
    let after = wait_native_waiting(&work, &admin, conversation).await;
    assert_eq!(before.id, after.id);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(
        work.history(&admin, conversation, 0).await.unwrap().len(),
        1
    );
    drop(child);
    server.abort();
    provider.abort();
    f.task.abort();
}

async fn receive_provider_request(mut socket: tokio::net::TcpStream) {
    use tokio::io::AsyncReadExt;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut bytes = Vec::new();
        loop {
            let mut chunk = [0_u8; 4096];
            let count = socket.read(&mut chunk).await.unwrap();
            assert_ne!(count, 0);
            bytes.extend_from_slice(&chunk[..count]);
            assert!(bytes.len() <= 256 * 1024);
            if complete_provider_request(&bytes) {
                return;
            }
        }
    })
    .await
    .unwrap();
}

fn complete_provider_request(bytes: &[u8]) -> bool {
    let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let headers = std::str::from_utf8(&bytes[..end]).unwrap();
    let length: usize = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .unwrap()
        .1
        .trim()
        .parse()
        .unwrap();
    bytes.len() >= end + 4 + length
}

async fn native_failure(message: &str, expected: usize) {
    let f = fixture().await;
    let (auth, work, admin, conversation) = native_chat_setup(&f, message).await;
    let (origin, server) = runtime_server(auth).await;
    let state = std::sync::Arc::new(TextProvider {
        fail_first: false,
        started: tokio::sync::Notify::new(),
        finish: tokio::sync::Notify::new(),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let router = Router::new()
        .route("/responses", axum::routing::post(text_reply))
        .with_state(state.clone());
    let (provider_origin, provider) = http_server(router).await;
    let child = spawn_native_text(&origin, &provider_origin);
    let before = wait_native_waiting(&work, &admin, conversation).await;
    assert_eq!(
        state.calls.load(std::sync::atomic::Ordering::SeqCst),
        expected
    );
    let retry = cubebot_orchestrator::work::RetryMessage {
        request_id: Uuid::new_v4(),
    };
    work.retry(&admin, conversation, before.id, &retry)
        .await
        .unwrap();
    work.retry(&admin, conversation, before.id, &retry)
        .await
        .unwrap();
    let after = wait_native_waiting(&work, &admin, conversation).await;
    assert_eq!(before.id, after.id);
    assert_eq!(
        state.calls.load(std::sync::atomic::Ordering::SeqCst),
        expected * 2
    );
    assert_eq!(
        work.history(&admin, conversation, 0).await.unwrap().len(),
        1
    );
    drop(child);
    server.abort();
    provider.abort();
    f.task.abort();
}
async fn wait_native_waiting(
    work: &cubebot_orchestrator::work::Work,
    admin: &cubebot_orchestrator::identity::Actor,
    conversation: Uuid,
) -> cubebot_orchestrator::work::Turn {
    for _ in 0..250 {
        let turn = work
            .history(admin, conversation, 0)
            .await
            .unwrap()
            .pop()
            .unwrap();
        if turn.status == "waiting" {
            return turn;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("native provider failure did not enter durable waiting");
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn try_channel(
    origin: &str,
    token: &str,
) -> Result<Socket, tokio_tungstenite::tungstenite::Error> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = format!("{}/api/runtime/channel", origin.replacen("http", "ws", 1))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(socket, _)| socket)
}
async fn open_channel(origin: &str, token: &str) -> Socket {
    try_channel(origin, token).await.unwrap()
}
async fn report(channel: &mut Socket, value: Value) {
    use futures_util::SinkExt;
    channel
        .send(tokio_tungstenite::tungstenite::Message::Text(
            value.to_string().into(),
        ))
        .await
        .unwrap();
}
/// The next delivery, or `null` once the orchestrator closes the channel.
async fn delivery(channel: &mut Socket) -> Value {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    loop {
        let next = tokio::time::timeout(std::time::Duration::from_secs(10), channel.next())
            .await
            .expect("channel delivery");
        match next {
            Some(Ok(Message::Text(text))) => return serde_json::from_str(&text).unwrap(),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => (),
            _ => return Value::Null,
        }
    }
}
async fn event(channel: &mut Socket, turn: &Value, sequence: u64, event: Value) {
    report(
        channel,
        json!({"type":"event","turn":turn,"sequence":sequence,"event":event}),
    )
    .await;
    assert_eq!(
        delivery(channel).await,
        json!({"type":"ack","turn":turn,"sequence":sequence})
    );
}
async fn send(
    work: &cubebot_orchestrator::work::Work,
    admin: &cubebot_orchestrator::identity::Actor,
    conversation: Uuid,
    message: &str,
) {
    work.send(
        admin,
        conversation,
        &cubebot_orchestrator::work::SendMessage {
            request_id: Uuid::new_v4(),
            message: message.into(),
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn channel_runs_one_turn_at_a_time_and_settles_it_once_from_idempotent_events() {
    let f = fixture().await;
    let (auth, work, admin, conversation) = native_chat_setup(&f, "First").await;
    send(&work, &admin, conversation, "Second").await;
    let session = auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    let (origin, server) = runtime_server(auth).await;
    assert!(try_channel(&origin, "cb_runtime_unknown").await.is_err());
    let mut channel = open_channel(&origin, &session.token).await;
    report(&mut channel, json!({"type":"pull"})).await;
    let first = delivery(&mut channel).await;
    assert_eq!(first["type"], "turn");
    assert_eq!(first["message"], "First");
    assert_eq!(first["sequence"], 1);
    assert_eq!(first["provider"], "openai");
    assert_eq!(first["model"], "controlled-model");
    assert_eq!(first["context_window"], 128_000);
    assert_eq!(first["summary"], Value::Null);
    assert_eq!(first["api_key"], "controlled-key");
    assert_eq!(first["history"], json!([]));
    let turn = first["turn"].clone();
    let attempt: Option<Uuid> =
        sqlx::query_scalar("SELECT attempt_id FROM chat_turns WHERE id = $1")
            .bind(Uuid::parse_str(turn.as_str().unwrap()).unwrap())
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(attempt, Some(f.attempt.0));
    // A second pull while a turn is active is not another turn.
    report(&mut channel, json!({"type":"pull"})).await;
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert_eq!(
        work.history(&admin, conversation, 0).await.unwrap()[1].status,
        "queued"
    );

    event(
        &mut channel,
        &turn,
        1,
        json!({"kind":"text_delta","text":"Ans"}),
    )
    .await;
    event(
        &mut channel,
        &turn,
        1,
        json!({"kind":"text_delta","text":"resent differently"}),
    )
    .await;
    let call = Uuid::new_v4();
    event(
        &mut channel,
        &turn,
        2,
        json!({"kind":"tool_call","id":call,"name":"terminal","input":{"command":"touch x"}}),
    )
    .await;
    let running = work.history(&admin, conversation, 0).await.unwrap();
    assert_eq!(running[0].terminal[0].status, "running");
    assert_eq!(running[0].terminal[0].command, "touch x");
    event(
        &mut channel,
        &turn,
        3,
        json!({"kind":"done","response":"Answer"}),
    )
    .await;
    // The pull still pending from earlier now receives the next turn.
    let second = delivery(&mut channel).await;
    assert_eq!(second["message"], "Second");
    assert_eq!(
        second["history"],
        json!([{"message":"First","response":"Answer"}])
    );
    event(
        &mut channel,
        &turn,
        4,
        json!({"kind":"error","code":"provider_unavailable"}),
    )
    .await;
    let settled = work.history(&admin, conversation, 0).await.unwrap();
    assert_eq!(settled[0].status, "complete");
    assert_eq!(settled[0].response.as_deref(), Some("Answer"));
    // The run ended without the call's result, so its effects are uncertain.
    assert_eq!(settled[0].terminal[0].status, "uncertain");
    let delta: Value =
        sqlx::query_scalar("SELECT payload FROM turn_events WHERE turn_id = $1 AND sequence = 1")
            .bind(settled[0].id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(delta["text"], "Ans");
    // An event for a turn this agent does not own closes the channel.
    report(
        &mut channel,
        json!({"type":"event","turn":Uuid::new_v4(),"sequence":1,"event":{"kind":"done","response":"x"}}),
    )
    .await;
    assert_eq!(delivery(&mut channel).await, Value::Null);
    server.abort();
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn cancel_and_revocation_are_messages_on_the_channel() {
    let f = fixture().await;
    let (auth, work, admin, conversation) = native_chat_setup(&f, "Long work").await;
    let session = auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    let (origin, server) = runtime_server(auth).await;
    let mut channel = open_channel(&origin, &session.token).await;
    report(&mut channel, json!({"type":"pull"})).await;
    let turn = delivery(&mut channel).await["turn"].clone();
    let id = Uuid::parse_str(turn.as_str().unwrap()).unwrap();
    work.cancel(&admin, conversation, id).await.unwrap();
    assert_eq!(
        delivery(&mut channel).await,
        json!({"type":"cancel","turn":turn})
    );
    event(
        &mut channel,
        &turn,
        1,
        json!({"kind":"error","code":"cancelled"}),
    )
    .await;
    assert_eq!(
        work.history(&admin, conversation, 0).await.unwrap()[0].status,
        "cancelled"
    );
    sqlx::query(
        "UPDATE agent_resources SET phase = 'stopped',termination_proof = '{}' WHERE id = $1",
    )
    .bind(f.attempt)
    .execute(&f.pool)
    .await
    .unwrap();
    assert_eq!(delivery(&mut channel).await, json!({"type":"revoked"}));
    assert_eq!(delivery(&mut channel).await, Value::Null);
    assert!(try_channel(&origin, &session.token).await.is_err());
    server.abort();
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn uncertain_tool_effects_block_retry_until_an_outcome_is_reported() {
    use cubebot_orchestrator::work::RetryMessage;
    let f = fixture().await;
    let (auth, work, admin, conversation) = native_chat_setup(&f, "Run it").await;
    let session = auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    let (origin, server) = runtime_server(auth).await;
    let mut channel = open_channel(&origin, &session.token).await;
    report(&mut channel, json!({"type":"pull"})).await;
    let turn = delivery(&mut channel).await["turn"].clone();
    let id = Uuid::parse_str(turn.as_str().unwrap()).unwrap();
    let call = Uuid::new_v4();
    event(
        &mut channel,
        &turn,
        1,
        json!({"kind":"tool_call","id":call,"name":"terminal","input":{"command":"rm data"}}),
    )
    .await;
    event(
        &mut channel,
        &turn,
        2,
        json!({"kind":"error","code":"provider_unavailable"}),
    )
    .await;
    let waiting = work.history(&admin, conversation, 0).await.unwrap();
    assert_eq!(waiting[0].status, "waiting");
    assert_eq!(waiting[0].terminal[0].status, "uncertain");
    let retry = RetryMessage {
        request_id: Uuid::new_v4(),
    };
    assert!(work.retry(&admin, conversation, id, &retry).await.is_err());
    // A recovered sandbox reports the outcome it journaled before the crash.
    let outcome = json!({"status":"exited","code":0,"output":"removed"});
    event(
        &mut channel,
        &turn,
        3,
        json!({"kind":"tool_result","id":call,"outcome":outcome}),
    )
    .await;
    assert_eq!(
        work.history(&admin, conversation, 0).await.unwrap()[0].terminal[0].status,
        "exited"
    );
    work.retry(&admin, conversation, id, &retry).await.unwrap();
    report(&mut channel, json!({"type":"pull"})).await;
    let again = delivery(&mut channel).await;
    assert_eq!(again["turn"], turn);
    assert_eq!(again["sequence"], 4);
    assert_eq!(again["prior"][0]["kind"], "tool_call");
    assert_eq!(again["prior"][1]["outcome"], outcome);
    server.abort();
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_lost_sandbox_interrupts_its_running_turn() {
    let f = fixture().await;
    let (auth, work, admin, conversation) = native_chat_setup(&f, "Interrupted").await;
    let session = auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    let (origin, server) = runtime_server(auth).await;
    let mut channel = open_channel(&origin, &session.token).await;
    report(&mut channel, json!({"type":"pull"})).await;
    delivery(&mut channel).await;
    work.recover_interrupted().await.unwrap();
    assert_eq!(
        work.history(&admin, conversation, 0).await.unwrap()[0].status,
        "running"
    );
    sqlx::query(
        "UPDATE agent_resources SET phase = 'stopped',termination_proof = '{}' WHERE id = $1",
    )
    .bind(f.attempt)
    .execute(&f.pool)
    .await
    .unwrap();
    work.recover_interrupted().await.unwrap();
    let turn = work
        .history(&admin, conversation, 0)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(turn.status, "waiting");
    assert_eq!(turn.error.as_deref(), Some("execution_interrupted"));
    server.abort();
    f.task.abort();
}

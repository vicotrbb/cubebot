#[path = "support/registry.rs"]
mod registry_support;
#[path = "support/runtime.rs"]
mod runtime_support;
mod support;
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
async fn runtime_sessions_are_finite_rotatable_and_separate_from_human_credentials() {
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
    assert_eq!(first.expires_in_seconds, 120);
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
    sqlx::query("UPDATE agent_runtime_sessions SET expires_at = clock_timestamp() - interval '1 second', created_at = clock_timestamp() - interval '2 minutes'").execute(&f.pool).await.unwrap();
    assert!(
        f.auth
            .authenticate(&second.token, RuntimeAccess::Recovery)
            .await
            .is_err()
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
async fn drain_reports_recheck_expiry_after_waiting_for_lifecycle_lock() {
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
    sqlx::query("UPDATE agent_runtime_sessions SET created_at = clock_timestamp() - interval '3 minutes', expires_at = clock_timestamp() - interval '1 second' WHERE attempt_id = $1").bind(f.attempt).execute(&mut *transaction).await.unwrap();
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
async fn drain_http_workflow_requires_machine_identity_and_retains_acknowledgement() {
    use cubebot_orchestrator::agents::LifecycleAction;
    let f = fixture().await;
    let session = f
        .auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    lifecycle(&f, LifecycleAction::Retire, 2).await;
    let (origin, task) = runtime_server(f.auth.clone()).await;
    let client = openidconnect::reqwest::Client::new();
    let response = client
        .get(format!("{origin}/api/runtime/control"))
        .bearer_auth(&session.token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let control: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(control["disposition"], "drain");
    let body = json!({"request_id":control["drain"]["request_id"]});
    let identity = Identity::new(f.pool.clone());
    let human = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let denied = client
        .post(format!("{origin}/api/runtime/drained"))
        .bearer_auth(human.token)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);
    let accepted = client
        .post(format!("{origin}/api/runtime/drained"))
        .bearer_auth(&session.token)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), 200);
    assert_eq!(accepted.headers()["cache-control"], "no-store");
    let result: Value = serde_json::from_slice(&accepted.bytes().await.unwrap()).unwrap();
    assert_eq!(result["acknowledged"], true);
    task.abort();
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn actual_native_runtime_synchronizes_retained_evidence_before_persisted_drain() {
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
    seed_native_journal(&root);
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
    assert!(root.join("journal/state").is_file());
    assert!(!root.join("journal/0000000000000001.entry").exists());
    let retained: Vec<u8> = sqlx::query_scalar(
        "SELECT payload FROM agent_journal_records WHERE agent_id = $1 AND sequence = 1",
    )
    .bind(f.agent)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(retained, vec![255u8; 65536]);

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
    failed.status = Some(serde_json::from_value(json!({"phase":"Running","containerStatuses":[{"name":"runtime","image":"test","imageID":"test","ready":false,"restartCount":0,"state":{"terminated":{"exitCode":1,"reason":"Error"}}}]})).unwrap());
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

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn journal_receipts_are_contiguous_immutable_and_hold_active_admission() {
    use cubebot_protocol::runtime::JournalRecord;
    let f = fixture().await;
    let instance = Uuid::new_v4();
    let session = f.auth.bootstrap("projected-token", instance).await.unwrap();
    let first = JournalRecord {
        sequence: 1,
        payload: b"unresolved dispatch".to_vec(),
    };
    assert!(
        f.auth
            .retain_journal(
                &session.token,
                &JournalRecord {
                    sequence: 2,
                    payload: vec![]
                }
            )
            .await
            .is_err()
    );
    let (accepted, repeated) = tokio::join!(
        f.auth.retain_journal(&session.token, &first),
        f.auth.retain_journal(&session.token, &first)
    );
    let receipt = accepted.unwrap();
    assert!(repeated.is_ok());
    assert_eq!(receipt.sequence, 1);
    assert_eq!(receipt.actor.agent, f.agent.0);
    let replay = f.auth.retain_journal(&session.token, &first).await.unwrap();
    assert_eq!(receipt.payload_digest, replay.payload_digest);
    assert!(
        f.auth
            .retain_journal(
                &session.token,
                &JournalRecord {
                    sequence: 1,
                    payload: b"changed".to_vec()
                }
            )
            .await
            .is_err()
    );
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
    let next = f.auth.bootstrap("projected-token", instance).await.unwrap();
    assert!(f.auth.retain_journal(&session.token, &first).await.is_err());
    f.auth.retain_journal(&next.token, &first).await.unwrap();
    f.auth
        .retain_journal(
            &next.token,
            &JournalRecord {
                sequence: 2,
                payload: b"known local outcome".to_vec(),
            },
        )
        .await
        .unwrap();
    let retained: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_journal_records WHERE agent_id = $1")
            .bind(f.agent)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(retained, 2);
    f.task.abort();
}

// Independent on-disk fixture exercises the native binary without an app-to-app dependency.
fn seed_native_journal(root: &std::path::Path) {
    use std::os::unix::fs::DirBuilderExt;
    let path = root.join("journal");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    let mut state = b"CBJNL001".to_vec();
    state.extend(1u64.to_be_bytes());
    state.extend(0u64.to_be_bytes());
    write_sealed(&path.join("state"), state);
    let mut record = 1u64.to_be_bytes().to_vec();
    record.extend(65536u32.to_be_bytes());
    record.extend(vec![255u8; 65536]);
    write_sealed(&path.join("0000000000000001.entry"), record);
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path.join("lock"))
        .unwrap();
}
fn write_sealed(path: &std::path::Path, mut bytes: Vec<u8>) {
    use sha2::{Digest, Sha256};
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    bytes.extend(Sha256::digest(&bytes));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(&bytes).unwrap();
    file.sync_all().unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn journal_http_accepts_bounded_evidence_and_rejects_human_or_malformed_reports() {
    let f = fixture().await;
    let session = f.auth.bootstrap("projected", Uuid::new_v4()).await.unwrap();
    let human = Identity::new(f.pool.clone())
        .login("admin", "administrator password")
        .await
        .unwrap();
    let (origin, task) = runtime_server(f.auth.clone()).await;
    let client = openidconnect::reqwest::Client::new();
    let url = format!("{origin}/api/runtime/journal");
    let body = json!({"sequence":1,"payload":vec![255u8;65536]});
    let denied = client
        .post(&url)
        .bearer_auth(human.token)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);
    assert_eq!(denied.headers()["cache-control"], "no-store");
    let accepted = client
        .post(&url)
        .bearer_auth(&session.token)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), 200);
    assert_eq!(accepted.headers()["cache-control"], "no-store");
    assert!(accepted.bytes().await.unwrap().len() < 1024);
    for (body, status) in [
        (json!({"sequence":0,"payload":[]}), 400),
        (json!({"sequence":2,"payload":vec![0;65537]}), 400),
        (json!({"sequence":2,"payload":[],"agent":f.agent}), 422),
    ] {
        let response = client
            .post(&url)
            .bearer_auth(&session.token)
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
    let oversized = client
        .post(&url)
        .bearer_auth(&session.token)
        .header("content-type", "application/json")
        .body(" ".repeat(272 * 1024 + 1))
        .send()
        .await
        .unwrap();
    assert_eq!(oversized.status(), 413);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_journal_records WHERE agent_id = $1")
            .bind(f.agent)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    task.abort();
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn journal_reports_recheck_expiry_after_waiting_for_lifecycle_lock() {
    use cubebot_protocol::runtime::JournalRecord;
    let f = fixture().await;
    let session = f.auth.bootstrap("projected", Uuid::new_v4()).await.unwrap();
    let mut transaction = f.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM agents WHERE id = $1 FOR UPDATE")
        .bind(f.agent)
        .execute(&mut *transaction)
        .await
        .unwrap();
    let auth = f.auth.clone();
    let report = tokio::spawn(async move {
        auth.retain_journal(
            &session.token,
            &JournalRecord {
                sequence: 1,
                payload: b"evidence".to_vec(),
            },
        )
        .await
    });
    wait_for_runtime_lock(&f.pool).await;
    sqlx::query("UPDATE agent_runtime_sessions SET created_at = clock_timestamp() - interval '3 minutes',expires_at = clock_timestamp() - interval '1 second' WHERE attempt_id = $1").bind(f.attempt).execute(&mut *transaction).await.unwrap();
    transaction.commit().await.unwrap();
    assert!(report.await.unwrap().is_err());
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_journal_records WHERE agent_id = $1")
            .bind(f.agent)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn text_grants_are_single_use_and_recovered_results_commit_once() {
    use cubebot_orchestrator::{
        connections::{ConfigureProvider, Connections, Credentials},
        work::{SendMessage, Work},
    };
    use cubebot_protocol::chat::{ChatEvidence, Outcome};
    let f = fixture().await;
    let identity = Identity::new(f.pool.clone());
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    let connections = Connections::new(f.pool.clone(), Credentials::new([7; 32]));
    connections
        .configure(
            &admin,
            &ConfigureProvider {
                request_id: Uuid::new_v4(),
                expected_revision: 0,
                model: "test-model".into(),
                api_key: "test-secret".into(),
                access: None,
            },
        )
        .await
        .unwrap();
    let auth = f.auth.clone().with_credentials(Credentials::new([7; 32]));
    let work = Work::new(f.pool.clone());
    let conversation = work.conversation(&admin, f.agent).await.unwrap();
    work.send(
        &admin,
        conversation.id,
        &SendMessage {
            request_id: Uuid::new_v4(),
            message: "Hello".into(),
        },
    )
    .await
    .unwrap();
    let session = auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    let grant = auth.claim_text(&session.token).await.unwrap().unwrap();
    assert_eq!(grant.api_key, "test-secret");
    assert_eq!(grant.messages.last().unwrap().content, "Hello");
    assert!(grant.expires_in_seconds <= 60);
    assert!(auth.claim_text(&session.token).await.unwrap().is_none());
    let evidence = ChatEvidence::TextOutcome {
        turn: grant.turn,
        authority: grant.authority,
        outcome: Outcome::Complete {
            response: "Hello back".into(),
        },
    };
    let record = cubebot_protocol::runtime::JournalRecord {
        sequence: 1,
        payload: serde_json::to_vec(&evidence).unwrap(),
    };
    auth.retain_journal(&session.token, &record).await.unwrap();
    auth.retain_journal(&session.token, &record).await.unwrap();
    let history = work.history(&admin, conversation.id, 0).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].response.as_deref(), Some("Hello back"));
    assert!(
        auth.authenticate(&session.token, RuntimeAccess::Active)
            .await
            .is_ok()
    );
    connections
        .configure(
            &admin,
            &ConfigureProvider {
                request_id: Uuid::new_v4(),
                expected_revision: 1,
                model: "test-model".into(),
                api_key: "test-secret".into(),
                access: Some(
                    cubebot_orchestrator::connections::ProviderAccess::ExplicitGrants {
                        agents: vec![],
                    },
                ),
            },
        )
        .await
        .unwrap();
    work.send(
        &admin,
        conversation.id,
        &SendMessage {
            request_id: Uuid::new_v4(),
            message: "Restricted".into(),
        },
    )
    .await
    .unwrap();
    assert!(auth.claim_text(&session.token).await.unwrap().is_none());
    let blocked = work
        .history(&admin, conversation.id, 0)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(blocked.status, "waiting");
    assert!(blocked.authority_id.is_none());
    connections
        .configure(
            &admin,
            &ConfigureProvider {
                request_id: Uuid::new_v4(),
                expected_revision: 2,
                model: "test-model".into(),
                api_key: "test-secret".into(),
                access: Some(
                    cubebot_orchestrator::connections::ProviderAccess::ExplicitGrants {
                        agents: vec![f.agent.0],
                    },
                ),
            },
        )
        .await
        .unwrap();
    let retry = cubebot_orchestrator::work::RetryMessage {
        request_id: Uuid::new_v4(),
        expected_authority: None,
    };
    work.retry(&admin, conversation.id, blocked.id, &retry)
        .await
        .unwrap();
    let issued = auth.claim_text(&session.token).await.unwrap().unwrap();
    work.retry(&admin, conversation.id, blocked.id, &retry)
        .await
        .unwrap();
    assert!(auth.claim_text(&session.token).await.unwrap().is_none());
    sqlx::query(
        "UPDATE chat_authorities SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(issued.authority)
    .execute(&f.pool)
    .await
    .unwrap();
    work.recover_expired().await.unwrap();
    assert_eq!(
        work.history(&admin, conversation.id, 0).await.unwrap()[1]
            .error
            .as_deref(),
        Some("execution_interrupted")
    );
    verify_stale_text_outcomes(
        &auth,
        &session.token,
        &work,
        &admin,
        conversation.id,
        &issued,
    )
    .await;
    f.task.abort();
}

async fn verify_stale_text_outcomes(
    auth: &RuntimeIdentity,
    token: &str,
    work: &cubebot_orchestrator::work::Work,
    admin: &cubebot_orchestrator::identity::Actor,
    conversation: Uuid,
    expired: &cubebot_protocol::chat::InferenceGrant,
) {
    use cubebot_protocol::{
        chat::{ChatEvidence, Outcome},
        runtime::JournalRecord,
    };
    work.retry(
        admin,
        conversation,
        expired.turn,
        &cubebot_orchestrator::work::RetryMessage {
            request_id: Uuid::new_v4(),
            expected_authority: Some(expired.authority),
        },
    )
    .await
    .unwrap();
    let current = auth.claim_text(token).await.unwrap().unwrap();
    assert_ne!(current.authority, expired.authority);
    let stale = JournalRecord {
        sequence: 2,
        payload: serde_json::to_vec(&ChatEvidence::TextOutcome {
            turn: expired.turn,
            authority: expired.authority,
            outcome: Outcome::Complete {
                response: "stale response".into(),
            },
        })
        .unwrap(),
    };
    auth.retain_journal(token, &stale).await.unwrap();
    let pending = work
        .history(admin, conversation, 0)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(pending.authority_id, Some(current.authority));
    assert_eq!(pending.status, "running");
    assert!(pending.response.is_none());
    let committed = JournalRecord {
        sequence: 3,
        payload: serde_json::to_vec(&ChatEvidence::TextOutcome {
            turn: current.turn,
            authority: current.authority,
            outcome: Outcome::Complete {
                response: "current response".into(),
            },
        })
        .unwrap(),
    };
    auth.retain_journal(token, &committed).await.unwrap();
    auth.retain_journal(token, &committed).await.unwrap();
    auth.retain_journal(token, &stale).await.unwrap();
    let final_turn = work
        .history(admin, conversation, 0)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(final_turn.status, "complete");
    assert_eq!(final_turn.response.as_deref(), Some("current response"));
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
) -> (axum::http::StatusCode, axum::Json<Value>) {
    assert_eq!(body["model"], "controlled-model");
    assert_eq!(body["store"], false);
    let call = state
        .calls
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let input = body["input"].as_array().unwrap().last().unwrap()["content"]
        .as_str()
        .unwrap();
    if input == "FAIL_PROVIDER" {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({"error":"controlled outage"})),
        );
    }
    if input == "REJECT_PROVIDER" {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(json!({"error":"controlled rejection"})),
        );
    }
    if state.fail_first && call == 0 {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({"error":"transient fixture"})),
        );
    }
    state.started.notify_one();
    state.finish.notified().await;
    (
        axum::http::StatusCode::OK,
        axum::Json(
            json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"Durable native reply"}]}]}),
        ),
    )
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn native_text_result_survives_control_server_restart_without_repeating_inference() {
    native_recovery(false).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn native_transient_failure_retries_once_with_durable_progress_and_the_same_model() {
    native_recovery(true).await;
}

async fn native_recovery(first_failure: bool) {
    let f = fixture().await;
    let (auth, work, admin, conversation) = native_chat_setup(&f, "Hello").await;
    let (origin, server) = runtime_server(auth.clone()).await;
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
    let mut child = spawn_native_text(&origin, &provider_origin);
    let root = child.root.clone();
    tokio::time::timeout(std::time::Duration::from_secs(10), state.started.notified())
        .await
        .unwrap();
    assert_eq!(
        work.history(&admin, conversation, 0).await.unwrap()[0].status,
        if first_failure { "retrying" } else { "running" }
    );
    let restarted =
        recover_control_server(&origin, auth, server, &state, &root, first_failure).await;
    wait_native_response(&work, &admin, conversation).await;
    assert_eq!(
        state.calls.load(std::sync::atomic::Ordering::SeqCst),
        1 + usize::from(first_failure)
    );
    child.process.kill().unwrap();
    child.process.wait().unwrap();
    let history = work.history(&admin, conversation, 0).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].response.as_deref(), Some("Durable native reply"));
    std::fs::remove_dir_all(root).unwrap();
    restarted.abort();
    provider.abort();
    f.task.abort();
}
async fn wait_native_evidence(root: &std::path::Path, sequence: u64) {
    for _ in 0..100 {
        if root
            .join(format!("journal/{sequence:016x}.entry"))
            .is_file()
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("native response was not journaled during control outage");
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
                model: "controlled-model".into(),
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
async fn restart_runtime_server(
    origin: &str,
    auth: RuntimeIdentity,
) -> tokio::task::JoinHandle<()> {
    let listener = tokio::net::TcpListener::bind(origin.trim_start_matches("http://"))
        .await
        .unwrap();
    tokio::spawn(async move {
        axum::serve(listener, cubebot_orchestrator::api::runtime_router(auth))
            .await
            .unwrap();
    })
}

async fn recover_control_server(
    origin: &str,
    auth: RuntimeIdentity,
    server: tokio::task::JoinHandle<()>,
    state: &TextProvider,
    root: &std::path::Path,
    first_failure: bool,
) -> tokio::task::JoinHandle<()> {
    server.abort();
    let _ = server.await;
    state.finish.notify_one();
    wait_native_evidence(root, if first_failure { 3 } else { 2 }).await;
    restart_runtime_server(origin, auth).await
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
async fn native_repeated_transient_failure_waits_after_two_attempts_and_manual_retry_is_idempotent()
{
    native_failure("FAIL_PROVIDER", 2).await;
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
            expected_authority: before.authority_id,
        },
    )
    .await
    .unwrap();
    let after = wait_native_waiting(&work, &admin, conversation).await;
    assert_ne!(before.authority_id, after.authority_id);
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
        expected_authority: before.authority_id,
    };
    work.retry(&admin, conversation, before.id, &retry)
        .await
        .unwrap();
    work.retry(&admin, conversation, before.id, &retry)
        .await
        .unwrap();
    let after = wait_native_waiting(&work, &admin, conversation).await;
    assert_ne!(before.authority_id, after.authority_id);
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

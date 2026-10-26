#[path = "support/registry.rs"]
mod registry_support;
mod support;
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{Method, Uri},
    routing::any,
};
use cubebot_orchestrator::{
    agents::{
        AgentId,
        kubernetes::{AttemptId, Layout, Registry, ResourceObject, ResourceSpec},
        runtime::{RuntimeAccess, RuntimeIdentity},
    },
    identity::Identity,
};
use k8s_openapi::api::core::v1::Pod;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

struct Api {
    pod: Pod,
    service_uid: String,
}
async fn api(
    State(state): State<Arc<Mutex<Api>>>,
    method: Method,
    uri: Uri,
    bytes: Bytes,
) -> Json<Value> {
    let state = state.lock().unwrap();
    if method == Method::GET {
        return Json(serde_json::to_value(&state.pod).unwrap());
    }
    assert_eq!(uri.path(), "/apis/authentication.k8s.io/v1/tokenreviews");
    let request: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(request["spec"]["audiences"], json!(["cubebot-runtime"]));
    Json(
        json!({"apiVersion":"authentication.k8s.io/v1","kind":"TokenReview","status":{"authenticated":true,"audiences":["cubebot-runtime"],"user":{"username":"system:serviceaccount:cubebot-test:cubebot-runtime","uid":state.service_uid,"extra":{"authentication.kubernetes.io/pod-name":[state.pod.metadata.name],"authentication.kubernetes.io/pod-uid":[state.pod.metadata.uid]}}}}),
    )
}
struct Fixture {
    pool: sqlx::PgPool,
    auth: RuntimeIdentity,
    api: Arc<Mutex<Api>>,
    task: tokio::task::JoinHandle<()>,
    agent: AgentId,
    attempt: AttemptId,
}
async fn fixture() -> Fixture {
    let (pool, registry, agent) = registry_support::setup().await;
    bind_anchor(&registry).await;
    let image = "registry.test/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let layout = Layout::new(
        image,
        image,
        "cubebot-runtime",
        "https://cubebot.test",
        None,
    )
    .unwrap();
    let attempt = registry
        .reserve(
            agent,
            2,
            ResourceSpec::Pod(Box::new(layout.pod("owned-workspace").unwrap())),
        )
        .await
        .unwrap();
    registry
        .record_prepared(attempt.id, &attempt.object)
        .await
        .unwrap();
    registry.begin_create(attempt.id).await.unwrap();
    let ResourceObject::Pod(mut pod) = attempt.object else {
        panic!("pod expected")
    };
    pod.metadata.uid = Some("pod-uid".into());
    pod.metadata.resource_version = Some("1".into());
    pod.status = Some(serde_json::from_value(json!({"phase":"Running"})).unwrap());
    registry
        .bind_object(attempt.id, &ResourceObject::Pod(pod.clone()))
        .await
        .unwrap();
    let state = Arc::new(Mutex::new(Api {
        pod: *pod,
        service_uid: "service-uid".into(),
    }));
    let router = Router::new().fallback(any(api)).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Fixture {
        auth: RuntimeIdentity::new(
            pool.clone(),
            kube::Config::new(url.parse().unwrap()),
            "cubebot-test",
        )
        .unwrap(),
        pool,
        api: state,
        task,
        agent,
        attempt: attempt.id,
    }
}
async fn bind_anchor(registry: &Registry) {
    let mut anchor = registry.configure("cubebot-test").await.unwrap();
    anchor.metadata.uid = Some("anchor-uid".into());
    anchor.metadata.resource_version = Some("1".into());
    registry.bind_anchor(&anchor).await.unwrap();
}

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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router = cubebot_orchestrator::api::runtime_router(auth);
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

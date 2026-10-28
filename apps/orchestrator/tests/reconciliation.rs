use crate::registry_support;
use crate::support;
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
};
use cubebot_orchestrator::agents::kubernetes::{Layout, ReconcileResult, Reconciler};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Server {
    objects: BTreeMap<String, Value>,
    creates: Vec<String>,
    create_attempts: Vec<String>,
    alter_pod_security: bool,
    alter_sandbox: bool,
    lose_kind: Option<String>,
    unavailable: bool,
    requests: usize,
    deletes: Vec<String>,
    patches: Vec<String>,
    lose_delete: bool,
    lose_finalize: bool,
    reject_finalize: bool,
}
type Shared = Arc<Mutex<Server>>;

async fn api(State(state): State<Shared>, method: Method, uri: Uri, bytes: Bytes) -> Response {
    let mut state = state.lock().unwrap();
    state.requests += 1;
    if state.unavailable {
        return failure(503);
    }
    match method {
        Method::GET => read(&state, uri.path()),
        Method::DELETE => delete(&mut state, uri.path(), &bytes),
        Method::PATCH => patch(&mut state, uri.path(), &bytes),
        _ => create(&mut state, uri, bytes),
    }
}
fn read(state: &Server, path: &str) -> Response {
    state
        .objects
        .get(path)
        .cloned()
        .map(Json)
        .map(IntoResponse::into_response)
        .unwrap_or_else(|| failure(404))
}
fn create(state: &mut Server, uri: Uri, bytes: Bytes) -> Response {
    let mut body: Value = serde_json::from_slice(&bytes).unwrap();
    let name = body["metadata"]["name"].as_str().unwrap().to_owned();
    let path = format!("{}/{name}", uri.path());
    let preview = uri
        .query()
        .is_some_and(|query| query.contains("dryRun=All"));
    if !preview {
        state.create_attempts.push(path.clone());
    }
    if !preview && state.objects.contains_key(&path) {
        return failure(409);
    }
    if state.alter_pod_security && body["kind"] == "Pod" {
        body["spec"]["hostNetwork"] = json!(true);
    }
    if state.alter_sandbox && body["kind"] == "Pod" {
        body["spec"]["containers"][0]["args"] = json!(["--backend", "kvm"]);
    }
    body["metadata"]["uid"] = json!(format!("uid-{name}"));
    body["metadata"]["resourceVersion"] = json!("1");
    if !preview {
        state.creates.push(path.clone());
        state.objects.insert(path, body.clone());
        if state.lose_kind.as_deref() == body["kind"].as_str() {
            return failure(503);
        }
    }
    Json(body).into_response()
}
fn delete(state: &mut Server, path: &str, bytes: &[u8]) -> Response {
    state.deletes.push(path.into());
    let body: Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(body["gracePeriodSeconds"], 30);
    let Some(pod) = state.objects.get_mut(path) else {
        return failure(404);
    };
    if body["preconditions"]["uid"] != pod["metadata"]["uid"]
        || body["preconditions"]["resourceVersion"] != pod["metadata"]["resourceVersion"]
    {
        return failure(409);
    }
    pod["metadata"]["deletionTimestamp"] = json!("2026-10-26T12:00:00Z");
    pod["metadata"]["resourceVersion"] = json!("2");
    if state.lose_delete {
        return failure(503);
    }
    Json(pod.clone()).into_response()
}
fn patch(state: &mut Server, path: &str, bytes: &[u8]) -> Response {
    state.patches.push(path.into());
    if state.reject_finalize {
        return failure(503);
    }
    let body: Value = serde_json::from_slice(bytes).unwrap();
    let Some(pod) = state.objects.get_mut(path) else {
        return failure(404);
    };
    if body["metadata"]["uid"] != pod["metadata"]["uid"]
        || body["metadata"]["resourceVersion"] != pod["metadata"]["resourceVersion"]
    {
        return failure(409);
    }
    pod["metadata"]["finalizers"] = body["metadata"]["finalizers"].clone();
    pod["metadata"]["resourceVersion"] = json!("3");
    let response = pod.clone();
    if pod["metadata"]["finalizers"].as_array().unwrap().is_empty() {
        state.objects.remove(path);
    }
    if state.lose_finalize {
        return failure(503);
    }
    Json(response).into_response()
}
fn failure(code: u16) -> Response {
    let reason = match code {
        404 => "NotFound",
        409 => "AlreadyExists",
        _ => "ServiceUnavailable",
    };
    (StatusCode::from_u16(code).unwrap(),Json(json!({"apiVersion":"v1","kind":"Status","status":"Failure","message":"controlled fault","reason":reason,"code":code}))).into_response()
}
async fn server() -> (kube::Config, Shared, tokio::task::JoinHandle<()>) {
    let state: Shared = Default::default();
    let router = Router::new().fallback(any(api)).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (kube::Config::new(url.parse().unwrap()), state, task)
}
fn layout() -> Layout {
    let image = "registry.test/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    Layout::new(
        image,
        "cubebot-runtime",
        "https://cubebot.test",
        cubebot_orchestrator::agents::sandbox::Backend::Process,
        cubebot_protocol::network::NetworkPolicy::new(vec!["10.96.0.10".parse().unwrap()]).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn reconnect_recovers_a_lost_pod_response_without_recreating_any_resource() {
    recover_lost_creation("Pod", 3).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn reconnect_recovers_a_lost_workspace_response_before_creating_the_pod() {
    recover_lost_creation("PersistentVolumeClaim", 2).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn reconnect_recovers_a_lost_anchor_response_without_transferring_identity() {
    recover_lost_creation("ConfigMap", 1).await;
}

async fn recover_lost_creation(kind: &str, initial_creates: usize) {
    let (pool, _, agent) = registry_support::setup().await;
    let (client, state, task) = server().await;
    state.lock().unwrap().lose_kind = Some(kind.into());
    let reconciler =
        Reconciler::new(pool.clone(), client.clone(), "cubebot-test", layout()).unwrap();
    assert!(reconciler.reconcile(agent).await.is_err());
    assert_eq!(state.lock().unwrap().creates.len(), initial_creates);
    let restarted = Reconciler::new(pool, client, "cubebot-test", layout()).unwrap();
    assert!(matches!(
        restarted.reconcile(agent).await.unwrap(),
        ReconcileResult::Provisioned { .. }
    ));
    assert_eq!(state.lock().unwrap().creates.len(), 3);
    assert_eq!(state.lock().unwrap().create_attempts.len(), 3);
    task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn missing_bound_objects_retain_identity_and_never_authorize_recreation() {
    let (pool, registry, agent) = registry_support::setup().await;
    let (client, state, task) = server().await;
    let reconciler = Reconciler::new(pool, client, "cubebot-test", layout()).unwrap();
    let ReconcileResult::Provisioned { workspace, pod } =
        reconciler.reconcile(agent).await.unwrap()
    else {
        panic!("provisioned expected")
    };
    for id in [workspace, pod] {
        let before = registry.attempt(id).await.unwrap();
        let name = before.object.metadata().name.as_deref().unwrap();
        let path = state
            .lock()
            .unwrap()
            .objects
            .keys()
            .find(|path| path.ends_with(&format!("/{name}")))
            .unwrap()
            .clone();
        let removed = state.lock().unwrap().objects.remove(&path).unwrap();
        assert!(
            matches!(reconciler.reconcile(agent).await.unwrap(), ReconcileResult::Uncertain { attempt } if attempt == id)
        );
        let after = registry.attempt(id).await.unwrap();
        assert_eq!(after.uid, before.uid);
        assert_eq!(after.blocked_reason.as_deref(), Some("object_missing"));
        assert_eq!(state.lock().unwrap().creates.len(), 3);
        state.lock().unwrap().objects.insert(path, removed);
    }
    task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn changed_object_uid_blocks_binding_and_never_overwrites_the_recorded_uid() {
    let (pool, registry, agent) = registry_support::setup().await;
    let (client, state, task) = server().await;
    let reconciler = Reconciler::new(pool, client, "cubebot-test", layout()).unwrap();
    let ReconcileResult::Provisioned { pod, .. } = reconciler.reconcile(agent).await.unwrap()
    else {
        panic!("provisioned expected")
    };
    let before = registry.attempt(pod).await.unwrap();
    let path = state
        .lock()
        .unwrap()
        .objects
        .keys()
        .find(|path| path.contains("/pods/"))
        .unwrap()
        .clone();
    state.lock().unwrap().objects.get_mut(&path).unwrap()["metadata"]["uid"] =
        json!("replacement-uid");
    assert!(reconciler.reconcile(agent).await.is_err());
    let after = registry.attempt(pod).await.unwrap();
    assert_eq!(after.uid, before.uid);
    assert_eq!(
        after.blocked_reason.as_deref(),
        Some("ownership_or_specification_conflict")
    );
    assert_eq!(state.lock().unwrap().creates.len(), 3);
    task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn unavailable_api_retains_intent_and_closed_database_performs_no_cluster_request() {
    let (pool, _, agent) = registry_support::setup().await;
    let (client, state, task) = server().await;
    let reconciler = Reconciler::new(pool.clone(), client, "cubebot-test", layout()).unwrap();
    state.lock().unwrap().unavailable = true;
    assert!(reconciler.reconcile(agent).await.is_err());
    assert!(state.lock().unwrap().creates.is_empty());
    assert_eq!(state.lock().unwrap().requests, 1);
    state.lock().unwrap().unavailable = false;
    assert!(matches!(
        reconciler.reconcile(agent).await.unwrap(),
        ReconcileResult::Provisioned { .. }
    ));
    let requests = state.lock().unwrap().requests;
    pool.close().await;
    assert!(reconciler.reconcile(agent).await.is_err());
    assert_eq!(state.lock().unwrap().requests, requests);
    task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn lifecycle_changes_hold_dispatched_execution_for_drain_without_cleanup() {
    use cubebot_orchestrator::{
        agents::{Agents, LifecycleAction, LifecycleCommand},
        identity::Identity,
    };
    let (pool, registry, agent) = registry_support::setup().await;
    let (client, state, task) = server().await;
    let reconciler = Reconciler::new(pool.clone(), client, "cubebot-test", layout()).unwrap();
    let ReconcileResult::Provisioned { pod, .. } = reconciler.reconcile(agent).await.unwrap()
    else {
        panic!("provisioned expected")
    };
    let identity = Identity::new(pool.clone());
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let actor = identity.authenticate(&session.token).await.unwrap();
    let agents = Agents::new(pool);
    let requests = state.lock().unwrap().requests;
    for (index, action) in [
        LifecycleAction::Sleep,
        LifecycleAction::Wake,
        LifecycleAction::Restart,
        LifecycleAction::Retire,
    ]
    .into_iter()
    .enumerate()
    {
        agents
            .lifecycle(
                &actor,
                agent,
                &LifecycleCommand {
                    request_id: uuid::Uuid::new_v4(),
                    expected_revision: 2 + index as i64,
                    action,
                },
            )
            .await
            .unwrap();
        assert!(
            matches!(reconciler.reconcile(agent).await.unwrap(), ReconcileResult::AwaitingDrain { attempt } if attempt == pod)
        );
    }
    assert_eq!(state.lock().unwrap().requests, requests + 4);
    assert_eq!(
        registry
            .attempt(pod)
            .await
            .unwrap()
            .blocked_reason
            .as_deref(),
        Some("drain_required")
    );
    task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_bound_anchor_cannot_be_recreated_or_rebound_after_loss() {
    let (pool, registry, agent) = registry_support::setup().await;
    let (client, state, task) = server().await;
    let reconciler = Reconciler::new(pool, client, "cubebot-test", layout()).unwrap();
    reconciler.reconcile(agent).await.unwrap();
    let expected = registry.configure("cubebot-test").await.unwrap();
    let path = state
        .lock()
        .unwrap()
        .objects
        .keys()
        .find(|path| path.contains("/configmaps/"))
        .unwrap()
        .clone();
    let mut anchor = state.lock().unwrap().objects.remove(&path).unwrap();
    assert!(reconciler.reconcile(agent).await.is_err());
    anchor["metadata"]["uid"] = json!("replaced-anchor");
    state.lock().unwrap().objects.insert(path, anchor);
    assert!(reconciler.reconcile(agent).await.is_err());
    assert_eq!(
        registry
            .configure("cubebot-test")
            .await
            .unwrap()
            .metadata
            .uid,
        expected.metadata.uid
    );
    assert_eq!(state.lock().unwrap().creates.len(), 3);
    task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn concurrent_reconcilers_converge_without_competing_resource_creates() {
    let (pool, _, agent) = registry_support::setup().await;
    let (client, state, task) = server().await;
    let left = Reconciler::new(pool.clone(), client.clone(), "cubebot-test", layout()).unwrap();
    let right = Reconciler::new(pool, client, "cubebot-test", layout()).unwrap();
    let (left_result, right_result) = tokio::join!(left.reconcile(agent), right.reconcile(agent));
    assert!(left_result.is_ok() || right_result.is_ok());
    assert!(matches!(
        left.reconcile(agent).await.unwrap(),
        ReconcileResult::Provisioned { .. }
    ));
    assert_eq!(state.lock().unwrap().creates.len(), 3);
    task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn unapproved_admission_changes_remain_planned_and_recover_after_policy_is_fixed() {
    verify_blocked_admission(false).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn admission_change_to_the_sandbox_backend_is_rejected_before_pod_dispatch() {
    verify_blocked_admission(true).await;
}

async fn verify_blocked_admission(sandbox: bool) {
    let (pool, _, agent) = registry_support::setup().await;
    let (client, state, task) = server().await;
    let reconciler = Reconciler::new(pool.clone(), client, "cubebot-test", layout()).unwrap();
    state.lock().unwrap().alter_pod_security = !sandbox;
    state.lock().unwrap().alter_sandbox = sandbox;
    assert!(reconciler.reconcile(agent).await.is_err());
    let (phase, reason, uid): (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT phase,blocked_reason,uid FROM agent_resources WHERE agent_id = $1 AND kind = 'pod'",
    )
    .bind(agent)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(phase, "planned");
    assert_eq!(reason.as_deref(), Some("preparation_failed"));
    assert!(uid.is_none());
    assert_eq!(state.lock().unwrap().create_attempts.len(), 2);
    state.lock().unwrap().alter_pod_security = false;
    state.lock().unwrap().alter_sandbox = false;
    assert!(matches!(
        reconciler.reconcile(agent).await.unwrap(),
        ReconcileResult::Provisioned { .. }
    ));
    assert_eq!(state.lock().unwrap().create_attempts.len(), 3);
    task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn acknowledged_sleep_stops_then_finalizes_the_exact_pod_and_retains_workspace() {
    let (pool, registry, agent) = registry_support::setup().await;
    let (client, state, task) = server().await;
    let reconciler = Reconciler::new(pool.clone(), client, "cubebot-test", layout()).unwrap();
    let ReconcileResult::Provisioned { pod, workspace } =
        reconciler.reconcile(agent).await.unwrap()
    else {
        panic!("provisioned expected")
    };
    request_sleep(&pool, agent).await;
    acknowledge(&pool, pod).await;
    assert!(
        matches!(reconciler.reconcile(agent).await.unwrap(), ReconcileResult::Stopping { attempt } if attempt == pod)
    );
    assert_eq!(
        registry.attempt(pod).await.unwrap().phase,
        cubebot_orchestrator::agents::kubernetes::Phase::Bound
    );
    complete_pod(&state);
    assert!(matches!(
        reconciler.reconcile(agent).await.unwrap(),
        ReconcileResult::Idle
    ));
    assert_eq!(
        registry.attempt(pod).await.unwrap().phase,
        cubebot_orchestrator::agents::kubernetes::Phase::Stopped
    );
    assert_eq!(
        registry.attempt(workspace).await.unwrap().phase,
        cubebot_orchestrator::agents::kubernetes::Phase::Bound
    );
    assert_eq!(state.lock().unwrap().deletes.len(), 1);
    assert_eq!(state.lock().unwrap().patches.len(), 1);
    assert_eq!(state.lock().unwrap().creates.len(), 3);
    task.abort();
}
async fn request_sleep(pool: &sqlx::PgPool, agent: cubebot_orchestrator::agents::AgentId) {
    use cubebot_orchestrator::{
        agents::{Agents, LifecycleAction, LifecycleCommand},
        identity::Identity,
    };
    let identity = Identity::new(pool.clone());
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let actor = identity.authenticate(&session.token).await.unwrap();
    Agents::new(pool.clone())
        .lifecycle(
            &actor,
            agent,
            &LifecycleCommand {
                request_id: uuid::Uuid::new_v4(),
                expected_revision: 2,
                action: LifecycleAction::Sleep,
            },
        )
        .await
        .unwrap();
}
async fn acknowledge(
    pool: &sqlx::PgPool,
    attempt: cubebot_orchestrator::agents::kubernetes::AttemptId,
) {
    sqlx::query("UPDATE agent_runtime_drains SET acknowledged_at = clock_timestamp(),acknowledged_session_version = 1 WHERE attempt_id = $1").bind(attempt).execute(pool).await.unwrap();
}
fn complete_pod(state: &Shared) {
    let mut state = state.lock().unwrap();
    let pod = state
        .objects
        .values_mut()
        .find(|object| object["kind"] == "Pod")
        .unwrap();
    let containers = stopped_from_spec(&pod["spec"]["containers"]);
    pod["status"] = json!({"phase":"Succeeded","containerStatuses":containers});
}
fn stopped_from_spec(value: &Value) -> Vec<Value> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|container| stopped_container(container["name"].as_str().unwrap()))
        .collect()
}
fn stopped_container(name: &str) -> Value {
    let id = format!("containerd://{name}");
    json!({"name":name,"image":"test","imageID":"test","ready":false,"restartCount":0,"containerID":id,"state":{"terminated":{"exitCode":0,"reason":"Completed","containerID":id,"startedAt":"2026-10-26T11:00:00Z","finishedAt":"2026-10-26T12:00:01Z"}}})
}

struct Shutdown {
    pool: sqlx::PgPool,
    registry: cubebot_orchestrator::agents::kubernetes::Registry,
    config: kube::Config,
    state: Shared,
    task: tokio::task::JoinHandle<()>,
    reconciler: Reconciler,
    agent: cubebot_orchestrator::agents::AgentId,
    pod: cubebot_orchestrator::agents::kubernetes::AttemptId,
}
async fn shutdown_fixture() -> Shutdown {
    let (pool, registry, agent) = registry_support::setup().await;
    let (config, state, task) = server().await;
    let reconciler =
        Reconciler::new(pool.clone(), config.clone(), "cubebot-test", layout()).unwrap();
    let ReconcileResult::Provisioned { pod, .. } = reconciler.reconcile(agent).await.unwrap()
    else {
        panic!("provisioned expected")
    };
    request_sleep(&pool, agent).await;
    acknowledge(&pool, pod).await;
    Shutdown {
        pool,
        registry,
        config,
        state,
        task,
        reconciler,
        agent,
        pod,
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn lost_delete_reply_is_observed_without_repeating_delete_or_releasing_ownership() {
    let f = shutdown_fixture().await;
    f.state.lock().unwrap().lose_delete = true;
    assert!(f.reconciler.reconcile(f.agent).await.is_err());
    assert!(matches!(
        f.reconciler.reconcile(f.agent).await.unwrap(),
        ReconcileResult::Stopping { .. }
    ));
    assert_eq!(f.state.lock().unwrap().deletes.len(), 1);
    assert_eq!(
        f.registry.attempt(f.pod).await.unwrap().phase,
        cubebot_orchestrator::agents::kubernetes::Phase::Bound
    );
    complete_pod(&f.state);
    assert!(matches!(
        f.reconciler.reconcile(f.agent).await.unwrap(),
        ReconcileResult::Idle
    ));
    assert_eq!(f.state.lock().unwrap().creates.len(), 3);
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn lost_finalizer_reply_recovers_only_from_durably_retained_terminal_proof() {
    let f = shutdown_fixture().await;
    f.reconciler.reconcile(f.agent).await.unwrap();
    complete_pod(&f.state);
    f.state.lock().unwrap().lose_finalize = true;
    assert!(f.reconciler.reconcile(f.agent).await.is_err());
    let (phase, retained): (String, bool) = sqlx::query_as(
        "SELECT phase,termination_proof IS NOT NULL FROM agent_resources WHERE id = $1",
    )
    .bind(f.pod)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(phase, "bound");
    assert!(retained);
    let restarted =
        Reconciler::new(f.pool.clone(), f.config.clone(), "cubebot-test", layout()).unwrap();
    assert!(matches!(
        restarted.reconcile(f.agent).await.unwrap(),
        ReconcileResult::Idle
    ));
    assert_eq!(
        f.registry.attempt(f.pod).await.unwrap().phase,
        cubebot_orchestrator::agents::kubernetes::Phase::Stopped
    );
    assert_eq!(f.state.lock().unwrap().patches.len(), 1);
    assert_eq!(f.state.lock().unwrap().deletes.len(), 1);
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn acknowledged_but_missing_pod_without_terminal_proof_remains_uncertain() {
    let f = shutdown_fixture().await;
    f.state
        .lock()
        .unwrap()
        .objects
        .retain(|_, object| object["kind"] != "Pod");
    assert!(
        matches!(f.reconciler.reconcile(f.agent).await.unwrap(), ReconcileResult::Uncertain { attempt } if attempt == f.pod)
    );
    assert_eq!(
        f.registry.attempt(f.pod).await.unwrap().phase,
        cubebot_orchestrator::agents::kubernetes::Phase::Bound
    );
    assert!(f.state.lock().unwrap().deletes.is_empty());
    assert!(f.state.lock().unwrap().patches.is_empty());
    assert_eq!(f.state.lock().unwrap().creates.len(), 3);
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn acknowledged_node_lost_status_cannot_release_the_execution_finalizer() {
    let f = shutdown_fixture().await;
    f.reconciler.reconcile(f.agent).await.unwrap();
    complete_pod(&f.state);
    {
        let mut state = f.state.lock().unwrap();
        let pod = state
            .objects
            .values_mut()
            .find(|object| object["kind"] == "Pod")
            .unwrap();
        pod["status"]["reason"] = json!("NodeLost");
    }
    assert!(matches!(
        f.reconciler.reconcile(f.agent).await.unwrap(),
        ReconcileResult::Stopping { .. }
    ));
    assert_eq!(
        f.registry.attempt(f.pod).await.unwrap().phase,
        cubebot_orchestrator::agents::kubernetes::Phase::Bound
    );
    assert!(f.state.lock().unwrap().patches.is_empty());
    assert_eq!(f.state.lock().unwrap().deletes.len(), 1);
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn finalization_preserves_foreign_finalizers_and_never_deletes_workspace() {
    let f = shutdown_fixture().await;
    f.reconciler.reconcile(f.agent).await.unwrap();
    complete_pod(&f.state);
    {
        let mut state = f.state.lock().unwrap();
        let pod = state
            .objects
            .values_mut()
            .find(|object| object["kind"] == "Pod")
            .unwrap();
        pod["metadata"]["finalizers"]
            .as_array_mut()
            .unwrap()
            .push(json!("other.test/retained"));
    }
    f.state.lock().unwrap().lose_finalize = true;
    assert!(f.reconciler.reconcile(f.agent).await.is_err());
    f.state.lock().unwrap().lose_finalize = false;
    assert!(matches!(
        f.reconciler.reconcile(f.agent).await.unwrap(),
        ReconcileResult::Idle
    ));
    let state = f.state.lock().unwrap();
    let pod = state
        .objects
        .values()
        .find(|object| object["kind"] == "Pod")
        .unwrap();
    assert_eq!(
        pod["metadata"]["finalizers"],
        json!(["other.test/retained"])
    );
    assert!(state.deletes.iter().all(|path| path.contains("/pods/")));
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn retained_terminal_proof_does_not_allow_finalizing_a_replaced_or_changed_pod() {
    for change in ["uid", "spec"] {
        let f = shutdown_fixture().await;
        f.reconciler.reconcile(f.agent).await.unwrap();
        complete_pod(&f.state);
        f.state.lock().unwrap().reject_finalize = true;
        assert!(f.reconciler.reconcile(f.agent).await.is_err());
        {
            let mut state = f.state.lock().unwrap();
            state.reject_finalize = false;
            let pod = state
                .objects
                .values_mut()
                .find(|object| object["kind"] == "Pod")
                .unwrap();
            if change == "uid" {
                pod["metadata"]["uid"] = json!("replacement-uid");
            } else {
                pod["spec"]["containers"][0]["image"] = json!("unapproved-image");
            }
        }
        assert!(f.reconciler.reconcile(f.agent).await.is_err());
        assert_eq!(f.state.lock().unwrap().patches.len(), 1);
        assert_eq!(
            f.registry.attempt(f.pod).await.unwrap().phase,
            cubebot_orchestrator::agents::kubernetes::Phase::Bound
        );
        f.task.abort();
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn sleep_after_a_lost_create_response_binds_the_same_pod_for_recovery_drain() {
    let (pool, registry, agent) = registry_support::setup().await;
    let (config, state, task) = server().await;
    state.lock().unwrap().lose_kind = Some("Pod".into());
    let reconciler = Reconciler::new(pool.clone(), config, "cubebot-test", layout()).unwrap();
    assert!(reconciler.reconcile(agent).await.is_err());
    request_sleep(&pool, agent).await;
    let ReconcileResult::AwaitingDrain { attempt } = reconciler.reconcile(agent).await.unwrap()
    else {
        panic!("drain expected")
    };
    assert_eq!(
        registry.attempt(attempt).await.unwrap().phase,
        cubebot_orchestrator::agents::kubernetes::Phase::Bound
    );
    acknowledge(&pool, attempt).await;
    assert!(matches!(
        reconciler.reconcile(agent).await.unwrap(),
        ReconcileResult::Stopping { .. }
    ));
    assert_eq!(state.lock().unwrap().create_attempts.len(), 3);
    assert_eq!(state.lock().unwrap().deletes.len(), 1);
    task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn waking_after_proven_stop_creates_one_new_epoch_and_reuses_the_workspace_uid() {
    use cubebot_orchestrator::{
        agents::{Agents, LifecycleAction, LifecycleCommand},
        identity::Identity,
    };
    let f = shutdown_fixture().await;
    let old = f.registry.attempt(f.pod).await.unwrap();
    f.reconciler.reconcile(f.agent).await.unwrap();
    complete_pod(&f.state);
    f.reconciler.reconcile(f.agent).await.unwrap();
    let identity = Identity::new(f.pool.clone());
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let actor = identity.authenticate(&session.token).await.unwrap();
    Agents::new(f.pool.clone())
        .lifecycle(
            &actor,
            f.agent,
            &LifecycleCommand {
                request_id: uuid::Uuid::new_v4(),
                expected_revision: 3,
                action: LifecycleAction::Wake,
            },
        )
        .await
        .unwrap();
    let ReconcileResult::Provisioned { pod, workspace } =
        f.reconciler.reconcile(f.agent).await.unwrap()
    else {
        panic!("new epoch expected")
    };
    let new = f.registry.attempt(pod).await.unwrap();
    assert_ne!(pod, f.pod);
    assert_eq!(new.epoch, old.epoch + 1);
    assert_eq!(new.target_generation, old.target_generation + 1);
    let claim = f.registry.attempt(workspace).await.unwrap();
    assert!(claim.uid.is_some());
    let state = f.state.lock().unwrap();
    assert_eq!(state.creates.len(), 4);
    assert_eq!(
        state
            .creates
            .iter()
            .filter(|path| path.contains("/persistentvolumeclaims/"))
            .count(),
        1
    );
    assert!(
        state
            .objects
            .values()
            .any(|object| object["metadata"]["uid"].as_str() == claim.uid.as_deref())
    );
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn same_generation_terminal_pod_is_fenced_before_one_new_epoch_is_created() {
    let (pool, registry, agent) = registry_support::setup().await;
    let (config, state, task) = server().await;
    let reconciler = Reconciler::new(pool.clone(), config, "cubebot-test", layout()).unwrap();
    let ReconcileResult::Provisioned { pod, workspace } =
        reconciler.reconcile(agent).await.unwrap()
    else {
        panic!("provisioned expected")
    };
    let previous = registry.attempt(pod).await.unwrap();
    complete_pod(&state);
    assert!(
        matches!(reconciler.reconcile(agent).await.unwrap(), ReconcileResult::Stopping { attempt } if attempt == pod)
    );
    let (bound, proof, latched): (bool, bool, bool) = sqlx::query_as("SELECT phase = 'bound',termination_proof IS NOT NULL,EXISTS(SELECT 1 FROM agent_runtime_drains WHERE attempt_id = r.id) FROM agent_resources r WHERE id = $1").bind(pod).fetch_one(&pool).await.unwrap();
    assert!(bound && proof && latched);
    let ReconcileResult::Provisioned {
        pod: replacement,
        workspace: retained,
    } = reconciler.reconcile(agent).await.unwrap()
    else {
        panic!("replacement expected")
    };
    let next = registry.attempt(replacement).await.unwrap();
    assert_ne!(pod, replacement);
    assert_eq!(workspace, retained);
    assert_eq!(next.epoch, previous.epoch + 1);
    assert_eq!(next.target_generation, previous.target_generation);
    assert_eq!(state.lock().unwrap().creates.len(), 4);
    task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn same_generation_node_lost_is_visible_and_never_qualifies_for_replacement() {
    use cubebot_orchestrator::{agents::Agents, identity::Identity};
    let (pool, registry, agent) = registry_support::setup().await;
    let (config, state, task) = server().await;
    let reconciler = Reconciler::new(pool.clone(), config, "cubebot-test", layout()).unwrap();
    let ReconcileResult::Provisioned { pod, .. } = reconciler.reconcile(agent).await.unwrap()
    else {
        panic!("provisioned expected")
    };
    complete_pod(&state);
    {
        let mut state = state.lock().unwrap();
        let pod = state
            .objects
            .values_mut()
            .find(|object| object["kind"] == "Pod")
            .unwrap();
        pod["status"]["phase"] = json!("Failed");
        pod["status"]["reason"] = json!("NodeLost");
    }
    assert!(
        matches!(reconciler.reconcile(agent).await.unwrap(), ReconcileResult::AwaitingDrain { attempt } if attempt == pod)
    );
    let identity = Identity::new(pool.clone());
    let human = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let actor = identity.authenticate(&human.token).await.unwrap();
    let status = Agents::new(pool).status(&actor, agent).await.unwrap();
    assert_eq!(status.kubernetes_reason.as_deref(), Some("NodeLost"));
    assert!(status.drain_requested);
    assert!(!status.terminal_evidence_retained);
    assert_eq!(
        registry.attempt(pod).await.unwrap().phase,
        cubebot_orchestrator::agents::kubernetes::Phase::Bound
    );
    assert!(state.lock().unwrap().deletes.is_empty());
    assert!(state.lock().unwrap().patches.is_empty());
    assert_eq!(state.lock().unwrap().creates.len(), 3);
    task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn current_generation_recovers_a_lost_finalizer_reply_before_replacement() {
    let (pool, registry, agent) = registry_support::setup().await;
    let (config, state, task) = server().await;
    let reconciler =
        Reconciler::new(pool.clone(), config.clone(), "cubebot-test", layout()).unwrap();
    let ReconcileResult::Provisioned { pod, .. } = reconciler.reconcile(agent).await.unwrap()
    else {
        panic!("provisioned expected")
    };
    complete_pod(&state);
    assert!(matches!(
        reconciler.reconcile(agent).await.unwrap(),
        ReconcileResult::Stopping { .. }
    ));
    state.lock().unwrap().lose_finalize = true;
    assert!(reconciler.reconcile(agent).await.is_err());
    assert_eq!(
        registry.attempt(pod).await.unwrap().phase,
        cubebot_orchestrator::agents::kubernetes::Phase::Bound
    );
    let restarted = Reconciler::new(pool, config, "cubebot-test", layout()).unwrap();
    let ReconcileResult::Provisioned {
        pod: replacement, ..
    } = restarted.reconcile(agent).await.unwrap()
    else {
        panic!("recovered epoch expected")
    };
    assert_ne!(pod, replacement);
    assert_eq!(
        registry.attempt(pod).await.unwrap().phase,
        cubebot_orchestrator::agents::kubernetes::Phase::Stopped
    );
    assert_eq!(state.lock().unwrap().patches.len(), 1);
    assert_eq!(state.lock().unwrap().creates.len(), 4);
    task.abort();
}

fn platform_identity() -> cubebot_orchestrator::operations::ownership::PodIdentity {
    cubebot_orchestrator::operations::ownership::PodIdentity {
        namespace: "cubebot-test".into(),
        name: "orchestrator-a".into(),
        uid: "platform-a".into(),
    }
}
fn platform_pod() -> Value {
    json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":"orchestrator-a","namespace":"cubebot-test","uid":"platform-a"},"spec":{"restartPolicy":"Always","serviceAccountName":"cubebot-orchestrator","containers":[{"name":"orchestrator","image":"test"}]},"status":{"phase":"Running","containerStatuses":[{"name":"orchestrator","image":"test","imageID":"test","ready":true,"restartCount":0,"containerID":"containerd://platform-a","state":{"running":{"startedAt":"2026-10-26T12:00:00Z"}}}]}})
}
async fn wait_bound_pod(pool: &sqlx::PgPool) {
    for _ in 0..200 {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM agent_resources WHERE kind = 'pod' AND phase = 'bound'",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        if count == 1 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("automatic controller did not bind a pod");
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn owned_server_reconciles_intent_and_releases_only_after_shutdown() {
    use cubebot_orchestrator::{agents::kubernetes::Controller, operations::server};
    let (pool, _, _) = registry_support::setup().await;
    let (config, state, api_task) = server().await;
    state.lock().unwrap().objects.insert(
        "/api/v1/namespaces/cubebot-test/pods/orchestrator-a".into(),
        platform_pod(),
    );
    let controller = Controller::new(pool.clone(), config.clone(), layout(), platform_identity())
        .await
        .unwrap();
    assert!(Controller::database_only(pool.clone()).await.is_err());
    let routes =
        cubebot_orchestrator::api::database_router(pool.clone()).merge(controller.runtime_router());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(server::serve(listener, routes, Some(controller), async {
        stopped.await.unwrap();
    }));
    wait_bound_pod(&pool).await;
    let http = openidconnect::reqwest::Client::new();
    assert_eq!(
        http.get(format!("{origin}/api/health/ready"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        http.get(format!("{origin}/api/runtime/identity"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert!(
        Controller::new(pool.clone(), config.clone(), layout(), platform_identity())
            .await
            .is_err()
    );
    stop.send(()).unwrap();
    serving.await.unwrap().unwrap();
    let released: bool = sqlx::query_scalar(
        "SELECT released_at IS NOT NULL FROM orchestrator_owner WHERE singleton",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(released);
    let next = Controller::new(pool.clone(), config, layout(), platform_identity())
        .await
        .unwrap();
    server::serve(
        tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap(),
        cubebot_orchestrator::api::router(),
        Some(next),
        async {},
    )
    .await
    .unwrap();
    let generation: i64 =
        sqlx::query_scalar("SELECT generation FROM orchestrator_owner WHERE singleton")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(generation, 2);
    assert_eq!(state.lock().unwrap().creates.len(), 3);
    api_task.abort();
}

struct OrchestratorProcess {
    child: std::process::Child,
    directory: std::path::PathBuf,
    origin: String,
}
impl Drop for OrchestratorProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
fn orchestrator_process(
    pool: &sqlx::PgPool,
    api: &kube::Config,
    configured: bool,
    extra: Option<(&str, String)>,
) -> OrchestratorProcess {
    use sqlx::ConnectOptions;
    let directory = std::env::current_dir().unwrap().join(format!(
        "../../target/controller-test-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir(&directory).unwrap();
    let kubeconfig = directory.join("kubeconfig.json");
    std::fs::write(&kubeconfig,json!({"apiVersion":"v1","kind":"Config","clusters":[{"name":"controlled","cluster":{"server":api.cluster_url.to_string()}}],"users":[{"name":"controlled","user":{}}],"contexts":[{"name":"controlled","context":{"cluster":"controlled","user":"controlled"}}],"current-context":"controlled"}).to_string()).unwrap();
    let image = "registry.test/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let configuration = directory.join("controller.json");
    std::fs::write(&configuration,json!({"connection":{"kind":"kubeconfig","path":kubeconfig,"context":"controlled"},"sandbox_image":image,"backend":"process","runtime_service_account":"cubebot-runtime","orchestrator_origin":"https://cubebot.test","network":{"dns":["10.96.0.10"]}}).to_string()).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_cubebot-orchestrator"));
    command
        .env(
            "CUBEBOT_DATABASE_URL",
            pool.connect_options().to_url_lossy().as_str(),
        )
        .env("CUBEBOT_BIND_ADDR", address.to_string())
        .env("CUBEBOT_POD_NAMESPACE", "cubebot-test")
        .env("CUBEBOT_POD_NAME", "orchestrator-a")
        .env("CUBEBOT_POD_UID", "platform-a")
        .env_remove("CUBEBOT_OIDC_CONFIG")
        .env_remove("CUBEBOT_PUBLIC_ORIGIN")
        .env_remove("CUBEBOT_CONTROLLER_CONFIG")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if configured {
        command.env("CUBEBOT_CONTROLLER_CONFIG", configuration);
    }
    if let Some((key, value)) = extra {
        command.env(key, value);
    }
    let child = command.spawn().unwrap();
    OrchestratorProcess {
        child,
        directory,
        origin: format!("http://{address}"),
    }
}
async fn process_exit(process: &mut OrchestratorProcess) -> std::process::ExitStatus {
    for _ in 0..400 {
        if let Some(status) = process.child.try_wait().unwrap() {
            return status;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("orchestrator did not exit");
}
async fn process_ready(process: &mut OrchestratorProcess) {
    let client = openidconnect::reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(100))
        .build()
        .unwrap();
    for _ in 0..200 {
        assert!(
            process.child.try_wait().unwrap().is_none(),
            "orchestrator exited before readiness"
        );
        if client
            .get(format!("{}/api/health/ready", process.origin))
            .send()
            .await
            .is_ok_and(|response| response.status() == 200)
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("orchestrator did not become ready");
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn actual_orchestrator_activates_explicit_controller_and_rejects_ownership_bypass() {
    let (pool, _, _) = registry_support::setup().await;
    let (config, state, api_task) = server().await;
    state.lock().unwrap().objects.insert(
        "/api/v1/namespaces/cubebot-test/pods/orchestrator-a".into(),
        platform_pod(),
    );
    let mut development = orchestrator_process(&pool, &config, false, None);
    process_ready(&mut development).await;
    let mut premature = orchestrator_process(&pool, &config, true, None);
    assert!(!process_exit(&mut premature).await.success());
    let mut duplicate_development = orchestrator_process(&pool, &config, false, None);
    assert!(!process_exit(&mut duplicate_development).await.success());
    assert!(
        std::process::Command::new("kill")
            .args(["-TERM", &development.child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    assert!(process_exit(&mut development).await.success());
    let mut process = orchestrator_process(&pool, &config, true, None);
    process_ready(&mut process).await;
    wait_bound_pod(&pool).await;
    let mut competing = orchestrator_process(&pool, &config, true, None);
    assert!(!process_exit(&mut competing).await.success());
    let mut bypass = orchestrator_process(&pool, &config, false, None);
    assert!(!process_exit(&mut bypass).await.success());
    assert!(
        std::process::Command::new("kill")
            .args(["-TERM", &process.child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    assert!(process_exit(&mut process).await.success());
    let released: bool = sqlx::query_scalar(
        "SELECT released_at IS NOT NULL FROM orchestrator_owner WHERE singleton",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(released);
    let mut restarted = orchestrator_process(&pool, &config, true, None);
    process_ready(&mut restarted).await;
    assert_eq!(state.lock().unwrap().creates.len(), 3);
    restarted.child.kill().unwrap();
    assert!(!process_exit(&mut restarted).await.success());
    let mut uncertain = orchestrator_process(&pool, &config, true, None);
    assert!(!process_exit(&mut uncertain).await.success());
    let generation: i64 =
        sqlx::query_scalar("SELECT generation FROM orchestrator_owner WHERE singleton")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(generation, 2);
    api_task.abort();
}

struct HeldRequest {
    entered: tokio::sync::Semaphore,
    finish: tokio::sync::Semaphore,
}
async fn held_request(State(state): State<Arc<HeldRequest>>) -> &'static str {
    state.entered.add_permits(1);
    state.finish.acquire().await.unwrap().forget();
    "settled"
}
async fn wait_listener_closed(address: std::net::SocketAddr) {
    for _ in 0..200 {
        if tokio::net::TcpStream::connect(address).await.is_err() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("HTTP listener remained active");
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn clean_owner_release_waits_for_in_flight_http_handlers() {
    use cubebot_orchestrator::{agents::kubernetes::Controller, operations::server};
    let (pool, _, _) = registry_support::setup().await;
    let (config, state, api_task) = server().await;
    state.lock().unwrap().objects.insert(
        "/api/v1/namespaces/cubebot-test/pods/orchestrator-a".into(),
        platform_pod(),
    );
    let controller = Controller::new(pool.clone(), config, layout(), platform_identity())
        .await
        .unwrap();
    let held = Arc::new(HeldRequest {
        entered: tokio::sync::Semaphore::new(0),
        finish: tokio::sync::Semaphore::new(0),
    });
    let routes = Router::new()
        .route("/hold", axum::routing::get(held_request))
        .with_state(held.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(server::serve(listener, routes, Some(controller), async {
        stopped.await.unwrap();
    }));
    let request = tokio::spawn(async move {
        openidconnect::reqwest::get(format!("http://{address}/hold"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    });
    held.entered.acquire().await.unwrap().forget();
    stop.send(()).unwrap();
    wait_listener_closed(address).await;
    let released: bool = sqlx::query_scalar(
        "SELECT released_at IS NOT NULL FROM orchestrator_owner WHERE singleton",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!released);
    assert!(!serving.is_finished());
    held.finish.add_permits(1);
    assert_eq!(request.await.unwrap(), "settled");
    serving.await.unwrap().unwrap();
    let released: bool = sqlx::query_scalar(
        "SELECT released_at IS NOT NULL FROM orchestrator_owner WHERE singleton",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(released);
    api_task.abort();
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn lost_owner_session_stops_http_and_controller_without_releasing_claim() {
    use cubebot_orchestrator::{agents::kubernetes::Controller, operations::server};
    let (pool, _, _) = registry_support::setup().await;
    let (config, state, api_task) = server().await;
    state.lock().unwrap().objects.insert(
        "/api/v1/namespaces/cubebot-test/pods/orchestrator-a".into(),
        platform_pod(),
    );
    let controller = Controller::new(pool.clone(), config, layout(), platform_identity())
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let serving = tokio::spawn(server::serve(
        listener,
        cubebot_orchestrator::api::router(),
        Some(controller),
        std::future::pending(),
    ));
    wait_bound_pod(&pool).await;
    let killed: bool = sqlx::query_scalar("SELECT pg_terminate_backend(pid) FROM pg_locks WHERE locktype = 'advisory' AND granted AND database = (SELECT oid FROM pg_database WHERE datname = current_database())").fetch_one(&pool).await.unwrap();
    assert!(killed);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(8), serving)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    wait_listener_closed(address).await;
    let released: bool = sqlx::query_scalar(
        "SELECT released_at IS NOT NULL FROM orchestrator_owner WHERE singleton",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!released);
    assert_eq!(state.lock().unwrap().creates.len(), 3);
    api_task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn handled_startup_errors_do_not_strand_development_ownership() {
    let pool = support::database().await;
    let (config, _, api_task) = server().await;
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    for (key, value) in [
        ("CUBEBOT_PUBLIC_ORIGIN", "http://invalid.test".to_string()),
        ("CUBEBOT_BIND_ADDR", "invalid-address".to_string()),
        (
            "CUBEBOT_BIND_ADDR",
            occupied.local_addr().unwrap().to_string(),
        ),
    ] {
        let mut process = orchestrator_process(&pool, &config, false, Some((key, value)));
        assert!(!process_exit(&mut process).await.success());
        let held: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM orchestrator_development_owner WHERE released_at IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(held, 0, "handled startup failure retained ownership");
    }
    let mut corrected = orchestrator_process(&pool, &config, false, None);
    process_ready(&mut corrected).await;
    assert!(
        std::process::Command::new("kill")
            .args(["-TERM", &corrected.child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    assert!(process_exit(&mut corrected).await.success());
    api_task.abort();
}

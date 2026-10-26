#[path = "support/registry.rs"]
mod registry_support;
mod support;
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
    lose_kind: Option<String>,
    unavailable: bool,
    requests: usize,
}
type Shared = Arc<Mutex<Server>>;

async fn api(State(state): State<Shared>, method: Method, uri: Uri, bytes: Bytes) -> Response {
    let mut state = state.lock().unwrap();
    state.requests += 1;
    if state.unavailable {
        return failure(503);
    }
    if method == Method::GET {
        return state
            .objects
            .get(uri.path())
            .cloned()
            .map(Json)
            .map(IntoResponse::into_response)
            .unwrap_or_else(|| failure(404));
    }
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
        image,
        "cubebot-runtime",
        "https://cubebot.test",
        None,
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
    assert_eq!(state.lock().unwrap().requests, requests);
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
    let (pool, _, agent) = registry_support::setup().await;
    let (client, state, task) = server().await;
    let reconciler = Reconciler::new(pool.clone(), client, "cubebot-test", layout()).unwrap();
    state.lock().unwrap().alter_pod_security = true;
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
    assert!(matches!(
        reconciler.reconcile(agent).await.unwrap(),
        ReconcileResult::Provisioned { .. }
    ));
    assert_eq!(state.lock().unwrap().create_attempts.len(), 3);
    task.abort();
}

use crate::registry_support;
use cubebot_orchestrator::{
    agents::{
        AgentId, Agents, LifecycleAction, LifecycleCommand,
        kubernetes::{Registry, ResourceObject, ResourceSpec},
    },
    identity::Identity,
};
use k8s_openapi::api::core::v1::PodSpec;
use serde_json::json;
use uuid::Uuid;

fn network_upgrade_specs() -> (PodSpec, PodSpec) {
    let image = "registry.test/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let network =
        cubebot_protocol::network::NetworkPolicy::new(vec!["10.96.0.10".parse().unwrap()]).unwrap();
    let layout = cubebot_orchestrator::agents::kubernetes::Layout::new(
        image,
        "cubebot-runtime",
        "https://cubebot.test",
        network,
    )
    .unwrap();
    let current = layout.pod("workspace").unwrap();
    let mut legacy = current.clone();
    legacy.dns_policy = Some("ClusterFirst".into());
    legacy.dns_config = None;
    (legacy, current)
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn unfiltered_plans_are_replaced_only_before_dispatch() {
    let (_, registry, agent) = setup().await;
    let (legacy, current) = network_upgrade_specs();
    let old = registry
        .reserve(agent, 2, ResourceSpec::Pod(Box::new(legacy)))
        .await
        .unwrap();
    let next = registry
        .reserve(agent, 2, ResourceSpec::Pod(Box::new(current)))
        .await
        .unwrap();
    assert_ne!(old.id, next.id);
    assert_eq!(next.epoch, old.epoch + 1);
    assert_eq!(
        registry.attempt(old.id).await.unwrap().phase,
        cubebot_orchestrator::agents::kubernetes::Phase::Stopped
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn dispatched_legacy_pods_hold_policy_upgrade_without_replacement() {
    let (_, registry, agent) = setup().await;
    let (legacy, current) = network_upgrade_specs();
    let old = registry
        .reserve(agent, 2, ResourceSpec::Pod(Box::new(legacy.clone())))
        .await
        .unwrap();
    dispatch(&registry, old.id).await;
    assert!(
        registry
            .reserve(agent, 2, ResourceSpec::Pod(Box::new(current)))
            .await
            .is_err()
    );
    let retained = registry.attempt(old.id).await.unwrap();
    assert_eq!(
        retained.phase,
        cubebot_orchestrator::agents::kubernetes::Phase::Creating
    );
    let ResourceObject::Pod(pod) = retained.intent else {
        panic!("Pod expected");
    };
    assert_eq!(pod.spec.unwrap(), legacy);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn reservation_replays_and_recovers_a_lost_create_response_without_transferring_uid() {
    let (pool, registry, agent) = setup().await;
    let spec: PodSpec = serde_json::from_value(
        json!({"restartPolicy":"Never","containers":[{"name":"sandbox","image":"sandbox-test"}]}),
    )
    .unwrap();
    let first = registry
        .reserve(agent, 2, ResourceSpec::Pod(Box::new(spec.clone())))
        .await
        .unwrap();
    let repeated = Registry::new(pool.clone())
        .reserve(agent, 2, ResourceSpec::Pod(Box::new(spec.clone())))
        .await
        .unwrap();
    assert_eq!(first.id, repeated.id);
    assert!(registry.bind_object(first.id, &first.object).await.is_err());
    dispatch(&registry, first.id).await;
    let ResourceObject::Pod(mut observed) = first.object else {
        panic!("pod expected")
    };
    observed.metadata.uid = Some("pod-uid".into());
    observed.metadata.resource_version = Some("2".into());
    registry
        .bind_object(first.id, &ResourceObject::Pod(observed.clone()))
        .await
        .unwrap();
    registry
        .bind_object(first.id, &ResourceObject::Pod(observed.clone()))
        .await
        .unwrap();
    registry.record_missing(first.id).await.unwrap();
    assert_eq!(
        registry
            .attempt(first.id)
            .await
            .unwrap()
            .blocked_reason
            .as_deref(),
        Some("object_missing")
    );
    assert!(registry.record_stopped(first.id, &observed).await.is_err());
    let retained = registry
        .reserve(agent, 2, ResourceSpec::Pod(Box::new(spec.clone())))
        .await
        .unwrap();
    assert_eq!(first.id, retained.id);
    terminate(&mut observed);
    registry.record_stopped(first.id, &observed).await.unwrap();
    registry.record_stopped(first.id, &observed).await.unwrap();
    let next = registry
        .reserve(agent, 2, ResourceSpec::Pod(Box::new(spec)))
        .await
        .unwrap();
    assert_ne!(first.id, next.id);
    assert_eq!(next.epoch, first.epoch + 1);
    observed.metadata.uid = Some("other-pod-uid".into());
    assert!(
        registry
            .bind_object(first.id, &ResourceObject::Pod(observed))
            .await
            .is_err()
    );
    assert_eq!(
        registry.attempt(first.id).await.unwrap().uid.as_deref(),
        Some("pod-uid")
    );
}

fn terminate(pod: &mut k8s_openapi::api::core::v1::Pod) {
    pod.metadata.deletion_timestamp =
        Some(serde_json::from_value(json!("2026-10-26T12:00:00Z")).unwrap());
    pod.status = Some(serde_json::from_value(json!({"phase":"Failed","containerStatuses":[{"name":"sandbox","image":"test","imageID":"test","ready":false,"restartCount":0,"containerID":"containerd://sandbox-id","state":{"terminated":{"exitCode":1,"reason":"Error","containerID":"containerd://sandbox-id","startedAt":"2026-10-26T11:00:00Z","finishedAt":"2026-10-26T12:00:01Z"}}}]})).unwrap());
}

async fn setup() -> (sqlx::PgPool, Registry, AgentId) {
    let (pool, registry, agent) = registry_support::setup().await;
    let mut anchor = registry.configure("cubebot-test").await.unwrap();
    anchor.metadata.uid = Some("anchor-uid".into());
    anchor.metadata.resource_version = Some("1".into());
    registry.bind_anchor(&anchor).await.unwrap();
    (pool, registry, agent)
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn concurrent_reservations_and_retained_workspace_keep_one_identity() {
    let (_, registry, agent) = setup().await;
    let spec = || {
        ResourceSpec::Workspace(Box::new(
            serde_json::from_value(
                json!({"accessModes":["ReadWriteOnce"],"resources":{"requests":{"storage":"1Gi"}}}),
            )
            .unwrap(),
        ))
    };
    let (left, right) = tokio::join!(
        registry.reserve(agent, 2, spec()),
        registry.reserve(agent, 2, spec())
    );
    let left = left.unwrap();
    assert_eq!(left.id, right.unwrap().id);
    assert_eq!(left.epoch, 0);
    dispatch(&registry, left.id).await;
    let ResourceObject::Workspace(mut claim) = left.object else {
        panic!("workspace expected")
    };
    claim.metadata.uid = Some("workspace-uid".into());
    claim.metadata.resource_version = Some("3".into());
    claim.spec.as_mut().unwrap().volume_name = Some("assigned-volume".into());
    registry
        .bind_object(left.id, &ResourceObject::Workspace(claim.clone()))
        .await
        .unwrap();
    claim.metadata.uid = Some("replacement-workspace".into());
    assert!(
        registry
            .bind_object(left.id, &ResourceObject::Workspace(claim))
            .await
            .is_err()
    );
    registry.record_missing(left.id).await.unwrap();
    assert_eq!(
        registry.reserve(agent, 2, spec()).await.unwrap().id,
        left.id
    );
    assert_eq!(
        registry.attempt(left.id).await.unwrap().uid.as_deref(),
        Some("workspace-uid")
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn changed_anchor_and_unplanned_security_changes_are_rejected() {
    let (_, registry, agent) = setup().await;
    assert!(registry.configure("another-namespace").await.is_err());
    let mut anchor = registry.configure("cubebot-test").await.unwrap();
    anchor.metadata.resource_version = Some("4".into());
    anchor.metadata.uid = Some("replacement-anchor".into());
    assert!(registry.bind_anchor(&anchor).await.is_err());
    let spec: PodSpec = serde_json::from_value(
        json!({"restartPolicy":"Never","containers":[{"name":"sandbox","image":"sandbox-test"}]}),
    )
    .unwrap();
    let attempt = registry
        .reserve(agent, 2, ResourceSpec::Pod(Box::new(spec)))
        .await
        .unwrap();
    dispatch(&registry, attempt.id).await;
    let ResourceObject::Pod(mut pod) = attempt.object else {
        panic!("pod expected")
    };
    pod.metadata.uid = Some("pod-uid".into());
    pod.metadata.resource_version = Some("5".into());
    pod.spec.as_mut().unwrap().host_network = Some(true);
    assert!(
        registry
            .bind_object(attempt.id, &ResourceObject::Pod(pod.clone()))
            .await
            .is_err()
    );
    pod.spec.as_mut().unwrap().host_network = None;
    pod.metadata.owner_references.as_mut().unwrap()[0].uid = "wrong-anchor".into();
    assert!(
        registry
            .bind_object(attempt.id, &ResourceObject::Pod(pod))
            .await
            .is_err()
    );
    assert!(registry.attempt(attempt.id).await.unwrap().uid.is_none());
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn unsent_plans_can_be_cancelled_but_dispatched_attempts_cannot() {
    let (_, registry, agent) = setup().await;
    let spec = || {
        ResourceSpec::Pod(Box::new(serde_json::from_value(json!({"restartPolicy":"Never","containers":[{"name":"sandbox","image":"sandbox-test"}]})).unwrap()))
    };
    let first = registry.reserve(agent, 2, spec()).await.unwrap();
    registry.cancel_planned(first.id).await.unwrap();
    let second = registry.reserve(agent, 2, spec()).await.unwrap();
    assert_ne!(first.id, second.id);
    dispatch(&registry, second.id).await;
    assert!(registry.cancel_planned(second.id).await.is_err());
    assert_eq!(
        registry.reserve(agent, 2, spec()).await.unwrap().id,
        second.id
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn restart_replaces_an_unsent_plan_without_abandoning_a_dispatched_attempt() {
    let (pool, registry, agent) = setup().await;
    let identity = Identity::new(pool.clone());
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let actor = identity.authenticate(&session.token).await.unwrap();
    let agents = Agents::new(pool);
    let spec = || {
        ResourceSpec::Pod(Box::new(serde_json::from_value(json!({"restartPolicy":"Never","containers":[{"name":"sandbox","image":"sandbox-test"}]})).unwrap()))
    };
    let first = registry.reserve(agent, 2, spec()).await.unwrap();
    agents
        .lifecycle(
            &actor,
            agent,
            &LifecycleCommand {
                request_id: Uuid::new_v4(),
                expected_revision: 2,
                action: LifecycleAction::Restart,
            },
        )
        .await
        .unwrap();
    assert!(registry.begin_create(first.id).await.is_err());
    let next = registry.reserve(agent, 3, spec()).await.unwrap();
    assert_ne!(first.id, next.id);
    assert_eq!(next.target_generation, first.target_generation + 1);
    assert_eq!(
        registry.attempt(first.id).await.unwrap().phase,
        cubebot_orchestrator::agents::kubernetes::Phase::Stopped
    );
    dispatch(&registry, next.id).await;
    agents
        .lifecycle(
            &actor,
            agent,
            &LifecycleCommand {
                request_id: Uuid::new_v4(),
                expected_revision: 3,
                action: LifecycleAction::Restart,
            },
        )
        .await
        .unwrap();
    assert!(registry.reserve(agent, 4, spec()).await.is_err());
    assert!(registry.cancel_planned(next.id).await.is_err());
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn preparation_is_retained_without_changing_retry_identity_or_accepting_drift() {
    let (pool, registry, agent) = setup().await;
    let spec: PodSpec = serde_json::from_value(
        json!({"restartPolicy":"Never","containers":[{"name":"sandbox","image":"sandbox-test"}]}),
    )
    .unwrap();
    let first = registry
        .reserve(agent, 2, ResourceSpec::Pod(Box::new(spec.clone())))
        .await
        .unwrap();
    assert!(registry.begin_create(first.id).await.is_err());
    let ResourceObject::Pod(mut preview) = first.object else {
        panic!("pod expected")
    };
    preview.spec.as_mut().unwrap().priority = Some(0);
    preview.spec.as_mut().unwrap().containers[0].termination_message_path =
        Some("/dev/termination-log".into());
    let prepared = ResourceObject::Pod(preview.clone());
    registry.record_prepared(first.id, &prepared).await.unwrap();
    registry.record_prepared(first.id, &prepared).await.unwrap();
    let repeated = Registry::new(pool)
        .reserve(agent, 2, ResourceSpec::Pod(Box::new(spec)))
        .await
        .unwrap();
    assert_eq!(first.id, repeated.id);
    assert!(repeated.prepared);
    assert_eq!(
        serde_json::to_value(repeated.object).unwrap(),
        serde_json::to_value(&prepared).unwrap()
    );
    let mut changed_default = preview.clone();
    changed_default.spec.as_mut().unwrap().containers[0].termination_message_policy =
        Some("File".into());
    assert!(
        registry
            .record_prepared(first.id, &ResourceObject::Pod(changed_default))
            .await
            .is_err()
    );
    preview.spec.as_mut().unwrap().host_network = Some(true);
    assert!(
        registry
            .record_prepared(first.id, &ResourceObject::Pod(preview))
            .await
            .is_err()
    );
    dispatch(&registry, first.id).await;
    assert!(registry.record_prepared(first.id, &prepared).await.is_err());
}

async fn dispatch(registry: &Registry, id: cubebot_orchestrator::agents::kubernetes::AttemptId) {
    let attempt = registry.attempt(id).await.unwrap();
    if !attempt.prepared {
        registry.record_prepared(id, &attempt.object).await.unwrap();
    }
    registry.begin_create(id).await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn server_preparation_precedes_dispatch_and_reconnect_uses_the_retained_manifest() {
    use axum::{Router, routing::post};
    use cubebot_orchestrator::agents::kubernetes::Kubernetes;
    let (pool, registry, agent) = setup().await;
    let calls: PreparationCalls = Default::default();
    let server_calls = calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/api/v1/namespaces/cubebot-test/pods", post(defaulted_pod))
                .with_state(server_calls),
        )
        .await
        .unwrap();
    });
    let config = kube::Config::new(url.parse().unwrap());
    let client = Kubernetes::new(config, "cubebot-test").unwrap();
    let spec: PodSpec = serde_json::from_value(
        json!({"restartPolicy":"Never","containers":[{"name":"sandbox","image":"sandbox-test"}]}),
    )
    .unwrap();
    let reserved = registry
        .reserve(agent, 2, ResourceSpec::Pod(Box::new(spec.clone())))
        .await
        .unwrap();
    let preview = client.prepare(&reserved.object).await.unwrap();
    assert!(preview.metadata().uid.is_none());
    registry
        .record_prepared(reserved.id, &preview)
        .await
        .unwrap();
    registry.begin_create(reserved.id).await.unwrap();
    let reconnected = Registry::new(pool)
        .reserve(agent, 2, ResourceSpec::Pod(Box::new(spec)))
        .await
        .unwrap();
    let ResourceObject::Pod(planned) = reconnected.object else {
        panic!("pod expected")
    };
    assert_eq!(planned.spec.as_ref().unwrap().priority, Some(0));
    let observed = client.create_pod(&planned).await.unwrap();
    registry
        .bind_object(reserved.id, &ResourceObject::Pod(Box::new(observed)))
        .await
        .unwrap();
    assert_eq!(
        registry.attempt(reserved.id).await.unwrap().uid.as_deref(),
        Some("physical-uid")
    );
    verify_preparation_calls(&calls.lock().unwrap());
    task.abort();
}

type PreparationCalls = std::sync::Arc<std::sync::Mutex<Vec<(bool, serde_json::Value)>>>;

async fn defaulted_pod(
    axum::extract::State(calls): axum::extract::State<PreparationCalls>,
    uri: axum::http::Uri,
    axum::Json(mut body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    calls.lock().unwrap().push((
        uri.query()
            .is_some_and(|query| query.contains("dryRun=All")),
        body.clone(),
    ));
    body["spec"]["priority"] = json!(0);
    body["metadata"]["uid"] = json!("physical-uid");
    body["metadata"]["resourceVersion"] = json!("7");
    axum::Json(body)
}

fn verify_preparation_calls(calls: &[(bool, serde_json::Value)]) {
    assert_eq!(calls.len(), 2);
    assert!(calls[0].0);
    assert!(!calls[1].0);
    assert!(calls[0].1["spec"]["priority"].is_null());
    assert_eq!(calls[1].1["spec"]["priority"], 0);
    assert!(calls[1].1["metadata"]["uid"].is_null());
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn competing_valid_preparations_cannot_change_the_committed_manifest() {
    let (_, registry, agent) = setup().await;
    let spec: PodSpec = serde_json::from_value(
        json!({"restartPolicy":"Never","containers":[{"name":"sandbox","image":"sandbox-test"}]}),
    )
    .unwrap();
    let attempt = registry
        .reserve(agent, 2, ResourceSpec::Pod(Box::new(spec)))
        .await
        .unwrap();
    let mut alternate = attempt.object.clone();
    let ResourceObject::Pod(pod) = &mut alternate else {
        panic!("pod expected")
    };
    pod.spec.as_mut().unwrap().priority = Some(0);
    let (left, right) = tokio::join!(
        registry.record_prepared(attempt.id, &attempt.object),
        registry.record_prepared(attempt.id, &alternate)
    );
    assert_ne!(left.is_ok(), right.is_ok());
    let retained = registry.attempt(attempt.id).await.unwrap();
    let expected = if left.is_ok() {
        &attempt.object
    } else {
        &alternate
    };
    assert_eq!(
        serde_json::to_value(retained.object).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn migration_preserves_legacy_dispatch_without_qualifying_unsent_cancellations() {
    let (pool, registry, agent) = setup().await;
    let spec = || {
        ResourceSpec::Pod(Box::new(serde_json::from_value(json!({"restartPolicy":"Never","containers":[{"name":"sandbox","image":"sandbox-test"}]})).unwrap()))
    };
    let cancelled = registry.reserve(agent, 2, spec()).await.unwrap();
    registry.cancel_planned(cancelled.id).await.unwrap();
    let live = registry.reserve(agent, 2, spec()).await.unwrap();
    dispatch(&registry, live.id).await;
    let ResourceObject::Pod(mut observed) = live.object else {
        panic!("pod expected")
    };
    observed.metadata.uid = Some("legacy-uid".into());
    observed.metadata.resource_version = Some("9".into());
    registry
        .bind_object(live.id, &ResourceObject::Pod(observed))
        .await
        .unwrap();
    // This database belongs only to this test. Reconstruct the previous schema boundary.
    sqlx::raw_sql("ALTER TABLE agent_resources DROP CONSTRAINT resource_prepared_before_dispatch, DROP COLUMN source_manifest, DROP COLUMN prepared").execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../migrations/0010_resource_preparation.sql"))
        .execute(&pool)
        .await
        .unwrap();
    assert!(!registry.attempt(cancelled.id).await.unwrap().prepared);
    let retained = registry.attempt(live.id).await.unwrap();
    assert!(retained.prepared);
    assert_eq!(retained.uid.as_deref(), Some("legacy-uid"));
    assert_eq!(
        serde_json::to_value(retained.intent).unwrap(),
        serde_json::to_value(retained.object).unwrap()
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_prepared_attempt_grants_dispatch_to_exactly_one_caller() {
    let (_, registry, agent) = setup().await;
    let spec: PodSpec = serde_json::from_value(
        json!({"restartPolicy":"Never","containers":[{"name":"sandbox","image":"sandbox-test"}]}),
    )
    .unwrap();
    let attempt = registry
        .reserve(agent, 2, ResourceSpec::Pod(Box::new(spec)))
        .await
        .unwrap();
    registry
        .record_prepared(attempt.id, &attempt.object)
        .await
        .unwrap();
    let (left, right) = tokio::join!(
        registry.begin_create(attempt.id),
        registry.begin_create(attempt.id)
    );
    assert_ne!(left.is_ok(), right.is_ok());
    assert!(registry.begin_create(attempt.id).await.is_err());
}

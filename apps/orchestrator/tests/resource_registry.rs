mod support;
use cubebot_orchestrator::{
    agents::{
        AgentId, Agents, CreateAgent, LifecycleAction, LifecycleCommand,
        kubernetes::{Registry, ResourceObject, ResourceSpec},
    },
    identity::Identity,
};
use k8s_openapi::api::core::v1::PodSpec;
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn reservation_replays_and_recovers_a_lost_create_response_without_transferring_uid() {
    let (pool, registry, agent) = setup().await;
    let spec:PodSpec = serde_json::from_value(json!({"restartPolicy":"Never","containers":[{"name":"runtime","image":"runtime-test"},{"name":"tools","image":"tools-test"}]})).unwrap();
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
    registry.begin_create(first.id).await.unwrap();
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
    pod.status = Some(serde_json::from_value(json!({"phase":"Failed","containerStatuses":[{"name":"runtime","image":"test","imageID":"test","ready":false,"restartCount":0,"containerID":"containerd://runtime-id","state":{"terminated":{"exitCode":1,"reason":"Error","containerID":"containerd://runtime-id","startedAt":"2026-10-26T11:00:00Z","finishedAt":"2026-10-26T12:00:01Z"}}},{"name":"tools","image":"test","imageID":"test","ready":false,"restartCount":0,"containerID":"containerd://tools-id","state":{"terminated":{"exitCode":0,"reason":"Completed","containerID":"containerd://tools-id","startedAt":"2026-10-26T11:00:00Z","finishedAt":"2026-10-26T12:00:01Z"}}}]})).unwrap());
}

async fn setup() -> (sqlx::PgPool, Registry, AgentId) {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let actor = identity.authenticate(&session.token).await.unwrap();
    let agents = Agents::new(pool.clone());
    let created = agents
        .create(
            &actor,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: "Registry agent".into(),
                instructions: "Preserve identity".into(),
            },
        )
        .await
        .unwrap();
    let agent = AgentId(created.id);
    agents
        .lifecycle(
            &actor,
            agent,
            &LifecycleCommand {
                request_id: Uuid::new_v4(),
                expected_revision: 1,
                action: LifecycleAction::Wake,
            },
        )
        .await
        .unwrap();
    let registry = Registry::new(pool.clone());
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
    registry.begin_create(left.id).await.unwrap();
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
    let spec: PodSpec = serde_json::from_value(json!({"restartPolicy":"Never","containers":[{"name":"runtime","image":"runtime-test"},{"name":"tools","image":"tools-test"}]})).unwrap();
    let attempt = registry
        .reserve(agent, 2, ResourceSpec::Pod(Box::new(spec)))
        .await
        .unwrap();
    registry.begin_create(attempt.id).await.unwrap();
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
        ResourceSpec::Pod(Box::new(serde_json::from_value(json!({"restartPolicy":"Never","containers":[{"name":"runtime","image":"runtime-test"},{"name":"tools","image":"tools-test"}]})).unwrap()))
    };
    let first = registry.reserve(agent, 2, spec()).await.unwrap();
    registry.cancel_planned(first.id).await.unwrap();
    let second = registry.reserve(agent, 2, spec()).await.unwrap();
    assert_ne!(first.id, second.id);
    registry.begin_create(second.id).await.unwrap();
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
        ResourceSpec::Pod(Box::new(serde_json::from_value(json!({"restartPolicy":"Never","containers":[{"name":"runtime","image":"runtime-test"},{"name":"tools","image":"tools-test"}]})).unwrap()))
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
    registry.begin_create(next.id).await.unwrap();
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

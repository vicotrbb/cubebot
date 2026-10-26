#[path = "support/kubernetes.rs"]
mod fixtures;
use cubebot_orchestrator::agents::kubernetes::{termination_proven, verify_pod};
use fixtures::{identity, pod};
use serde_json::json;

#[test]
fn recorded_uid_and_installation_anchor_are_required_for_ownership() {
    let identity = identity();
    let recorded = fixtures::recorded(identity.clone());
    let mut pod = pod(&identity);
    assert!(verify_pod(&pod, &recorded).is_ok());
    pod.metadata.uid = Some("replacement-uid".into());
    assert!(verify_pod(&pod, &recorded).is_err());
    pod.metadata.uid = Some("pod-uid".into());
    pod.metadata.owner_references.as_mut().unwrap()[0].uid = "other-installation".into();
    assert!(verify_pod(&pod, &recorded).is_err());
}

#[test]
fn ready_failed_or_missing_objects_do_not_establish_executor_termination() {
    let identity = identity();
    let mut pod = pod(&identity);
    assert!(!termination_proven(&pod));
    pod.status = Some(serde_json::from_value(json!({"phase":"Failed"})).unwrap());
    assert!(!termination_proven(&pod));
    pod.status = Some(serde_json::from_value(json!({"phase":"Failed","containerStatuses":[{"name":"runtime","image":"test","imageID":"test","ready":false,"restartCount":0,"state":{"terminated":{"exitCode":1}}},{"name":"tools","image":"test","imageID":"test","ready":false,"restartCount":0,"state":{"running":{}}}]})).unwrap());
    assert!(!termination_proven(&pod));
    fixtures::terminate(&mut pod);
    assert!(termination_proven(&pod));
    pod.spec.as_mut().unwrap().ephemeral_containers = Some(vec![Default::default()]);
    assert!(!termination_proven(&pod));
}

#[test]
fn unknown_container_status_and_restartable_pods_do_not_prove_termination() {
    let identity = identity();
    let mut pod = pod(&identity);
    fixtures::terminate(&mut pod);
    pod.status
        .as_mut()
        .unwrap()
        .container_statuses
        .as_mut()
        .unwrap()[0]
        .state
        .as_mut()
        .unwrap()
        .terminated
        .as_mut()
        .unwrap()
        .reason = Some("ContainerStatusUnknown".into());
    assert!(!termination_proven(&pod));
    fixtures::terminate(&mut pod);
    pod.spec.as_mut().unwrap().restart_policy = Some("Always".into());
    assert!(!termination_proven(&pod));
}

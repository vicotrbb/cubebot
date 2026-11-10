use crate::fixtures;
use cubebot_orchestrator::agents::kubernetes::{termination_proven, verify_pod};
use fixtures::{identity, pod};
use k8s_openapi::api::core::v1::Pod;
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
    pod.status = Some(serde_json::from_value(json!({"phase":"Failed","containerStatuses":[{"name":"sandbox","image":"test","imageID":"test","ready":false,"restartCount":0,"state":{"running":{}}}]})).unwrap());
    assert!(!termination_proven(&pod));
    fixtures::terminate(&mut pod);
    assert!(termination_proven(&pod));
    pod.spec.as_mut().unwrap().ephemeral_containers = Some(vec![Default::default()]);
    assert!(!termination_proven(&pod));
}

fn forget(pod: &mut Pod, reason: &str) {
    let status = pod.status.as_mut().unwrap();
    status.container_statuses.as_mut().unwrap()[0]
        .state
        .as_mut()
        .unwrap()
        .terminated
        .as_mut()
        .unwrap()
        .reason = Some(reason.into());
}

#[test]
fn a_container_the_runtime_forgot_proves_termination_only_on_a_failed_pod() {
    let identity = identity();
    for reason in ["Unknown", "ContainerStatusUnknown"] {
        // What the kubelet reports for each sandbox after its host reboots.
        let mut pod = pod(&identity);
        fixtures::terminate(&mut pod);
        forget(&mut pod, reason);
        assert!(termination_proven(&pod), "{reason} on a failed pod");
        pod.status.as_mut().unwrap().phase = Some("Succeeded".into());
        assert!(!termination_proven(&pod), "{reason} on a succeeded pod");
        fixtures::terminate(&mut pod);
        forget(&mut pod, reason);
        pod.spec.as_mut().unwrap().restart_policy = Some("Always".into());
        assert!(!termination_proven(&pod), "{reason} on a restartable pod");
    }
    let mut pod = pod(&identity);
    fixtures::terminate(&mut pod);
    forget(&mut pod, "Unrecognized");
    assert!(!termination_proven(&pod));
}

#[test]
fn restartable_pods_do_not_prove_termination() {
    let identity = identity();
    let mut pod = pod(&identity);
    fixtures::terminate(&mut pod);
    pod.spec.as_mut().unwrap().restart_policy = Some("Always".into());
    assert!(!termination_proven(&pod));
}

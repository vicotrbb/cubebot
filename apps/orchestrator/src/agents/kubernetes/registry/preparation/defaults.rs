use super::{Error, ResourceObject};
use k8s_openapi::api::core::v1::{Container, PodSpec, Toleration};

pub(super) fn verify(intent: &ResourceObject, preview: &ResourceObject) -> Result<(), Error> {
    if intent.metadata() != preview.metadata() {
        return Err(Error::OwnershipMismatch);
    }
    let matches = match (intent, preview) {
        (ResourceObject::Pod(intent), ResourceObject::Pod(preview)) => {
            if preview.status.is_some() {
                return Err(Error::OwnershipMismatch);
            }
            let mut expected = intent.spec.clone().ok_or(Error::InvalidConfiguration)?;
            let actual = preview.spec.as_ref().ok_or(Error::OwnershipMismatch)?;
            pod_defaults(&mut expected, actual);
            expected == *actual
        }
        (ResourceObject::Workspace(intent), ResourceObject::Workspace(preview)) => {
            if preview.status.is_some() {
                return Err(Error::OwnershipMismatch);
            }
            let mut expected = intent.spec.clone().ok_or(Error::InvalidConfiguration)?;
            let actual = preview.spec.as_ref().ok_or(Error::OwnershipMismatch)?;
            fill(
                &mut expected.volume_mode,
                &actual.volume_mode,
                "Filesystem".into(),
            );
            if expected.storage_class_name.is_none()
                && actual
                    .storage_class_name
                    .as_ref()
                    .is_some_and(|name| !name.is_empty())
            {
                expected.storage_class_name = actual.storage_class_name.clone();
            }
            expected == *actual
        }
        _ => false,
    };
    if !matches {
        return Err(Error::OwnershipMismatch);
    }
    Ok(())
}

fn pod_defaults(expected: &mut PodSpec, actual: &PodSpec) {
    fill(&mut expected.priority, &actual.priority, 0);
    fill(
        &mut expected.preemption_policy,
        &actual.preemption_policy,
        "PreemptLowerPriority".into(),
    );
    fill(
        &mut expected.dns_policy,
        &actual.dns_policy,
        "ClusterFirst".into(),
    );
    fill(
        &mut expected.scheduler_name,
        &actual.scheduler_name,
        "default-scheduler".into(),
    );
    fill(
        &mut expected.termination_grace_period_seconds,
        &actual.termination_grace_period_seconds,
        30,
    );
    fill(
        &mut expected.security_context,
        &actual.security_context,
        Default::default(),
    );
    fill(
        &mut expected.enable_service_links,
        &actual.enable_service_links,
        true,
    );
    for (expected, actual) in expected.containers.iter_mut().zip(&actual.containers) {
        container_defaults(expected, actual);
    }
    if let (Some(expected), Some(actual)) = (&mut expected.init_containers, &actual.init_containers)
    {
        for (expected, actual) in expected.iter_mut().zip(actual) {
            container_defaults(expected, actual);
        }
    }
    if expected.tolerations.is_none()
        && actual
            .tolerations
            .as_ref()
            .is_some_and(|values| default_tolerations(values))
    {
        expected.tolerations = actual.tolerations.clone();
    }
}
fn container_defaults(expected: &mut Container, actual: &Container) {
    fill(
        &mut expected.termination_message_path,
        &actual.termination_message_path,
        "/dev/termination-log".into(),
    );
    fill(
        &mut expected.termination_message_policy,
        &actual.termination_message_policy,
        "File".into(),
    );
    // Managed images are digest-pinned. Never infer defaults for a mutable :latest image.
    if expected
        .image
        .as_ref()
        .is_some_and(|image| image.contains("@sha256:"))
    {
        fill(
            &mut expected.image_pull_policy,
            &actual.image_pull_policy,
            "IfNotPresent".into(),
        );
    }
}
fn fill<T: PartialEq + Clone>(expected: &mut Option<T>, actual: &Option<T>, default: T) {
    if expected.is_none() && actual.as_ref() == Some(&default) {
        *expected = Some(default);
    }
}
fn default_tolerations(values: &[Toleration]) -> bool {
    let keys = [
        "node.kubernetes.io/not-ready",
        "node.kubernetes.io/unreachable",
    ];
    values.len() <= 2
        && values.iter().all(|value| {
            keys.contains(&value.key.as_deref().unwrap_or_default())
                && value.effect.as_deref() == Some("NoExecute")
                && value.operator.as_deref() == Some("Exists")
                && value.toleration_seconds == Some(300)
                && value.value.is_none()
        })
        && keys.iter().all(|key| {
            values
                .iter()
                .filter(|value| value.key.as_deref() == Some(key))
                .count()
                <= 1
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn pod() -> ResourceObject {
        ResourceObject::Pod(Box::new(serde_json::from_value(json!({"metadata":{"name":"owned-pod"},"spec":{"restartPolicy":"Never","containers":[{"name":"runtime","image":"registry.test/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},{"name":"tools","image":"registry.test/tools@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}]}})).unwrap()))
    }
    fn mutate(object: &ResourceObject, path: &str, value: Value) -> ResourceObject {
        let mut serialized = serde_json::to_value(object).unwrap();
        let (parent, key) = path.rsplit_once('/').unwrap();
        serialized.pointer_mut(parent).unwrap()[key] = value;
        match object {
            ResourceObject::Pod(_) => {
                ResourceObject::Pod(Box::new(serde_json::from_value(serialized).unwrap()))
            }
            ResourceObject::Workspace(_) => {
                ResourceObject::Workspace(Box::new(serde_json::from_value(serialized).unwrap()))
            }
        }
    }
    #[test]
    fn accepts_known_container_defaults_but_rejects_execution_changes() {
        let intent = pod();
        let defaulted = mutate(
            &intent,
            "/spec/containers/0/terminationMessagePath",
            json!("/dev/termination-log"),
        );
        let defaulted = mutate(
            &defaulted,
            "/spec/containers/0/imagePullPolicy",
            json!("IfNotPresent"),
        );
        assert!(verify(&intent, &defaulted).is_ok());
        for (path, value) in [
            ("/spec/hostNetwork", json!(true)),
            (
                "/spec/containers/0/env",
                json!([{"name":"INJECTED","value":"unplanned"}]),
            ),
            (
                "/spec/containers/0/securityContext",
                json!({"privileged":true}),
            ),
            (
                "/spec/containers/0/terminationMessagePath",
                json!("/runtime/secret"),
            ),
            (
                "/spec/volumes",
                json!([{"name":"host","hostPath":{"path":"/"}}]),
            ),
        ] {
            assert!(
                verify(&intent, &mutate(&defaulted, path, value)).is_err(),
                "{path}"
            );
        }
    }
    #[test]
    fn only_bounded_standard_node_tolerations_are_accepted() {
        let intent = pod();
        let toleration = json!({"key":"node.kubernetes.io/not-ready","effect":"NoExecute","operator":"Exists","tolerationSeconds":300});
        assert!(
            verify(
                &intent,
                &mutate(&intent, "/spec/tolerations", json!([toleration.clone()]))
            )
            .is_ok()
        );
        assert!(
            verify(
                &intent,
                &mutate(
                    &intent,
                    "/spec/tolerations",
                    json!([toleration.clone(), toleration])
                )
            )
            .is_err()
        );
        let unbounded = json!([{"key":"node.kubernetes.io/not-ready","effect":"NoExecute","operator":"Exists"}]);
        assert!(verify(&intent, &mutate(&intent, "/spec/tolerations", unbounded)).is_err());
    }
    #[test]
    fn storage_default_selection_does_not_allow_expansion_or_reselection() {
        let intent = ResourceObject::Workspace(Box::new(serde_json::from_value(json!({"metadata":{"name":"workspace"},"spec":{"accessModes":["ReadWriteOncePod"],"resources":{"requests":{"storage":"10Gi"}}}})).unwrap()));
        let defaulted = mutate(&intent, "/spec/storageClassName", json!("selected-csi"));
        let defaulted = mutate(&defaulted, "/spec/volumeMode", json!("Filesystem"));
        assert!(verify(&intent, &defaulted).is_ok());
        assert!(
            verify(
                &defaulted,
                &mutate(&defaulted, "/spec/storageClassName", json!("different-csi"))
            )
            .is_err()
        );
        assert!(
            verify(
                &intent,
                &mutate(
                    &defaulted,
                    "/spec/resources/requests/storage",
                    json!("100Gi")
                )
            )
            .is_err()
        );
    }
    #[test]
    fn physical_identity_status_and_metadata_mutations_are_rejected() {
        let intent = pod();
        assert!(
            verify(
                &intent,
                &mutate(&intent, "/metadata/uid", json!("dry-run-uid"))
            )
            .is_err()
        );
        assert!(
            verify(
                &intent,
                &mutate(&intent, "/metadata/labels", json!({"injected":"label"}))
            )
            .is_err()
        );
        assert!(
            verify(
                &intent,
                &mutate(&intent, "/status", json!({"phase":"Running"}))
            )
            .is_err()
        );
    }
}

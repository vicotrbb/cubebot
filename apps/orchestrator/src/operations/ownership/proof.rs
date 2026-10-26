use super::{Error, PodIdentity, Row};
use k8s_openapi::api::core::v1::{ContainerStateTerminated, ContainerStatus, Pod};
use serde_json::{Value, json};

pub(super) struct Current {
    pub pod: Pod,
    pub container_id: String,
}

pub(super) fn current(pod: Pod, identity: &PodIdentity) -> Result<Current, Error> {
    verify_identity(&pod, &identity.namespace, &identity.name, &identity.uid)?;
    let status = pod.status.as_ref().ok_or(Error::OwnershipMismatch)?;
    if pod.metadata.deletion_timestamp.is_some()
        || status.phase.as_deref() != Some("Running")
        || status.reason.as_deref() == Some("NodeLost")
    {
        return Err(Error::OwnershipMismatch);
    }
    let container = container(&pod)?;
    let state = container.state.as_ref().ok_or(Error::OwnershipMismatch)?;
    if state.terminated.is_some()
        || state.waiting.is_some()
        || state
            .running
            .as_ref()
            .and_then(|running| running.started_at.as_ref())
            .is_none()
    {
        return Err(Error::OwnershipMismatch);
    }
    let container_id = container
        .container_id
        .as_deref()
        .filter(|id| valid_id(id, 512))
        .ok_or(Error::OwnershipMismatch)?
        .to_owned();
    Ok(Current { pod, container_id })
}

pub(super) fn restart(pod: &Pod, previous: &Row) -> Result<Value, Error> {
    let container = container(pod)?;
    if container.container_id.as_deref() == Some(&previous.container_id) {
        return Err(Error::TerminationUnproven);
    }
    let last = container
        .last_state
        .as_ref()
        .ok_or(Error::TerminationUnproven)?;
    if last.running.is_some() || last.waiting.is_some() {
        return Err(Error::TerminationUnproven);
    }
    let exit = last.terminated.as_ref().ok_or(Error::TerminationUnproven)?;
    let proof = evidence(exit, previous)?;
    let start = container
        .state
        .as_ref()
        .and_then(|state| state.running.as_ref())
        .and_then(|state| state.started_at.as_ref())
        .ok_or(Error::TerminationUnproven)?;
    if exit
        .finished_at
        .as_ref()
        .is_none_or(|finished| finished.0 > start.0)
    {
        return Err(Error::TerminationUnproven);
    }
    Ok(proof)
}

pub(super) fn replacement(pod: &Pod, previous: &Row) -> Result<Value, Error> {
    verify_identity(
        pod,
        &previous.namespace,
        &previous.pod_name,
        &previous.pod_uid,
    )?;
    let status = pod.status.as_ref().ok_or(Error::TerminationUnproven)?;
    if pod.metadata.deletion_timestamp.is_none()
        || status.reason.as_deref() == Some("NodeLost")
        || !matches!(status.phase.as_deref(), Some("Succeeded" | "Failed"))
    {
        return Err(Error::TerminationUnproven);
    }
    let container = container(pod)?;
    if container.container_id.as_deref() != Some(&previous.container_id) {
        return Err(Error::TerminationUnproven);
    }
    let state = container.state.as_ref().ok_or(Error::TerminationUnproven)?;
    if state.running.is_some() || state.waiting.is_some() {
        return Err(Error::TerminationUnproven);
    }
    evidence(
        state
            .terminated
            .as_ref()
            .ok_or(Error::TerminationUnproven)?,
        previous,
    )
}

fn container(pod: &Pod) -> Result<&ContainerStatus, Error> {
    let spec = pod.spec.as_ref().ok_or(Error::OwnershipMismatch)?;
    if !spec
        .service_account_name
        .as_deref()
        .is_some_and(|name| !name.is_empty() && name != "default")
        || spec.restart_policy.as_deref() != Some("Always")
        || spec.containers.len() != 1
        || spec.containers[0].name != "orchestrator"
        || spec
            .ephemeral_containers
            .as_ref()
            .is_some_and(|containers| !containers.is_empty())
    {
        return Err(Error::OwnershipMismatch);
    }
    let containers = pod
        .status
        .as_ref()
        .and_then(|status| status.container_statuses.as_deref())
        .ok_or(Error::OwnershipMismatch)?;
    if containers.len() != 1 || containers[0].name != "orchestrator" {
        return Err(Error::OwnershipMismatch);
    }
    Ok(&containers[0])
}
fn verify_identity(pod: &Pod, namespace: &str, name: &str, uid: &str) -> Result<(), Error> {
    if !valid_id(uid, 255)
        || pod.metadata.namespace.as_deref() != Some(namespace)
        || pod.metadata.name.as_deref() != Some(name)
        || pod.metadata.uid.as_deref() != Some(uid)
    {
        return Err(Error::OwnershipMismatch);
    }
    Ok(())
}
fn valid_id(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
}
fn evidence(exit: &ContainerStateTerminated, previous: &Row) -> Result<Value, Error> {
    if exit.container_id.as_deref() != Some(&previous.container_id)
        || !matches!(
            exit.reason.as_deref(),
            Some("Completed" | "Error" | "OOMKilled")
        )
    {
        return Err(Error::TerminationUnproven);
    }
    let start = exit.started_at.as_ref().ok_or(Error::TerminationUnproven)?;
    let finish = exit
        .finished_at
        .as_ref()
        .ok_or(Error::TerminationUnproven)?;
    if finish.0 < start.0 {
        return Err(Error::TerminationUnproven);
    }
    Ok(
        json!({"previous_instance":previous.instance_id,"previous_generation":previous.generation,"namespace":previous.namespace,"pod_name":previous.pod_name,"pod_uid":previous.pod_uid,"container_id":previous.container_id,"reason":exit.reason,"started_at":start,"finished_at":finish}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contradictory_previous_container_state_cannot_prove_process_termination() {
        let previous = Row {
            instance_id: uuid::Uuid::from_u128(1),
            generation: 1,
            namespace: "cubebot-test".into(),
            pod_name: "orchestrator-a".into(),
            pod_uid: "pod-a".into(),
            container_id: "containerd://old".into(),
            released: false,
        };
        let pod: Pod = serde_json::from_value(json!({"metadata":{},"spec":{"restartPolicy":"Always","serviceAccountName":"cubebot-orchestrator","containers":[{"name":"orchestrator"}]},"status":{"containerStatuses":[{"name":"orchestrator","image":"test","imageID":"test","ready":true,"restartCount":1,"containerID":"containerd://new","state":{"running":{"startedAt":"2026-10-26T12:00:00Z"}},"lastState":{"running":{"startedAt":"2026-10-26T11:00:00Z"},"terminated":{"exitCode":1,"reason":"Error","containerID":"containerd://old","startedAt":"2026-10-26T11:00:00Z","finishedAt":"2026-10-26T11:59:00Z"}}}]}})).unwrap();
        assert!(restart(&pod, &previous).is_err());
    }
}

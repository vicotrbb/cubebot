use super::Error;
use crate::agents::AgentId;
use k8s_openapi::{
    api::core::v1::{ContainerStateTerminated, ContainerStatus, Pod},
    apimachinery::pkg::apis::meta::v1::ObjectMeta,
};
use uuid::Uuid;

pub const FINALIZER: &str = "cubebot.dev/execution-stopped";

#[derive(Clone)]
pub struct Identity {
    pub installation: Uuid,
    pub agent: AgentId,
    pub attempt: Uuid,
    pub anchor_name: String,
    pub anchor_uid: String,
}

pub struct RecordedPod {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub identity: Identity,
}

pub fn verify_pod(pod: &Pod, recorded: &RecordedPod) -> Result<(), Error> {
    let metadata = &pod.metadata;
    if metadata.namespace.as_deref() != Some(&recorded.namespace)
        || metadata.name.as_deref() != Some(&recorded.name)
        || metadata.uid.as_deref() != Some(&recorded.uid)
        || recorded.uid.is_empty()
    {
        return Err(Error::OwnershipMismatch);
    }
    verify_identity(metadata, &recorded.identity)
}

fn verify_identity(metadata: &ObjectMeta, identity: &Identity) -> Result<(), Error> {
    let labels = metadata.labels.as_ref().ok_or(Error::OwnershipMismatch)?;
    let expected = [
        (
            "cubebot.dev/installation",
            identity.installation.to_string(),
        ),
        ("cubebot.dev/agent", identity.agent.0.to_string()),
        ("cubebot.dev/attempt", identity.attempt.to_string()),
    ];
    if !expected
        .iter()
        .all(|(key, value)| labels.get(*key) == Some(value))
    {
        return Err(Error::OwnershipMismatch);
    }
    let owners = metadata.owner_references.as_deref().unwrap_or_default();
    let correct = owners.iter().any(|owner| {
        owner.api_version == "v1"
            && owner.kind == "ConfigMap"
            && owner.name == identity.anchor_name
            && owner.uid == identity.anchor_uid
            && !owner.uid.is_empty()
            && owner.controller == Some(true)
    });
    if !correct {
        return Err(Error::OwnershipMismatch);
    }
    Ok(())
}

pub fn termination_proven(pod: &Pod) -> bool {
    let Some(spec) = &pod.spec else {
        return false;
    };
    let Some(status) = &pod.status else {
        return false;
    };
    if spec.restart_policy.as_deref() != Some("Never")
        || status.reason.as_deref() == Some("NodeLost")
        || !matches!(status.phase.as_deref(), Some("Succeeded" | "Failed"))
        || spec
            .ephemeral_containers
            .as_ref()
            .is_some_and(|containers| !containers.is_empty())
    {
        return false;
    }
    let expected = ["runtime", "tools"];
    let shape = spec.containers.len() == expected.len()
        && expected.iter().all(|name| {
            spec.containers
                .iter()
                .any(|container| container.name == *name)
        });
    let containers = status.container_statuses.as_deref().unwrap_or_default();
    let stopped = containers.len() == expected.len()
        && expected.iter().all(|name| stopped_named(containers, name));
    shape && stopped && init_stopped(pod)
}

fn init_stopped(pod: &Pod) -> bool {
    let expected = pod
        .spec
        .as_ref()
        .and_then(|spec| spec.init_containers.as_deref())
        .unwrap_or_default();
    let observed = pod
        .status
        .as_ref()
        .and_then(|status| status.init_container_statuses.as_deref())
        .unwrap_or_default();
    expected.len() == observed.len()
        && expected
            .iter()
            .all(|container| stopped_named(observed, &container.name))
}

fn stopped_named(containers: &[ContainerStatus], name: &str) -> bool {
    containers
        .iter()
        .any(|container| container.name == name && terminated(container))
}

fn terminated(container: &ContainerStatus) -> bool {
    let Some(state) = &container.state else {
        return false;
    };
    let Some(exit) = &state.terminated else {
        return false;
    };
    if state.running.is_some() || state.waiting.is_some() {
        return false;
    }
    let identified = exit
        .container_id
        .as_deref()
        .is_some_and(|id| !id.is_empty() && container.container_id.as_deref() == Some(id));
    identified && known_exit(exit)
}

fn known_exit(exit: &ContainerStateTerminated) -> bool {
    let known = matches!(
        exit.reason.as_deref(),
        Some("Completed" | "Error" | "OOMKilled")
    );
    let timed = match (&exit.started_at, &exit.finished_at) {
        (Some(start), Some(end)) => end.0 >= start.0,
        _ => false,
    };
    known && timed
}

//! Kubernetes readiness is a reported condition, not native execution qualification.
use super::ResourceObject;
use k8s_openapi::api::core::v1::Pod;

pub(super) struct Observation<'a> {
    pub phase: Option<&'a str>,
    pub reason: Option<&'a str>,
    pub ready: bool,
    pub deleting: bool,
    pub recovery: bool,
}
impl<'a> Observation<'a> {
    pub fn of(object: &'a ResourceObject) -> Self {
        match object {
            ResourceObject::Workspace(_) => Self {
                phase: None,
                reason: None,
                ready: false,
                deleting: false,
                recovery: false,
            },
            ResourceObject::Pod(pod) => Self::pod(pod),
        }
    }
    fn pod(pod: &'a Pod) -> Self {
        let status = pod.status.as_ref();
        let phase = status.and_then(|status| status.phase.as_deref());
        let known = phase.filter(|phase| {
            matches!(
                *phase,
                "Pending" | "Running" | "Succeeded" | "Failed" | "Unknown"
            )
        });
        let reason = status
            .and_then(|status| status.reason.as_deref())
            .filter(|reason| {
                !reason.is_empty()
                    && reason.len() <= 128
                    && reason.bytes().all(|byte| byte.is_ascii_alphanumeric())
            });
        let deleting = pod.metadata.deletion_timestamp.is_some();
        let recovery = deleting
            || reason == Some("NodeLost")
            || phase.is_some_and(|phase| !matches!(phase, "Pending" | "Running"))
            || application_stopped(pod);
        let ready = !recovery
            && status
                .and_then(|status| status.conditions.as_deref())
                .unwrap_or_default()
                .iter()
                .any(|condition| condition.type_ == "Ready" && condition.status == "True");
        Self {
            phase: known,
            reason,
            ready,
            deleting,
            recovery,
        }
    }
}
fn application_stopped(pod: &Pod) -> bool {
    pod.status
        .as_ref()
        .and_then(|status| status.container_statuses.as_deref())
        .unwrap_or_default()
        .iter()
        .any(|container| {
            container.name == "sandbox"
                && container
                    .state
                    .as_ref()
                    .is_some_and(|state| state.terminated.is_some())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_exit_requires_recovery_even_while_the_pod_reports_running() {
        let pod: Pod = serde_json::from_value(serde_json::json!({
            "status": {
                "phase": "Running",
                "conditions": [{"type": "Ready", "status": "True"}],
                "containerStatuses": [{
                    "name": "sandbox", "image": "fixture", "imageID": "fixture",
                    "ready": false, "restartCount": 0,
                    "state": {"terminated": {"exitCode": 137}}
                }]
            }
        }))
        .unwrap();
        let observed = Observation::pod(&pod);
        assert!(observed.recovery);
        assert!(!observed.ready);
        assert!(!super::super::ownership::termination_proven(&pod));
    }
}

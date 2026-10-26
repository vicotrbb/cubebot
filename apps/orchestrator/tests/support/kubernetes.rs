use cubebot_orchestrator::agents::{
    AgentId,
    kubernetes::{Identity, RecordedPod},
};
use k8s_openapi::api::core::v1::Pod;
use serde_json::json;
use uuid::Uuid;

pub fn identity() -> Identity {
    Identity {
        installation: Uuid::new_v4(),
        agent: AgentId(Uuid::new_v4()),
        attempt: Uuid::new_v4(),
        anchor_name: "cubebot-installation".into(),
        anchor_uid: "anchor-uid".into(),
    }
}

pub fn pod(identity: &Identity) -> Pod {
    serde_json::from_value(json!({"metadata":{"name":"agent-test","namespace":"cubebot-test","uid":"pod-uid", "resourceVersion":"1", "finalizers":["cubebot.dev/execution-stopped", "other-controller.example/retain"], "labels":{"cubebot.dev/installation":identity.installation.to_string(),"cubebot.dev/agent":identity.agent.0.to_string(),"cubebot.dev/attempt":identity.attempt.to_string()},"ownerReferences":[{"apiVersion":"v1","kind":"ConfigMap","name":identity.anchor_name,"uid":identity.anchor_uid,"controller":true}]},"spec":{"restartPolicy":"Never","containers":[{"name":"runtime","image":"test"},{"name":"tools","image":"test"}]}})).unwrap()
}

pub fn recorded(identity: Identity) -> RecordedPod {
    RecordedPod {
        namespace: "cubebot-test".into(),
        name: "agent-test".into(),
        uid: "pod-uid".into(),
        identity,
    }
}

pub fn terminate(pod: &mut Pod) {
    pod.metadata.deletion_timestamp =
        Some(serde_json::from_value(json!("2026-10-26T12:00:00Z")).unwrap());
    pod.status = Some(serde_json::from_value(json!({"phase":"Failed","containerStatuses":[{"name":"runtime","image":"test","imageID":"test","ready":false,"restartCount":0,"containerID":"containerd://runtime-id","state":{"terminated":{"exitCode":1,"reason":"Error","containerID":"containerd://runtime-id","startedAt":"2026-10-26T11:00:00Z","finishedAt":"2026-10-26T12:00:01Z"}}},{"name":"tools","image":"test","imageID":"test","ready":false,"restartCount":0,"containerID":"containerd://tools-id","state":{"terminated":{"exitCode":0,"reason":"Completed","containerID":"containerd://tools-id","startedAt":"2026-10-26T11:00:00Z","finishedAt":"2026-10-26T12:00:01Z"}}}]})).unwrap());
}

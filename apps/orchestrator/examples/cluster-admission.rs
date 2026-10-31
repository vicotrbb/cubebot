//! Actual API-server/defaulting fixture. Never discovers a developer's Kubernetes context.
#[path = "../tests/support/registry.rs"]
mod registry_support;
#[path = "../tests/support/mod.rs"]
mod support;
use cubebot_orchestrator::agents::kubernetes::{
    Error, Kubernetes, Layout, ResourceObject, ResourceSpec,
};
use cubebot_protocol::network::NetworkPolicy;
#[path = "support/cluster.rs"]
mod cluster;
use cluster::{configuration, required, verify_namespace};
use serde_json::{Value, json};
/// Fixture failures end the run with their cause.
type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

#[tokio::main]
async fn main() -> Result<()> {
    let config = configuration().await?;
    // ADR 0084: the sandbox pod runs as root with NET_ADMIN, which baseline forbids, so it needs
    // a privileged namespace.
    let kvm = verify(&config, "cubebot-admission", true).await?;
    let kvm_baseline = verify(&config, "cubebot-admission-baseline", false).await?;
    println!(
        "{}",
        json!({"kvm_privileged":kvm,"kvm_baseline":kvm_baseline})
    );
    Ok(())
}
async fn verify(config: &kube::Config, namespace: &str, allowed: bool) -> Result<Value> {
    verify_namespace(config, namespace).await?;
    let (pool, registry, agent) = registry_support::setup().await;
    let api = Kubernetes::new(config.clone(), namespace)?;
    let anchor = registry.configure(namespace).await?;
    let anchor = api.create_anchor(&anchor).await?;
    registry.bind_anchor(&anchor).await?;
    let image = "registry.test/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let network = NetworkPolicy::new(vec![required("CUBEBOT_CLUSTER_DNS")?.parse()?])?;
    let layout = Layout::new(
        image,
        "cubebot-runtime",
        "https://orchestrator.cubebot.test",
        network,
    )?
    .with_control_ca("cubebot-control-ca.crt")?;
    let workspace = registry
        .reserve(
            agent,
            2,
            ResourceSpec::Workspace(Box::new(layout.workspace())),
        )
        .await?;
    let preview = api.prepare(&workspace.intent).await?;
    save_preview("workspace", &workspace.intent, &preview)?;
    registry.record_prepared(workspace.id, &preview).await?;
    let claim = workspace
        .object
        .metadata()
        .name
        .as_deref()
        .ok_or("workspace name missing")?;
    let pod = registry
        .reserve(agent, 2, ResourceSpec::Pod(Box::new(layout.pod(claim)?)))
        .await?;
    let result = api.prepare(&pod.intent).await;
    let report = check_preview(&api, &registry, &pod, result, namespace, allowed).await?;
    assert!(api.fetch_candidate(&workspace.intent).await?.is_none());
    pool.close().await;
    Ok(report)
}
async fn check_preview(
    api: &Kubernetes,
    registry: &cubebot_orchestrator::agents::kubernetes::Registry,
    pod: &cubebot_orchestrator::agents::kubernetes::Attempt,
    result: Result<ResourceObject, Error>,
    namespace: &str,
    allowed: bool,
) -> Result<Value> {
    if !allowed {
        assert!(
            matches!(result,Err(Error::Api(kube::Error::Api(status))) if status.code==403 && status.message.contains("violates PodSecurity"))
        );
        return Ok(json!({"namespace":namespace,"pod_security_rejected":true}));
    }
    let preview = result?;
    save_preview("pod", &pod.intent, &preview)?;
    registry.record_prepared(pod.id, &preview).await?;
    let repeated = api.prepare(&pod.intent).await?;
    registry.record_prepared(pod.id, &repeated).await?;
    let retained = registry.attempt(pod.id).await?;
    assert!(retained.prepared);
    assert!(retained.uid.is_none());
    assert!(api.fetch_candidate(&pod.intent).await?.is_none());
    Ok(
        json!({"namespace":namespace,"server_defaulting_accepted":true,"repeated_preparation_stable":true,"physical_pod_created":false}),
    )
}

fn save_preview(kind: &str, intent: &ResourceObject, preview: &ResourceObject) -> Result<()> {
    let directory = std::path::PathBuf::from(required("CUBEBOT_CLUSTER_ARTIFACTS")?);
    for (name, object) in [("intent", intent), ("preview", preview)] {
        std::fs::write(
            directory.join(format!("{kind}-{name}.json")),
            serde_json::to_vec_pretty(object)?,
        )?;
    }
    Ok(())
}

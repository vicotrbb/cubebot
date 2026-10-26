//! Actual API-server/defaulting fixture. Never discovers a developer's Kubernetes context.
#[path = "../tests/support/registry.rs"]
mod registry_support;
#[path = "../tests/support/mod.rs"]
mod support;
use cubebot_orchestrator::agents::kubernetes::{
    Error, Kubernetes, Layout, ResourceObject, ResourceSpec,
};
use cubebot_protocol::network::NetworkPolicy;
use kube::{Api, Client};
use serde_json::{Value, json};

#[tokio::main]
async fn main() {
    let config = configuration().await;
    let allowed = verify(&config, "cubebot-admission", true).await;
    let denied = verify(&config, "cubebot-admission-restricted", false).await;
    println!("{}", json!({"allowed":allowed,"restricted":denied}));
}
fn required(key: &str) -> String {
    std::env::var(key).expect("explicit isolated fixture configuration required")
}
async fn configuration() -> kube::Config {
    let source =
        kube::config::Kubeconfig::read_from(required("CUBEBOT_CLUSTER_KUBECONFIG")).unwrap();
    let options = kube::config::KubeConfigOptions {
        context: Some(required("CUBEBOT_CLUSTER_CONTEXT")),
        ..Default::default()
    };
    let config = kube::Config::from_custom_kubeconfig(source, &options)
        .await
        .unwrap();
    assert_eq!(config.cluster_url.host(), Some("127.0.0.1"));
    assert!(!config.accept_invalid_certs);
    config
}
async fn verify_namespace(config: &kube::Config, namespace: &str) {
    let client = Client::try_from(config.clone()).unwrap();
    let observed = Api::<k8s_openapi::api::core::v1::Namespace>::all(client)
        .get(namespace)
        .await
        .unwrap();
    assert_eq!(
        observed.metadata.labels.unwrap().get("cubebot.dev/fixture"),
        Some(&required("CUBEBOT_CLUSTER_FIXTURE"))
    );
}
async fn verify(config: &kube::Config, namespace: &str, allowed: bool) -> Value {
    verify_namespace(config, namespace).await;
    let (pool, registry, agent) = registry_support::setup().await;
    let api = Kubernetes::new(config.clone(), namespace).unwrap();
    let anchor = registry.configure(namespace).await.unwrap();
    let anchor = api.create_anchor(&anchor).await.unwrap();
    registry.bind_anchor(&anchor).await.unwrap();
    let image = "registry.test/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let network = NetworkPolicy::new(
        vec![],
        vec![required("CUBEBOT_CLUSTER_DNS").parse().unwrap()],
    )
    .unwrap();
    let layout = Layout::new(
        image,
        image,
        "cubebot-runtime",
        "https://orchestrator.cubebot.test",
        None,
        network,
    )
    .unwrap();
    let workspace = registry
        .reserve(
            agent,
            2,
            ResourceSpec::Workspace(Box::new(layout.workspace())),
        )
        .await
        .unwrap();
    let preview = api.prepare(&workspace.intent).await.unwrap();
    save_preview("workspace", &workspace.intent, &preview);
    registry
        .record_prepared(workspace.id, &preview)
        .await
        .unwrap();
    let claim = workspace.object.metadata().name.as_deref().unwrap();
    let pod = registry
        .reserve(
            agent,
            2,
            ResourceSpec::Pod(Box::new(layout.pod(claim).unwrap())),
        )
        .await
        .unwrap();
    let result = api.prepare(&pod.intent).await;
    let report = check_preview(&api, &registry, &pod, result, namespace, allowed).await;
    assert!(
        api.fetch_candidate(&workspace.intent)
            .await
            .unwrap()
            .is_none()
    );
    pool.close().await;
    report
}
async fn check_preview(
    api: &Kubernetes,
    registry: &cubebot_orchestrator::agents::kubernetes::Registry,
    pod: &cubebot_orchestrator::agents::kubernetes::Attempt,
    result: Result<ResourceObject, Error>,
    namespace: &str,
    allowed: bool,
) -> Value {
    if !allowed {
        assert!(
            matches!(result,Err(Error::Api(kube::Error::Api(status))) if status.code==403 && status.message.contains("violates PodSecurity"))
        );
        return json!({"namespace":namespace,"pod_security_rejected":true});
    }
    let preview = result.unwrap();
    save_preview("pod", &pod.intent, &preview);
    registry.record_prepared(pod.id, &preview).await.unwrap();
    let repeated = api.prepare(&pod.intent).await.unwrap();
    registry.record_prepared(pod.id, &repeated).await.unwrap();
    let retained = registry.attempt(pod.id).await.unwrap();
    assert!(retained.prepared);
    assert!(retained.uid.is_none());
    assert!(api.fetch_candidate(&pod.intent).await.unwrap().is_none());
    json!({"namespace":namespace,"server_defaulting_accepted":true,"repeated_preparation_stable":true,"physical_pod_created":false})
}

fn save_preview(kind: &str, intent: &ResourceObject, preview: &ResourceObject) {
    let directory = std::path::PathBuf::from(required("CUBEBOT_CLUSTER_ARTIFACTS"));
    for (name, object) in [("intent", intent), ("preview", preview)] {
        std::fs::write(
            directory.join(format!("{kind}-{name}.json")),
            serde_json::to_vec_pretty(object).unwrap(),
        )
        .unwrap();
    }
}

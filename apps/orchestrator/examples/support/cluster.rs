//! Explicit isolated cluster guard shared by actual-platform fixtures.
use kube::{Api, Client};
pub fn required(key: &str) -> String {
    std::env::var(key).expect("explicit isolated fixture configuration required")
}
pub async fn configuration() -> kube::Config {
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
pub async fn verify_namespace(config: &kube::Config, namespace: &str) {
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

//! Explicit isolated cluster guard shared by actual-platform fixtures.
use kube::{Api, Client};
/// Fixture failures end the run with their cause.
type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;
pub fn required(key: &str) -> Result<String> {
    std::env::var(key).map_err(|error| {
        format!("explicit isolated fixture configuration required: {error:?}").into()
    })
}
pub async fn configuration() -> Result<kube::Config> {
    let source = kube::config::Kubeconfig::read_from(required("CUBEBOT_CLUSTER_KUBECONFIG")?)?;
    let options = kube::config::KubeConfigOptions {
        context: Some(required("CUBEBOT_CLUSTER_CONTEXT")?),
        ..Default::default()
    };
    let config = kube::Config::from_custom_kubeconfig(source, &options).await?;
    assert_eq!(config.cluster_url.host(), Some("127.0.0.1"));
    assert!(!config.accept_invalid_certs);
    Ok(config)
}
pub async fn verify_namespace(config: &kube::Config, namespace: &str) -> Result<()> {
    let client = Client::try_from(config.clone())?;
    let observed = Api::<k8s_openapi::api::core::v1::Namespace>::all(client)
        .get(namespace)
        .await?;
    assert_eq!(
        observed
            .metadata
            .labels
            .ok_or("fixture namespace labels missing")?
            .get("cubebot.dev/fixture"),
        Some(&required("CUBEBOT_CLUSTER_FIXTURE")?)
    );
    Ok(())
}

//! Explicit installation configuration. Never infer a developer's default Kubernetes context.
use super::{Controller, Error, Layout};
use crate::operations::ownership::PodIdentity;
use serde::Deserialize;
use std::io::Read;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerConfig {
    connection: Connection,
    sandbox_image: String,
    backend: crate::agents::sandbox::Backend,
    runtime_service_account: String,
    orchestrator_origin: String,
    network: cubebot_protocol::network::NetworkPolicy,
    control_ca_config_map: Option<String>,
    #[serde(default)]
    terminal_enabled: bool,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Connection {
    InCluster,
    Kubeconfig { path: String, context: String },
}
impl ControllerConfig {
    pub fn parse(input: &str) -> Result<Self, Error> {
        if input.len() > 65536 {
            return Err(Error::InvalidConfiguration);
        }
        let config: Self = serde_json::from_str(input).map_err(|_| Error::InvalidConfiguration)?;
        config.layout()?;
        if let Connection::Kubeconfig { path, context } = &config.connection
            && (path.is_empty() || context.is_empty())
        {
            return Err(Error::InvalidConfiguration);
        }
        Ok(config)
    }
    pub fn environment() -> Result<Option<Self>, Error> {
        let Some(path) = std::env::var_os("CUBEBOT_CONTROLLER_CONFIG") else {
            return Ok(None);
        };
        let file = std::fs::File::open(path).map_err(|_| Error::InvalidConfiguration)?;
        let mut input = String::new();
        file.take(65537)
            .read_to_string(&mut input)
            .map_err(|_| Error::InvalidConfiguration)?;
        Self::parse(&input).map(Some)
    }
    pub async fn activate(self, pool: sqlx::PgPool) -> Result<Controller, Error> {
        let layout = self.layout()?;
        let identity = PodIdentity {
            namespace: required("CUBEBOT_POD_NAMESPACE")?,
            name: required("CUBEBOT_POD_NAME")?,
            uid: required("CUBEBOT_POD_UID")?,
        };
        let config = self.connection.load().await?;
        Controller::new(pool, config, layout, identity).await
    }
    fn layout(&self) -> Result<Layout, Error> {
        let layout = Layout::new(
            &self.sandbox_image,
            &self.runtime_service_account,
            &self.orchestrator_origin,
            self.backend,
            self.network.clone(),
        )?
        .with_terminal(self.terminal_enabled);
        match &self.control_ca_config_map {
            Some(name) => layout.with_control_ca(name),
            None => Ok(layout),
        }
    }
}
impl Connection {
    async fn load(self) -> Result<kube::Config, Error> {
        let config = match self {
            Self::InCluster => {
                kube::Config::incluster().map_err(|_| Error::InvalidConfiguration)?
            }
            Self::Kubeconfig { path, context } => {
                let input = kube::config::Kubeconfig::read_from(path)
                    .map_err(|_| Error::InvalidConfiguration)?;
                let options = kube::config::KubeConfigOptions {
                    context: Some(context),
                    ..Default::default()
                };
                kube::Config::from_custom_kubeconfig(input, &options)
                    .await
                    .map_err(|_| Error::InvalidConfiguration)?
            }
        };
        validate_transport(&config)?;
        Ok(config)
    }
}
fn validate_transport(config: &kube::Config) -> Result<(), Error> {
    let url = openidconnect::reqwest::Url::parse(&config.cluster_url.to_string())
        .map_err(|_| Error::InvalidConfiguration)?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]"));
    if config.accept_invalid_certs
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidConfiguration);
    }
    Ok(())
}
fn required(key: &str) -> Result<String, Error> {
    std::env::var(key).map_err(|_| Error::InvalidConfiguration)
}

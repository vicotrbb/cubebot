//! Where agent sandboxes run (docs/adr/sandbox.md#in-house-vmm-as-the-agent-sandbox). Every implementation launches the installation's declared
//! [`Runtime`].
mod docker;
pub(crate) use docker::LAYOUT_MISMATCH;
pub use docker::{Docker, DockerConfig};

use super::AgentId;
use crate::agents::kubernetes::Error;
use serde::Deserialize;
use sqlx::PgPool;
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncRead, AsyncWrite};

/// The installation's sandbox runtime, set at install for every agent: `kvm`, a guest of
/// `cubebot-sandbox`, or `container`, the agent image run directly, optionally under a named
/// RuntimeClass or Docker runtime.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Runtime {
    /// A struct variant, so a stray `class` is refused like any unknown field.
    Kvm {},
    Container {
        #[serde(default)]
        class: Option<String>,
    },
}
impl Runtime {
    /// A class must name a RuntimeClass, so it is a DNS subdomain, which is also a safe argument.
    pub fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Container { class: Some(class) } if !subdomain(class) => {
                Err(Error::InvalidConfiguration)
            }
            _ => Ok(()),
        }
    }
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Kvm {} => "kvm",
            Self::Container { .. } => "container",
        }
    }
    pub fn class(&self) -> Option<&str> {
        match self {
            Self::Container { class } => class.as_deref(),
            Self::Kvm {} => None,
        }
    }
    /// The host channel command (docs/adr/sandbox.md#desktop-as-a-vmm-device) inside this runtime's sandbox.
    fn desktop(&self) -> [&'static str; 2] {
        match self {
            Self::Kvm {} => ["/usr/local/bin/cubebot-sandbox", "desktop"],
            Self::Container { .. } => ["/usr/local/bin/cubebot-agent", "desktop"],
        }
    }
}

/// A Kubernetes DNS subdomain name.
pub(crate) fn subdomain(name: &str) -> bool {
    name.len() <= 253
        && name.split('.').all(|part| {
            !part.is_empty()
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Absent,
    Starting,
    Running,
    Stopping,
    Stopped,
}

/// A command attached inside the sandbox. Dropping it ends the session.
pub struct ExecChannel {
    pub stdin: Box<dyn AsyncWrite + Send + Unpin>,
    pub stdout: Box<dyn AsyncRead + Send + Unpin>,
    pub _session: Box<dyn Send>,
}

pub trait Sandbox {
    /// Creates the workspace and anything else that must outlive one run.
    fn create(&self, agent: AgentId) -> impl Future<Output = Result<(), Error>> + Send;
    fn start(&self, agent: AgentId) -> impl Future<Output = Result<(), Error>> + Send;
    /// Stops execution and keeps the workspace (docs/adr/memory.md#retain-durable-data-until-explicitly-deleted).
    fn stop(&self, agent: AgentId) -> impl Future<Output = Result<(), Error>> + Send;
    fn exec_channel(
        &self,
        agent: AgentId,
        command: &[&str],
    ) -> impl Future<Output = Result<ExecChannel, Error>> + Send;
    /// The retained workspace volume's name, if one exists.
    fn workspace(
        &self,
        agent: AgentId,
    ) -> impl Future<Output = Result<Option<String>, Error>> + Send;
    fn status(&self, agent: AgentId) -> impl Future<Output = Result<Status, Error>> + Send;
}

type Opening = Pin<Box<dyn Future<Output = Result<ExecChannel, Error>> + Send>>;

/// Opens the desktop host channel (docs/adr/sandbox.md#desktop-as-a-vmm-device) in an agent's running sandbox, whichever kind runs
/// it, and counts the people connected to each agent's desktop through this orchestrator.
#[derive(Clone)]
pub struct Desktop {
    open: Arc<dyn Fn(AgentId) -> Opening + Send + Sync>,
    pub viewers: Arc<Mutex<HashMap<AgentId, usize>>>,
}

impl Desktop {
    pub fn new<S: Sandbox + Send + Sync + 'static>(sandbox: Arc<S>, runtime: &Runtime) -> Self {
        let command = runtime.desktop();
        Self {
            open: Arc::new(move |agent| {
                let sandbox = sandbox.clone();
                Box::pin(async move { sandbox.exec_channel(agent, &command).await })
            }),
            viewers: Arc::default(),
        }
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub async fn open(&self, agent: AgentId) -> Result<ExecChannel, Error> {
        (self.open)(agent).await
    }
}

/// Converges one agent's sandbox to its recorded lifecycle intent.
#[tracing::instrument(skip_all, fields(agent = %agent.0))]
pub async fn drive<S: Sandbox + Sync>(
    sandbox: &S,
    pool: &PgPool,
    agent: AgentId,
) -> Result<(), Error> {
    let state = sqlx::query_scalar!(
        "
        SELECT desired_state
        FROM agents
        WHERE id = $1
        ",
        agent.0,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(Error::Conflict)?;
    if state != "running" {
        return sandbox.stop(agent).await;
    }
    sandbox.create(agent).await?;
    sandbox.start(agent).await
}

//! Where agent sandboxes run (ADR 0084). Every implementation launches `cubebot-sandbox`.
mod docker;
pub use docker::{Docker, DockerConfig};

use super::AgentId;
use crate::agents::kubernetes::Error;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::future::Future;
use tokio::io::{AsyncRead, AsyncWrite};

/// `process` runs the agent as a plain child for development and never carries an isolation claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Kvm,
    Process,
}
impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kvm => "kvm",
            Self::Process => "process",
        }
    }
}

/// A qualified installation refuses to start with the `process` backend.
pub(crate) fn require_isolation(qualified: bool, backend: Backend) -> Result<(), Error> {
    if qualified && backend == Backend::Process {
        return Err(Error::InvalidConfiguration);
    }
    Ok(())
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
    pub(crate) _session: Box<dyn Send>,
}

pub trait Sandbox {
    /// Creates the workspace and anything else that must outlive one run.
    fn create(&self, agent: AgentId) -> impl Future<Output = Result<(), Error>> + Send;
    fn start(&self, agent: AgentId) -> impl Future<Output = Result<(), Error>> + Send;
    /// Stops execution and keeps the workspace (ADR 0079).
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

/// Converges one agent's sandbox to its recorded lifecycle intent.
pub async fn drive<S: Sandbox + Sync>(
    sandbox: &S,
    pool: &PgPool,
    agent: AgentId,
) -> Result<(), Error> {
    let state: String = sqlx::query_scalar("SELECT desired_state FROM agents WHERE id = $1")
        .bind(agent)
        .fetch_optional(pool)
        .await?
        .ok_or(Error::Conflict)?;
    if state != "running" {
        return sandbox.stop(agent).await;
    }
    sandbox.create(agent).await?;
    sandbox.start(agent).await
}

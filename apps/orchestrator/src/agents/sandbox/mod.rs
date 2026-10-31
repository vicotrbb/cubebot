//! Where agent sandboxes run (ADR 0084). Every implementation launches `cubebot-sandbox`.
mod docker;
pub use docker::{Docker, DockerConfig};

use super::AgentId;
use crate::agents::kubernetes::Error;
use sqlx::PgPool;
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncRead, AsyncWrite};

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

type Opening = Pin<Box<dyn Future<Output = Result<ExecChannel, Error>> + Send>>;

/// Opens the desktop host channel (ADR 0085) in an agent's running sandbox, whichever kind runs
/// it, and counts the people connected to each agent's desktop through this orchestrator.
#[derive(Clone)]
pub struct Desktop {
    open: Arc<dyn Fn(AgentId) -> Opening + Send + Sync>,
    pub viewers: Arc<Mutex<HashMap<AgentId, usize>>>,
}

impl Desktop {
    pub fn new<S: Sandbox + Send + Sync + 'static>(sandbox: Arc<S>) -> Self {
        Self {
            open: Arc::new(move |agent| {
                let sandbox = sandbox.clone();
                Box::pin(async move {
                    let command = ["/usr/local/bin/cubebot-sandbox", "desktop"];
                    sandbox.exec_channel(agent, &command).await
                })
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

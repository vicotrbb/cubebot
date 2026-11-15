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
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

const RESPONSE_LIMIT: u64 = 32 << 20;

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
    fn desktop(&self) -> [&'static str; 2] {
        match self {
            Self::Kvm {} => ["/usr/local/bin/cubebot-sandbox", "desktop"],
            Self::Container { .. } => ["/usr/local/bin/cubebot-agent", "desktop"],
        }
    }
}

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

pub struct ExecChannel {
    pub stdin: Box<dyn AsyncWrite + Send + Unpin>,
    pub stdout: Box<dyn AsyncRead + Send + Unpin>,
    pub _session: Box<dyn Send>,
}

pub trait Sandbox {
    fn create(&self, agent: AgentId) -> impl Future<Output = Result<(), Error>> + Send;
    fn start(&self, agent: AgentId) -> impl Future<Output = Result<(), Error>> + Send;
    fn stop(&self, agent: AgentId) -> impl Future<Output = Result<(), Error>> + Send;
    fn exec_channel(
        &self,
        agent: AgentId,
        command: &[&str],
    ) -> impl Future<Output = Result<ExecChannel, Error>> + Send;
    fn workspace(
        &self,
        agent: AgentId,
    ) -> impl Future<Output = Result<Option<String>, Error>> + Send;
    fn status(&self, agent: AgentId) -> impl Future<Output = Result<Status, Error>> + Send;
}

type Opening = Pin<Box<dyn Future<Output = Result<ExecChannel, Error>> + Send>>;

#[derive(Clone)]
pub struct Desktop {
    open: Arc<dyn Fn(AgentId) -> Opening + Send + Sync>,
    viewers: Arc<Mutex<HashMap<AgentId, usize>>>,
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

    pub async fn channel(&self, agent: AgentId) -> Result<DesktopChannel, Error> {
        Ok(DesktopChannel::new(self.open(agent).await?))
    }

    fn viewers(&self) -> std::sync::MutexGuard<'_, HashMap<AgentId, usize>> {
        self.viewers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn watch(&self, agent: AgentId) {
        *self.viewers().entry(agent).or_default() += 1;
    }

    pub fn unwatch(&self, agent: AgentId) -> bool {
        let mut viewers = self.viewers();
        let count = viewers.entry(agent).or_default();
        *count = count.saturating_sub(1);
        let last = *count == 0;
        if last {
            viewers.remove(&agent);
        }
        last
    }
}

pub type ChannelError = Box<dyn std::error::Error + Send + Sync>;

pub struct DesktopChannel {
    input: Box<dyn AsyncWrite + Send + Unpin>,
    output: BufReader<Box<dyn AsyncRead + Send + Unpin>>,
    _session: Box<dyn Send>,
}

impl DesktopChannel {
    pub fn new(channel: ExecChannel) -> Self {
        Self {
            input: channel.stdin,
            output: BufReader::new(channel.stdout),
            _session: channel._session,
        }
    }

    pub async fn request(
        &mut self,
        request: &cubebot_protocol::desktop::HostRequest,
    ) -> Result<String, ChannelError> {
        let mut line = serde_json::to_vec(request)?;
        line.push(b'\n');
        self.input.write_all(&line).await?;
        self.input.flush().await?;
        let mut response = Vec::new();
        let mut bounded = (&mut self.output).take(RESPONSE_LIMIT);
        bounded.read_until(b'\n', &mut response).await?;
        if response.pop() != Some(b'\n') {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
        }
        Ok(String::from_utf8(response).map_err(|error| error.utf8_error())?)
    }
}

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
    if backing_off(pool, agent).await? {
        return Ok(());
    }
    sandbox.create(agent).await?;
    sandbox.start(agent).await
}

/// The delay doubles per undrained exit this hour, up to five minutes, so a crash loop neither spins
/// nor grows `agent_resources` quickly; a person's restart starts a new generation, and over.
async fn backing_off(pool: &PgPool, agent: AgentId) -> Result<bool, Error> {
    Ok(sqlx::query_scalar!(
        r#"
        SELECT COALESCE(max(r.updated_at) + make_interval(secs => LEAST(300, power(2, count(*)) - 2)) > clock_timestamp(), false) AS "waiting!"
        FROM agent_resources r
        JOIN agents a ON a.id = r.agent_id
        WHERE r.agent_id = $1
          AND r.kind = 'pod'
          AND r.phase = 'stopped'
          AND r.target_generation = a.restart_generation
          AND r.updated_at > clock_timestamp() - interval '1 hour'
          AND r.termination_proof->>'kind' IS DISTINCT FROM 'not_dispatched'
          AND NOT EXISTS (SELECT 1 FROM agent_runtime_drains d WHERE d.attempt_id = r.id AND d.acknowledged_at IS NOT NULL)
          AND NOT EXISTS (SELECT 1 FROM agent_resources o WHERE o.agent_id = $1 AND o.kind = 'pod' AND o.phase <> 'stopped')
        "#,
        agent.0,
    )
    .fetch_one(pool)
    .await?)
}

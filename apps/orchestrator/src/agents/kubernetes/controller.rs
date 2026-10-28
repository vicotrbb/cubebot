//! One serial reconciler, activated only while installation process ownership is held.
use super::{Error, Kubernetes, Layout, Reconciler};
use crate::{
    agents::{
        AgentId,
        runtime::RuntimeIdentity,
        sandbox::{Docker, DockerConfig, drive},
    },
    operations::ownership::{Owner, PodIdentity},
};
use sqlx::PgPool;
use std::time::Duration;
use tokio::sync::watch;
use uuid::Uuid;

pub struct Controller {
    owner: Owner,
    pool: PgPool,
    mode: Mode,
}
enum Mode {
    Kubernetes(Box<Lifecycle<Reconciler>>),
    Docker(Box<Lifecycle<Docker>>),
    DatabaseOnly,
}
struct Lifecycle<S> {
    sandbox: S,
    runtime: RuntimeIdentity,
}
impl Controller {
    pub async fn new(
        pool: PgPool,
        config: kube::Config,
        layout: Layout,
        identity: PodIdentity,
    ) -> Result<Self, Error> {
        let terminal_enabled = layout.terminal_enabled();
        let api = Kubernetes::new(config.clone(), &identity.namespace)?;
        let sandbox = Reconciler::new(pool.clone(), config.clone(), &identity.namespace, layout)?;
        let runtime = credentials(
            RuntimeIdentity::new(pool.clone(), config, &identity.namespace)?
                .with_terminal(terminal_enabled),
        )?;
        let owner = Owner::acquire(&pool, &api, identity).await?;
        Ok(Self {
            owner,
            pool,
            mode: Mode::Kubernetes(Box::new(Lifecycle { sandbox, runtime })),
        })
    }
    pub async fn database_only(pool: PgPool) -> Result<Self, Error> {
        let owner = Owner::database_only(&pool).await?;
        Ok(Self {
            owner,
            pool,
            mode: Mode::DatabaseOnly,
        })
    }
    /// Cluster-less sandboxes under development ownership; there is no pod to prove ownership.
    pub async fn docker(pool: PgPool, config: DockerConfig) -> Result<Self, Error> {
        let runtime = credentials(
            RuntimeIdentity::docker(pool.clone()).with_terminal(config.terminal_enabled()),
        )?;
        let owner = Owner::database_only(&pool).await?;
        Ok(Self {
            mode: Mode::Docker(Box::new(Lifecycle {
                sandbox: Docker::new(pool.clone(), config),
                runtime,
            })),
            owner,
            pool,
        })
    }
    pub fn runtime_router(&self) -> axum::Router {
        match &self.mode {
            Mode::Kubernetes(lifecycle) => crate::api::runtime_router(lifecycle.runtime.clone()),
            Mode::Docker(lifecycle) => crate::api::runtime_router(lifecycle.runtime.clone()),
            Mode::DatabaseOnly => axum::Router::new(),
        }
    }
    pub async fn run(&mut self, mut stop: watch::Receiver<bool>) -> Result<(), Error> {
        let mut cursor = AgentId(Uuid::nil());
        loop {
            if *stop.borrow() {
                return Ok(());
            }
            self.owner.check().await?;
            crate::work::Work::new(self.pool.clone())
                .recover_expired()
                .await?;
            let batch = self.batch(cursor).await?;
            self.reconcile_batch(&batch, &stop).await?;
            if batch.len() == 32 {
                cursor = *batch.last().expect("full batch has last agent");
            } else {
                cursor = AgentId(Uuid::nil());
                tokio::select! {
                    _ = stop.changed() => return Ok(()),
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {},
                }
            }
        }
    }
    async fn reconcile_batch(
        &mut self,
        batch: &[AgentId],
        stop: &watch::Receiver<bool>,
    ) -> Result<(), Error> {
        for agent in batch {
            if *stop.borrow() {
                return Ok(());
            }
            self.owner.check().await?;
            // Each effect is durably reserved before dispatch. Cancellation never replays it.
            if !matches!(
                tokio::time::timeout(Duration::from_secs(60), self.reconcile(*agent)).await,
                Ok(Ok(_))
            ) {
                tracing::warn!(agent = %agent.0,"agent lifecycle reconciliation held or failed");
            }
        }
        Ok(())
    }
    async fn reconcile(&self, agent: AgentId) -> Result<(), Error> {
        match &self.mode {
            Mode::Kubernetes(lifecycle) => drive(&lifecycle.sandbox, &self.pool, agent).await,
            Mode::Docker(lifecycle) => drive(&lifecycle.sandbox, &self.pool, agent).await,
            Mode::DatabaseOnly => Ok(()),
        }
    }
    async fn batch(&self, cursor: AgentId) -> Result<Vec<AgentId>, Error> {
        if matches!(self.mode, Mode::DatabaseOnly) {
            return Ok(Vec::new());
        }
        tokio::time::timeout(
            Duration::from_secs(5),
            sqlx::query_scalar("SELECT id FROM agents WHERE id > $1 ORDER BY id LIMIT 32")
                .bind(cursor)
                .fetch_all(&self.pool),
        )
        .await
        .map_err(|_| Error::DeadlineExceeded)?
        .map_err(Error::from)
    }
    pub async fn release(self) -> Result<(), Error> {
        self.owner.release().await
    }
}
fn credentials(runtime: RuntimeIdentity) -> Result<RuntimeIdentity, Error> {
    let Some(path) = std::env::var_os("CUBEBOT_CREDENTIAL_KEY_FILE") else {
        return Ok(runtime);
    };
    let credentials = crate::connections::Credentials::from_file(std::path::Path::new(&path))
        .map_err(|_| Error::InvalidConfiguration)?;
    Ok(runtime.with_credentials(credentials))
}

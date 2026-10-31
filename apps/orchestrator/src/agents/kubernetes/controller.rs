//! One serial reconciler, activated only while installation process ownership is held.
use super::{Error, Kubernetes, Layout, Reconciler};
use crate::{
    agents::{
        AgentId,
        runtime::RuntimeIdentity,
        sandbox::{Desktop, Docker, DockerConfig, drive},
    },
    operations::ownership::{Owner, PodIdentity},
};
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
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
    sandbox: Arc<S>,
    runtime: RuntimeIdentity,
}
/// The kubelet can publish this container's restart after the container starts. A failed start
/// would become the previous container the next start must prove, so retry briefly instead.
async fn acquire(pool: &PgPool, api: &Kubernetes, identity: PodIdentity) -> Result<Owner, Error> {
    for _ in 0..4 {
        match Owner::acquire(pool, api, identity.clone()).await {
            Err(Error::TerminationUnproven | Error::OwnershipMismatch) => {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            result => return result,
        }
    }
    Owner::acquire(pool, api, identity).await
}
impl Controller {
    pub async fn new(
        pool: PgPool,
        config: kube::Config,
        layout: Layout,
        identity: PodIdentity,
    ) -> Result<Self, Error> {
        let api = Kubernetes::new(config.clone(), &identity.namespace)?;
        let sandbox = Reconciler::new(pool.clone(), config.clone(), &identity.namespace, layout)?;
        let runtime = credentials(RuntimeIdentity::new(
            pool.clone(),
            config,
            &identity.namespace,
        )?)?;
        let owner = acquire(&pool, &api, identity).await?;
        Ok(Self {
            owner,
            pool,
            mode: Mode::Kubernetes(Box::new(Lifecycle {
                sandbox: Arc::new(sandbox),
                runtime,
            })),
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
        let runtime = credentials(RuntimeIdentity::docker(pool.clone()))?;
        let owner = Owner::database_only(&pool).await?;
        Ok(Self {
            mode: Mode::Docker(Box::new(Lifecycle {
                sandbox: Arc::new(Docker::new(pool.clone(), config)),
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
    /// The console's routes beside the runtime's, with desktop sessions reaching this
    /// controller's sandboxes, if it has any.
    pub fn routes(&self, console: axum::Router) -> axum::Router {
        let router = console.merge(self.runtime_router());
        let desktop = match &self.mode {
            Mode::Kubernetes(lifecycle) => Desktop::new(lifecycle.sandbox.clone()),
            Mode::Docker(lifecycle) => Desktop::new(lifecycle.sandbox.clone()),
            Mode::DatabaseOnly => return router,
        };
        router.layer(axum::Extension(desktop))
    }
    /// Reconciles agents, with room coordination running beside it while the installation's
    /// provider connection is configured.
    pub async fn run(&mut self, stop: watch::Receiver<bool>) -> Result<(), Error> {
        let connections = match &self.mode {
            Mode::Kubernetes(lifecycle) => lifecycle.runtime.connections.clone(),
            Mode::Docker(lifecycle) => lifecycle.runtime.connections.clone(),
            Mode::DatabaseOnly => None,
        };
        let coordinator = connections.map(|connections| {
            tokio::spawn(crate::work::coordinate(
                self.pool.clone(),
                connections,
                stop.clone(),
            ))
        });
        let result = self.reconcile_all(stop).await;
        if let Some(coordinator) = coordinator {
            coordinator.abort();
        }
        result
    }
    async fn reconcile_all(&mut self, mut stop: watch::Receiver<bool>) -> Result<(), Error> {
        let mut cursor = AgentId(Uuid::nil());
        loop {
            if *stop.borrow() {
                return Ok(());
            }
            self.owner.check().await?;
            crate::work::Work::new(self.pool.clone())
                .recover_interrupted()
                .await?;
            let batch = self.batch(cursor).await?;
            self.reconcile_batch(&batch, &stop).await?;
            match batch.last() {
                Some(last) if batch.len() == 32 => cursor = *last,
                _ => {
                    cursor = AgentId(Uuid::nil());
                    tokio::select! {
                        _ = stop.changed() => return Ok(()),
                        _ = tokio::time::sleep(Duration::from_secs(5)) => {},
                    }
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
            Mode::Kubernetes(lifecycle) => drive(&*lifecycle.sandbox, &self.pool, agent).await,
            Mode::Docker(lifecycle) => drive(&*lifecycle.sandbox, &self.pool, agent).await,
            Mode::DatabaseOnly => Ok(()),
        }
    }
    #[tracing::instrument(skip_all)]
    async fn batch(&self, cursor: AgentId) -> Result<Vec<AgentId>, Error> {
        if matches!(self.mode, Mode::DatabaseOnly) {
            return Ok(Vec::new());
        }
        tokio::time::timeout(
            Duration::from_secs(5),
            sqlx::query_scalar!(
                r#"
                SELECT id AS "id: AgentId"
                FROM agents
                WHERE id > $1
                ORDER BY id
                LIMIT 32
                "#,
                cursor.0,
            )
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
        .map_err(|error| Error::Configuration(error.into()))?;
    Ok(runtime.with_credentials(credentials))
}

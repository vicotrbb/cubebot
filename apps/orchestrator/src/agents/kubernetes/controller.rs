//! One serial reconciler, activated only while installation process ownership is held.
use super::{Error, Kubernetes, Layout, Reconciler};
use crate::{
    agents::{
        AgentId,
        runtime::RuntimeIdentity,
        sandbox::{Desktop, Docker, DockerConfig, drive},
    },
    api::Installation,
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
        let router = console
            .merge(self.runtime_router())
            .layer(axum::Extension(self.installation()));
        let desktop = match &self.mode {
            Mode::Kubernetes(lifecycle) => Desktop::new(
                lifecycle.sandbox.clone(),
                lifecycle.sandbox.layout.runtime(),
            ),
            Mode::Docker(lifecycle) => {
                Desktop::new(lifecycle.sandbox.clone(), lifecycle.sandbox.runtime())
            }
            Mode::DatabaseOnly => return router,
        };
        router.layer(axum::Extension(desktop))
    }
    /// What the console may know about this controller.
    pub fn installation(&self) -> Installation {
        match &self.mode {
            Mode::Kubernetes(lifecycle) => Installation {
                controller: "kubernetes",
                terminal_enabled: lifecycle.sandbox.layout.terminal(),
                computer_use_enabled: lifecycle.sandbox.layout.computer_use(),
                runtime: Some(lifecycle.sandbox.layout.runtime().clone()),
            },
            Mode::Docker(lifecycle) => Installation {
                controller: "docker",
                terminal_enabled: lifecycle.sandbox.terminal_enabled(),
                computer_use_enabled: lifecycle.sandbox.computer_use_enabled(),
                runtime: Some(lifecycle.sandbox.runtime().clone()),
            },
            Mode::DatabaseOnly => Installation::default(),
        }
    }
    /// Reconciles agents, with routine schedules running beside it, and room coordination while
    /// the installation's provider connection is configured.
    pub async fn run(&mut self, stop: watch::Receiver<bool>) -> Result<(), Error> {
        let connections = match &self.mode {
            Mode::Kubernetes(lifecycle) => lifecycle.runtime.connections.clone(),
            Mode::Docker(lifecycle) => lifecycle.runtime.connections.clone(),
            Mode::DatabaseOnly => None,
        };
        let tasks = connections.clone().map(|connections| {
            tokio::spawn(crate::work::tasks::orchestrate(
                self.pool.clone(),
                connections,
                stop.clone(),
            ))
        });
        let coordinator = connections.map(|connections| {
            tokio::spawn(crate::work::coordinate(
                self.pool.clone(),
                connections,
                stop.clone(),
            ))
        });
        let routines = tokio::spawn(crate::work::routines::schedule(
            self.pool.clone(),
            stop.clone(),
        ));
        let prices = tokio::spawn(crate::work::keep_prices(self.pool.clone(), stop.clone()));
        let result = self.reconcile_all(stop).await;
        if let Some(coordinator) = coordinator {
            coordinator.abort();
        }
        if let Some(tasks) = tasks {
            tasks.abort();
        }
        routines.abort();
        prices.abort();
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
                        () = tokio::time::sleep(Duration::from_secs(5)) => {},
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
                Ok(Ok(()))
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

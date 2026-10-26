//! One serial reconciler, activated only while installation process ownership is held.
use super::{Error, Kubernetes, Layout, Reconciler};
use crate::{
    agents::AgentId,
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
    Lifecycle(Box<Lifecycle>),
    DatabaseOnly,
}
struct Lifecycle {
    reconciler: Reconciler,
    runtime: crate::agents::runtime::RuntimeIdentity,
}
impl Controller {
    pub async fn new(
        pool: PgPool,
        config: kube::Config,
        layout: Layout,
        identity: PodIdentity,
    ) -> Result<Self, Error> {
        let api = Kubernetes::new(config.clone(), &identity.namespace)?;
        let reconciler =
            Reconciler::new(pool.clone(), config.clone(), &identity.namespace, layout)?;
        let runtime = crate::agents::runtime::RuntimeIdentity::new(
            pool.clone(),
            config,
            &identity.namespace,
        )?;
        let owner = Owner::acquire(&pool, &api, identity).await?;
        Ok(Self {
            owner,
            pool,
            mode: Mode::Lifecycle(Box::new(Lifecycle {
                reconciler,
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
    pub fn runtime_router(&self) -> axum::Router {
        match &self.mode {
            Mode::Lifecycle(lifecycle) => crate::api::runtime_router(lifecycle.runtime.clone()),
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
    async fn reconcile(&self, agent: AgentId) -> Result<super::ReconcileResult, Error> {
        match &self.mode {
            Mode::Lifecycle(lifecycle) => lifecycle.reconciler.reconcile(agent).await,
            Mode::DatabaseOnly => Ok(super::ReconcileResult::Idle),
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

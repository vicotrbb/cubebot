//! Explicit reconciliation steps. Construction never discovers or connects to a cluster.
use super::{
    Attempt, AttemptId, Error, Kubernetes, Layout, Phase, Registry, ResourceObject, ResourceSpec,
};
use crate::agents::AgentId;
use sqlx::PgPool;

pub enum ReconcileResult {
    Idle,
    AwaitingDrain {
        attempt: AttemptId,
    },
    Stopping {
        attempt: AttemptId,
    },
    Uncertain {
        attempt: AttemptId,
    },
    /// Object identities are bound. This does not imply scheduling, readiness or runtime authority.
    Provisioned {
        workspace: AttemptId,
        pod: AttemptId,
    },
}
pub struct Reconciler {
    pub(super) pool: PgPool,
    pub(super) registry: Registry,
    pub(super) api: Kubernetes,
    namespace: String,
    layout: Layout,
}
impl Reconciler {
    pub fn new(
        pool: PgPool,
        config: kube::Config,
        namespace: &str,
        layout: Layout,
    ) -> Result<Self, Error> {
        Ok(Self {
            registry: Registry::new(pool.clone()),
            pool,
            api: Kubernetes::new(config, namespace)?,
            namespace: namespace.into(),
            layout,
        })
    }
    pub async fn reconcile(&self, agent: AgentId) -> Result<ReconcileResult, Error> {
        let (revision, generation, state): (i64, i64, String) = sqlx::query_as(
            "SELECT revision,restart_generation,desired_state FROM agents WHERE id = $1",
        )
        .bind(agent)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(Error::Conflict)?;
        let verified = match self.inspect_pod(agent, generation, &state).await? {
            super::recovery::Inspection::Vacant => None,
            super::recovery::Inspection::Healthy(id) => Some(id),
            super::recovery::Inspection::Held(outcome) => return Ok(outcome),
        };
        if state != "running" {
            return Ok(ReconcileResult::Idle);
        }
        self.provision(agent, revision, verified).await
    }
    async fn provision(
        &self,
        agent: AgentId,
        revision: i64,
        verified: Option<AttemptId>,
    ) -> Result<ReconcileResult, Error> {
        self.anchor().await?;
        let workspace = self
            .registry
            .reserve(
                agent,
                revision,
                ResourceSpec::Workspace(Box::new(self.layout.workspace())),
            )
            .await?;
        let workspace_id = workspace.id;
        let Some(workspace) = self.ensure(workspace).await? else {
            return Ok(ReconcileResult::Uncertain {
                attempt: workspace_id,
            });
        };
        let name = workspace
            .object
            .metadata()
            .name
            .as_deref()
            .ok_or(Error::InvalidConfiguration)?;
        let pod = self
            .registry
            .reserve(
                agent,
                revision,
                ResourceSpec::Pod(Box::new(self.layout.pod(name)?)),
            )
            .await?;
        let pod_id = pod.id;
        if verified != Some(pod_id) && self.ensure(pod).await?.is_none() {
            return Ok(ReconcileResult::Uncertain { attempt: pod_id });
        }
        Ok(ReconcileResult::Provisioned {
            workspace: workspace_id,
            pod: pod_id,
        })
    }
    async fn anchor(&self) -> Result<(), Error> {
        let expected = self.registry.configure(&self.namespace).await?;
        let name = expected
            .metadata
            .name
            .as_deref()
            .ok_or(Error::InvalidConfiguration)?;
        let observed = match self.api.fetch_anchor(name).await? {
            Some(observed) => observed,
            None if expected.metadata.uid.is_none() => self.api.create_anchor(&expected).await?,
            None => return Err(Error::OwnershipMismatch),
        };
        self.registry.bind_anchor(&observed).await
    }
    async fn ensure(&self, attempt: Attempt) -> Result<Option<Attempt>, Error> {
        match attempt.phase {
            Phase::Planned => self.create(attempt).await.map(Some),
            Phase::Creating | Phase::Bound => self.observe(attempt).await,
            Phase::Stopped => Err(Error::Conflict),
        }
    }
    async fn create(&self, attempt: Attempt) -> Result<Attempt, Error> {
        if !attempt.prepared
            && let Err(error) = self.prepare(&attempt).await
        {
            self.registry
                .block(attempt.id, Phase::Planned, "preparation_failed")
                .await?;
            return Err(error);
        }
        self.registry.begin_create(attempt.id).await?;
        let dispatched = self.registry.attempt(attempt.id).await?;
        let created = match &dispatched.object {
            ResourceObject::Pod(pod) => self
                .api
                .create_pod(pod)
                .await
                .map(|pod| ResourceObject::Pod(Box::new(pod))),
            ResourceObject::Workspace(claim) => self
                .api
                .create_workspace(claim)
                .await
                .map(|claim| ResourceObject::Workspace(Box::new(claim))),
        };
        let created = match created {
            Ok(created) => created,
            Err(error) => {
                self.registry
                    .block(attempt.id, Phase::Creating, "create_outcome_unknown")
                    .await?;
                return Err(error);
            }
        };
        self.bind(dispatched, created).await
    }
    async fn prepare(&self, attempt: &Attempt) -> Result<(), Error> {
        let preview = self.api.prepare(&attempt.intent).await?;
        self.registry.record_prepared(attempt.id, &preview).await
    }
    pub(super) async fn observe(&self, attempt: Attempt) -> Result<Option<Attempt>, Error> {
        let Some(observed) = self.api.fetch_candidate(&attempt.object).await? else {
            self.registry.record_missing(attempt.id).await?;
            return Ok(None);
        };
        self.bind(attempt, observed).await.map(Some)
    }
    pub(super) async fn bind(
        &self,
        attempt: Attempt,
        observed: ResourceObject,
    ) -> Result<Attempt, Error> {
        if let Err(error) = self.registry.bind_object(attempt.id, &observed).await {
            self.registry
                .block(
                    attempt.id,
                    attempt.phase,
                    "ownership_or_specification_conflict",
                )
                .await?;
            return Err(error);
        }
        self.registry.attempt(attempt.id).await
    }
}

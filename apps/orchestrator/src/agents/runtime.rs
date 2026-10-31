//! Runtime identity for one sandbox incarnation, and its channel.
mod channel;
mod drain;
mod sessions;
use super::{
    AgentId,
    kubernetes::{Attempt, AttemptId, Error, Kubernetes, Registry, ResourceObject, ReviewedPod},
};
pub use drain::{DrainRequest, RuntimeControl, RuntimeDisposition};
use serde::Serialize;
pub use sessions::revoke;
pub(crate) use sessions::{sandbox_digest, sandbox_token};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct RuntimeIdentity {
    pub(crate) pool: PgPool,
    /// `None` serves cluster-less Docker sandboxes, which bootstrap with a per-container token.
    api: Option<Kubernetes>,
    pub(crate) connections: Option<crate::connections::Connections>,
}
#[derive(Serialize)]
pub struct RuntimeSession {
    pub token: String,
    pub agent: AgentId,
    pub attempt: AttemptId,
    pub epoch: i64,
    pub version: i64,
}
#[derive(Debug, Serialize)]
pub struct RuntimeActor {
    pub agent: AgentId,
    pub attempt: AttemptId,
    pub epoch: i64,
    pub version: i64,
}
pub enum RuntimeAccess {
    Active,
    Recovery,
}
impl RuntimeIdentity {
    pub fn new(pool: PgPool, config: kube::Config, namespace: &str) -> Result<Self, Error> {
        Ok(Self {
            pool,
            api: Some(Kubernetes::new(config, namespace)?),
            connections: None,
        })
    }
    pub fn docker(pool: PgPool) -> Self {
        Self {
            pool,
            api: None,
            connections: None,
        }
    }
    pub fn with_credentials(mut self, credentials: crate::connections::Credentials) -> Self {
        self.connections = Some(crate::connections::Connections::new(
            self.pool.clone(),
            credentials,
        ));
        self
    }
    #[tracing::instrument(skip_all)]
    pub async fn bootstrap(
        &self,
        projected_token: &str,
        instance: Uuid,
    ) -> Result<RuntimeSession, Error> {
        if instance.is_nil() {
            return Err(Error::Unauthorized);
        }
        tokio::time::timeout(
            std::time::Duration::from_secs(20),
            self.bootstrap_verified(projected_token, instance),
        )
        .await
        .map_err(|_| Error::DeadlineExceeded)?
    }
    async fn bootstrap_verified(
        &self,
        projected_token: &str,
        instance: Uuid,
    ) -> Result<RuntimeSession, Error> {
        let Some(api) = &self.api else {
            let (attempt, reviewed) = self.verify_container(projected_token).await?;
            return self.issue(attempt, reviewed, instance).await;
        };
        let reviewed = api.review_runtime_token(projected_token).await?;
        let attempt = self.verify_pod(api, &reviewed).await?;
        self.issue(attempt, reviewed, instance).await
    }
    async fn verify_container(&self, token: &str) -> Result<(Attempt, ReviewedPod), Error> {
        let row = sqlx::query!(
            r#"
            SELECT r.id AS "id: AttemptId", r.name, r.uid AS "uid!"
            FROM docker_sandbox_tokens t
            JOIN agent_resources r ON r.id = t.attempt_id
            WHERE t.token_hash = $1
              AND r.kind = 'pod'
              AND r.namespace = 'docker'
              AND r.phase = 'bound'
              AND r.kubernetes_phase = 'Running'
            "#,
            sessions::sandbox_digest(token)?,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(Error::Unauthorized)?;
        let attempt = Registry::new(self.pool.clone()).attempt(row.id).await?;
        let reviewed = ReviewedPod {
            namespace: "docker".into(),
            service_account: "docker".into(),
            service_account_uid: "docker".into(),
            pod_name: row.name,
            pod_uid: row.uid,
        };
        Ok((attempt, reviewed))
    }
    async fn verify_pod(&self, api: &Kubernetes, identity: &ReviewedPod) -> Result<Attempt, Error> {
        let id = sqlx::query_scalar!(
            r#"
            SELECT id AS "id: AttemptId"
            FROM agent_resources
            WHERE kind = 'pod'
              AND phase = 'bound'
              AND namespace = $1
              AND name = $2
              AND uid = $3
            "#,
            identity.namespace,
            identity.pod_name,
            identity.pod_uid,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(Error::Unauthorized)?;
        let registry = Registry::new(self.pool.clone());
        let expected = registry.attempt(id).await?;
        let observed = api
            .fetch_candidate(&expected.object)
            .await?
            .ok_or(Error::Unauthorized)?;
        let ResourceObject::Pod(pod) = &observed else {
            return Err(Error::Unauthorized);
        };
        if pod.metadata.deletion_timestamp.is_some()
            || pod
                .spec
                .as_ref()
                .and_then(|spec| spec.service_account_name.as_deref())
                != Some(&identity.service_account)
            || pod
                .status
                .as_ref()
                .and_then(|status| status.phase.as_deref())
                != Some("Running")
        {
            return Err(Error::Unauthorized);
        }
        registry.bind_object(id, &observed).await?;
        Ok(expected)
    }
}

impl From<RuntimeActor> for cubebot_protocol::runtime::RuntimeActor {
    fn from(value: RuntimeActor) -> Self {
        Self {
            agent: value.agent.0,
            attempt: value.attempt.0,
            epoch: value.epoch,
            version: value.version,
        }
    }
}
impl From<RuntimeSession> for cubebot_protocol::runtime::RuntimeSession {
    fn from(value: RuntimeSession) -> Self {
        Self {
            token: value.token,
            agent: value.agent.0,
            attempt: value.attempt.0,
            epoch: value.epoch,
            version: value.version,
        }
    }
}

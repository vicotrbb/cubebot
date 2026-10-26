//! Short-lived runtime identity. It does not grant a turn or permission to execute effects.
mod sessions;
use super::{
    AgentId,
    kubernetes::{Attempt, AttemptId, Error, Kubernetes, Registry, ResourceObject, ReviewedPod},
};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct RuntimeIdentity {
    pool: PgPool,
    api: Kubernetes,
}
#[derive(Serialize)]
pub struct RuntimeSession {
    pub token: String,
    pub agent: AgentId,
    pub attempt: AttemptId,
    pub epoch: i64,
    pub version: i64,
    /// Upper bound measured from the client's request start, never from response receipt.
    pub expires_in_seconds: u64,
}
#[derive(Debug, Serialize, sqlx::FromRow)]
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
            api: Kubernetes::new(config, namespace)?,
        })
    }
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
        let reviewed = self.api.review_runtime_token(projected_token).await?;
        let attempt = self.verify_pod(&reviewed).await?;
        self.issue(attempt, reviewed, instance).await
    }
    async fn verify_pod(&self, identity: &ReviewedPod) -> Result<Attempt, Error> {
        let id: AttemptId = sqlx::query_scalar("SELECT id FROM agent_resources WHERE kind = 'pod' AND phase = 'bound' AND namespace = $1 AND name = $2 AND uid = $3")
            .bind(&identity.namespace).bind(&identity.pod_name).bind(&identity.pod_uid).fetch_optional(&self.pool).await?.ok_or(Error::Unauthorized)?;
        let registry = Registry::new(self.pool.clone());
        let expected = registry.attempt(id).await?;
        let observed = self
            .api
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

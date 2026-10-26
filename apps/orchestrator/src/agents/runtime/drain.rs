//! Drain acknowledgement is not proof of process termination or synchronized recovery records.
use super::{Error, RuntimeAccess, RuntimeActor, RuntimeIdentity};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize, sqlx::FromRow)]
pub struct DrainRequest {
    pub request_id: Uuid,
    pub acknowledged: bool,
}
pub use cubebot_protocol::runtime::RuntimeDisposition;
#[derive(Serialize)]
pub struct RuntimeControl {
    pub actor: RuntimeActor,
    pub disposition: RuntimeDisposition,
    pub drain: Option<DrainRequest>,
}

impl RuntimeIdentity {
    pub async fn control(&self, token: &str) -> Result<RuntimeControl, Error> {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.control_locked(token),
        )
        .await
        .map_err(|_| Error::DeadlineExceeded)?
    }

    async fn control_locked(&self, token: &str) -> Result<RuntimeControl, Error> {
        let actor = self.authenticate(token, RuntimeAccess::Recovery).await?;
        let mut transaction = self.lock_runtime(&actor).await?;
        self.recheck(token, &actor, &mut transaction).await?;
        let drain = sqlx::query_as::<_, DrainRequest>("SELECT request_id,acknowledged_at IS NOT NULL AS acknowledged FROM agent_runtime_drains WHERE attempt_id = $1")
            .bind(actor.attempt).fetch_optional(&mut *transaction).await?;
        let available: bool = sqlx::query_scalar("SELECT a.desired_state = 'running' AND a.restart_generation = r.target_generation AND r.blocked_reason IS NULL FROM agent_resources r JOIN agents a ON a.id = r.agent_id WHERE r.id = $1")
            .bind(actor.attempt).fetch_one(&mut *transaction).await?;
        let disposition = match (&drain, available) {
            (Some(_), _) => RuntimeDisposition::Drain,
            (None, true) => RuntimeDisposition::Available,
            (None, false) => RuntimeDisposition::Hold,
        };
        transaction.commit().await?;
        Ok(RuntimeControl {
            actor,
            disposition,
            drain,
        })
    }

    /// Called only after all lanes stop starting effects and active work has settled.
    /// The controller must separately establish journal acceptance and physical termination.
    pub async fn acknowledge_drain(
        &self,
        token: &str,
        request: Uuid,
    ) -> Result<DrainRequest, Error> {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.acknowledge_locked(token, request),
        )
        .await
        .map_err(|_| Error::DeadlineExceeded)?
    }

    async fn acknowledge_locked(&self, token: &str, request: Uuid) -> Result<DrainRequest, Error> {
        let actor = self.authenticate(token, RuntimeAccess::Recovery).await?;
        let mut transaction = self.lock_runtime(&actor).await?;
        self.recheck(token, &actor, &mut transaction).await?;
        let report = sqlx::query_as("UPDATE agent_runtime_drains SET acknowledged_at = COALESCE(acknowledged_at,clock_timestamp()), acknowledged_session_version = COALESCE(acknowledged_session_version,$3) WHERE attempt_id = $1 AND request_id = $2 RETURNING request_id,true AS acknowledged")
            .bind(actor.attempt).bind(request).bind(actor.version).fetch_optional(&mut *transaction).await?.ok_or(Error::Conflict)?;
        transaction.commit().await?;
        Ok(report)
    }

    async fn lock_runtime(
        &self,
        actor: &RuntimeActor,
    ) -> Result<sqlx::Transaction<'_, sqlx::Postgres>, Error> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
            .execute(&mut *transaction)
            .await?;
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM agents WHERE id = $1 FOR UPDATE")
            .bind(actor.agent)
            .fetch_one(&mut *transaction)
            .await?;
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM agent_resources WHERE id = $1 AND agent_id = $2 FOR UPDATE",
        )
        .bind(actor.attempt)
        .bind(actor.agent)
        .fetch_one(&mut *transaction)
        .await?;
        Ok(transaction)
    }

    async fn recheck(
        &self,
        token: &str,
        actor: &RuntimeActor,
        transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<(), Error> {
        sqlx::query_scalar::<_,Uuid>("SELECT r.id FROM agent_runtime_sessions s JOIN agent_resources r ON r.id = s.attempt_id WHERE r.id = $1 AND s.version = $2 AND s.token_hash = $3 AND s.expires_at > clock_timestamp() AND r.kind = 'pod' AND r.phase = 'bound' AND r.uid = s.pod_uid")
            .bind(actor.attempt).bind(actor.version).bind(super::sessions::digest(token)).fetch_optional(&mut **transaction).await?.ok_or(Error::Unauthorized)?;
        Ok(())
    }
}

impl From<DrainRequest> for cubebot_protocol::runtime::DrainRequest {
    fn from(value: DrainRequest) -> Self {
        Self {
            request_id: value.request_id,
            acknowledged: value.acknowledged,
        }
    }
}
impl From<RuntimeControl> for cubebot_protocol::runtime::RuntimeControl {
    fn from(value: RuntimeControl) -> Self {
        Self {
            actor: value.actor.into(),
            disposition: value.disposition,
            drain: value.drain.map(Into::into),
        }
    }
}

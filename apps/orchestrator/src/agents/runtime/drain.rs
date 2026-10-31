//! Drain acknowledgement is not proof of process termination or synchronized recovery records.
use super::{Error, RuntimeAccess, RuntimeActor, RuntimeIdentity};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
pub struct DrainRequest {
    pub request_id: Uuid,
    pub acknowledged: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeDisposition {
    Available,
    Hold,
    Drain,
}
#[derive(Serialize)]
pub struct RuntimeControl {
    pub actor: RuntimeActor,
    pub disposition: RuntimeDisposition,
    pub drain: Option<DrainRequest>,
}

impl RuntimeIdentity {
    #[tracing::instrument(skip_all)]
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
        let drain = sqlx::query_as!(
            DrainRequest,
            r#"
            SELECT request_id, acknowledged_at IS NOT NULL AS "acknowledged!"
            FROM agent_runtime_drains
            WHERE attempt_id = $1
            "#,
            actor.attempt.0,
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let available = sqlx::query_scalar!(
            r#"
            SELECT COALESCE(
                (SELECT active FROM runtime_session_access WHERE attempt_id = $1),
                false
            ) AS "available!"
            "#,
            actor.attempt.0,
        )
        .fetch_one(&mut *transaction)
        .await?;
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
    #[tracing::instrument(skip_all, fields(request = %request))]
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
        let report = sqlx::query_as!(
            DrainRequest,
            r#"
            UPDATE agent_runtime_drains
            SET acknowledged_at = COALESCE(acknowledged_at, clock_timestamp()),
                acknowledged_session_version = COALESCE(acknowledged_session_version, $3)
            WHERE attempt_id = $1
              AND request_id = $2
            RETURNING request_id, true AS "acknowledged!"
            "#,
            actor.attempt.0,
            request,
            actor.version,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(Error::Conflict)?;
        transaction.commit().await?;
        Ok(report)
    }

    pub(super) async fn lock_runtime(
        &self,
        actor: &RuntimeActor,
    ) -> Result<sqlx::Transaction<'_, sqlx::Postgres>, Error> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query!("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
            .execute(&mut *transaction)
            .await?;
        // Tests find this lock wait in pg_stat_activity by its `SELECT id FROM agents` prefix.
        sqlx::query_scalar!(
            "SELECT id FROM agents WHERE id = $1 FOR UPDATE",
            actor.agent.0
        )
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query_scalar!(
            "
            SELECT id
            FROM agent_resources
            WHERE id = $1
              AND agent_id = $2
            FOR UPDATE
            ",
            actor.attempt.0,
            actor.agent.0,
        )
        .fetch_one(&mut *transaction)
        .await?;
        Ok(transaction)
    }

    pub(super) async fn recheck(
        &self,
        token: &str,
        actor: &RuntimeActor,
        transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<(), Error> {
        sqlx::query_scalar!(
            r#"
            SELECT attempt_id AS "attempt_id!"
            FROM runtime_session_access
            WHERE attempt_id = $1
              AND version = $2
              AND token_hash = $3
              AND recovery
            "#,
            actor.attempt.0,
            actor.version,
            super::sessions::digest(token),
        )
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(Error::Unauthorized)?;
        Ok(())
    }
}

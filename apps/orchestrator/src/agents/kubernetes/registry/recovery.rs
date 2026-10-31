use super::{AttemptId, Error, Phase, Registry, ResourceKind, ResourceObject, Row, objects};
use crate::agents::AgentId;
use crate::agents::kubernetes::termination_proven;
use k8s_openapi::api::core::v1::Pod;

impl Registry {
    /// Only a plan that has never entered the dispatch boundary can be cancelled.
    #[tracing::instrument(skip_all, fields(attempt = %id.0))]
    pub async fn cancel_planned(&self, id: super::AttemptId) -> Result<(), Error> {
        let changed = sqlx::query!(
            r#"
            UPDATE agent_resources
            SET phase = 'stopped',
                termination_proof = '{"kind":"not_dispatched"}'::jsonb,
                updated_at = clock_timestamp()
            WHERE id = $1
              AND kind = 'pod'
              AND phase = 'planned'
            "#,
            id.0,
        )
        .execute(&self.pool)
        .await?
        .rows_affected();
        if changed != 1 {
            return Err(Error::Conflict);
        }
        Ok(())
    }

    /// Absence is not termination evidence and never releases a reservation.
    #[tracing::instrument(skip_all, fields(attempt = %id.0))]
    pub async fn record_missing(&self, id: super::AttemptId) -> Result<(), Error> {
        let changed = sqlx::query!(
            "
            UPDATE agent_resources
            SET blocked_reason = 'object_missing',
                updated_at = clock_timestamp()
            WHERE id = $1
              AND phase IN ('creating', 'bound')
            ",
            id.0,
        )
        .execute(&self.pool)
        .await?
        .rows_affected();
        if changed != 1 {
            return Err(Error::Conflict);
        }
        Ok(())
    }

    /// Retain the observed terminal object before allowing the next pod epoch.
    pub async fn record_stopped(&self, id: super::AttemptId, pod: &Pod) -> Result<(), Error> {
        self.store_termination(id, pod, true).await
    }

    pub(crate) async fn retain_termination(
        &self,
        id: super::AttemptId,
        pod: &Pod,
    ) -> Result<(), Error> {
        self.store_termination(id, pod, false).await
    }

    #[tracing::instrument(skip_all, fields(attempt = %id.0, stopped = stopped))]
    async fn store_termination(
        &self,
        id: super::AttemptId,
        pod: &Pod,
        stopped: bool,
    ) -> Result<(), Error> {
        let mut transaction = self.transaction().await?;
        super::lock_attempt_agent(&mut transaction, id).await?;
        let row = sqlx::query_as!(
            Row,
            r#"
            SELECT id AS "id: AttemptId", agent_id AS "agent_id: AgentId", kind AS "kind: ResourceKind",
                   epoch, target_generation, definition_revision, phase AS "phase: Phase", uid,
                   blocked_reason,
                   manifest AS "manifest: sqlx::types::Json<serde_json::Value>",
                   source_manifest AS "source_manifest: sqlx::types::Json<serde_json::Value>",
                   prepared
            FROM agent_resources
            WHERE id = $1
            FOR UPDATE
            "#,
            id.0,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(Error::Conflict)?;
        let expected = row.attempt()?;
        if expected.kind != ResourceKind::Pod
            || !matches!(expected.phase, Phase::Bound | Phase::Stopped)
            || expected.uid.is_none()
        {
            return Err(Error::Conflict);
        }
        objects::verify(&expected, &ResourceObject::Pod(Box::new(pod.clone())))?;
        if !termination_proven(pod) {
            return Err(Error::TerminationUnproven);
        }
        sqlx::query!(
            "
            UPDATE agent_resources
            SET phase = CASE WHEN $3 THEN 'stopped' ELSE phase END,
                blocked_reason = NULL,
                termination_proof = COALESCE(termination_proof, $2),
                updated_at = clock_timestamp()
            WHERE id = $1
            ",
            id.0,
            sqlx::types::Json(pod) as _,
            stopped,
        )
        .execute(&mut *transaction)
        .await?;
        super::latch_drain(&mut transaction, id).await?;
        transaction.commit().await?;
        Ok(())
    }
    #[tracing::instrument(skip_all, fields(attempt = %id.0))]
    pub(crate) async fn retained_termination(
        &self,
        id: super::AttemptId,
    ) -> Result<Option<Pod>, Error> {
        let proof = sqlx::query_scalar!(
            r#"
            SELECT termination_proof AS "termination_proof: sqlx::types::Json<Pod>"
            FROM agent_resources
            WHERE id = $1
              AND kind = 'pod'
              AND phase = 'bound'
            "#,
            id.0,
        )
        .fetch_optional(&self.pool)
        .await?
        .flatten();
        Ok(proof.map(|proof| proof.0))
    }

    #[tracing::instrument(skip_all, fields(attempt = %attempt.id.0))]
    pub(crate) async fn recorded_pod(
        &self,
        attempt: &super::Attempt,
    ) -> Result<crate::agents::kubernetes::RecordedPod, Error> {
        let mut transaction = self.transaction().await?;
        let binding = super::anchor::load(&mut transaction).await?;
        transaction.commit().await?;
        Ok(crate::agents::kubernetes::RecordedPod {
            namespace: binding.namespace,
            name: attempt
                .object
                .metadata()
                .name
                .clone()
                .ok_or(Error::OwnershipMismatch)?,
            uid: attempt.uid.clone().ok_or(Error::OwnershipMismatch)?,
            identity: crate::agents::kubernetes::Identity {
                installation: binding.installation,
                agent: attempt.agent,
                attempt: attempt.id.0,
                anchor_name: binding.name,
                anchor_uid: binding.uid,
            },
        })
    }
}

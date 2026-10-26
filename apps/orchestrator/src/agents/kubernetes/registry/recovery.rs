use super::{Error, Phase, Registry, ResourceKind, ResourceObject, Row, objects};
use crate::agents::kubernetes::termination_proven;
use k8s_openapi::api::core::v1::Pod;

impl Registry {
    /// Only a plan that has never entered the dispatch boundary can be cancelled.
    pub async fn cancel_planned(&self, id: super::AttemptId) -> Result<(), Error> {
        let changed = sqlx::query("UPDATE agent_resources SET phase = 'stopped', termination_proof = '{\"kind\":\"not_dispatched\"}'::jsonb, updated_at = clock_timestamp() WHERE id = $1 AND kind = 'pod' AND phase = 'planned'")
            .bind(id).execute(&self.pool).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Conflict);
        }
        Ok(())
    }

    /// Absence is not termination evidence and never releases a reservation.
    pub async fn record_missing(&self, id: super::AttemptId) -> Result<(), Error> {
        let changed = sqlx::query("UPDATE agent_resources SET blocked_reason = 'object_missing', updated_at = clock_timestamp() WHERE id = $1 AND phase IN ('creating','bound')")
            .bind(id).execute(&self.pool).await?.rows_affected();
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

    async fn store_termination(
        &self,
        id: super::AttemptId,
        pod: &Pod,
        stopped: bool,
    ) -> Result<(), Error> {
        let mut transaction = self.transaction().await?;
        super::lock_attempt_agent(&mut transaction, id).await?;
        let row: Row = sqlx::query_as("SELECT id,agent_id,kind,epoch,target_generation,definition_revision,phase,uid,blocked_reason,manifest,source_manifest,prepared FROM agent_resources WHERE id = $1 FOR UPDATE")
            .bind(id).fetch_optional(&mut *transaction).await?.ok_or(Error::Conflict)?;
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
        sqlx::query("UPDATE agent_resources SET phase = CASE WHEN $3 THEN 'stopped' ELSE phase END, blocked_reason = NULL, termination_proof = COALESCE(termination_proof,$2), updated_at = clock_timestamp() WHERE id = $1")
            .bind(id).bind(sqlx::types::Json(pod)).bind(stopped).execute(&mut *transaction).await?;
        super::latch_drain(&mut transaction, id).await?;
        transaction.commit().await?;
        Ok(())
    }
    pub(crate) async fn retained_termination(
        &self,
        id: super::AttemptId,
    ) -> Result<Option<Pod>, Error> {
        let proof: Option<sqlx::types::Json<Pod>> = sqlx::query_scalar("SELECT termination_proof FROM agent_resources WHERE id = $1 AND kind = 'pod' AND phase = 'bound'")
            .bind(id).fetch_optional(&self.pool).await?.flatten();
        Ok(proof.map(|proof| proof.0))
    }

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

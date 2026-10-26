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
        let mut transaction = self.transaction().await?;
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
        sqlx::query("UPDATE agent_resources SET phase = 'stopped', blocked_reason = NULL, termination_proof = $2, updated_at = clock_timestamp() WHERE id = $1")
            .bind(id).bind(sqlx::types::Json(pod)).execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(())
    }
}

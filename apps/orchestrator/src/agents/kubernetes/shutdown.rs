//! Retain terminal evidence before releasing a finalizer. A 404 alone never releases ownership.
use super::{
    Attempt, Error, Phase, ReconcileResult, Reconciler, ResourceObject, termination_proven,
    verify_pod,
};
use k8s_openapi::api::core::v1::Pod;

impl Reconciler {
    pub(super) async fn settle_pod(
        &self,
        attempt: Attempt,
        proof: Option<Pod>,
        observed: Option<ResourceObject>,
    ) -> Result<Option<ReconcileResult>, Error> {
        let Some(ResourceObject::Pod(pod)) = observed else {
            return self.absent(&attempt, proof).await;
        };
        if let Err(error) = self.verify_shutdown(&attempt, &pod, proof.as_ref()).await {
            self.registry
                .block(
                    attempt.id,
                    attempt.phase,
                    "ownership_or_specification_conflict",
                )
                .await?;
            return Err(error);
        }
        if termination_proven(&pod) {
            return self.finish(&attempt, &pod, proof).await;
        }
        self.wait_or_stop(&attempt, &pod).await.map(Some)
    }

    async fn verify_shutdown(
        &self,
        attempt: &Attempt,
        pod: &Pod,
        proof: Option<&Pod>,
    ) -> Result<(), Error> {
        if let Some(proof) = proof {
            let recorded = self.registry.recorded_pod(attempt).await?;
            verify_pod(pod, &recorded)?;
            if pod.spec != proof.spec || !termination_proven(pod) {
                return Err(Error::OwnershipMismatch);
            }
        }
        Ok(())
    }

    async fn absent(
        &self,
        attempt: &Attempt,
        proof: Option<Pod>,
    ) -> Result<Option<ReconcileResult>, Error> {
        if let Some(proof) = proof {
            self.registry.record_stopped(attempt.id, &proof).await?;
            return Ok(None);
        }
        self.registry.record_missing(attempt.id).await?;
        Ok(Some(ReconcileResult::Uncertain {
            attempt: attempt.id,
        }))
    }

    async fn finish(
        &self,
        attempt: &Attempt,
        pod: &Pod,
        proof: Option<Pod>,
    ) -> Result<Option<ReconcileResult>, Error> {
        // Keep the row Bound until finalization succeeds, retaining proof across lost replies.
        let retained = proof.as_ref().unwrap_or(pod);
        self.registry
            .retain_termination(attempt.id, retained)
            .await?;
        if pod.metadata.deletion_timestamp.is_none() {
            self.api.stop_observed(pod).await?;
            return Ok(Some(ReconcileResult::Stopping {
                attempt: attempt.id,
            }));
        }
        self.api.finalize_observed(pod).await?;
        self.registry.record_stopped(attempt.id, retained).await?;
        Ok(None)
    }

    async fn wait_or_stop(&self, attempt: &Attempt, pod: &Pod) -> Result<ReconcileResult, Error> {
        let acknowledged = sqlx::query_scalar!(
            r#"
            SELECT EXISTS (
                SELECT 1
                FROM agent_runtime_drains
                WHERE attempt_id = $1
                  AND acknowledged_at IS NOT NULL
            ) AS "acknowledged!"
            "#,
            attempt.id.0,
        )
        .fetch_one(&self.pool)
        .await?;
        self.registry
            .block(attempt.id, Phase::Bound, "drain_required")
            .await?;
        if !acknowledged {
            return Ok(ReconcileResult::AwaitingDrain {
                attempt: attempt.id,
            });
        }
        if pod.metadata.deletion_timestamp.is_none() {
            self.api.stop_observed(pod).await?;
        }
        Ok(ReconcileResult::Stopping {
            attempt: attempt.id,
        })
    }
}

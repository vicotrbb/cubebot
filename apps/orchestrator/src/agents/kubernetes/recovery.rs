//! Inspect a physical attempt once before provisioning or recovery decisions.
use super::{Attempt, AttemptId, Error, Phase, ReconcileResult, Reconciler, ResourceKind};
use crate::agents::AgentId;

pub(super) enum Inspection {
    Vacant,
    Healthy(AttemptId),
    Held(ReconcileResult),
}
impl Reconciler {
    pub(super) async fn inspect_pod(
        &self,
        agent: AgentId,
        generation: i64,
        running: bool,
    ) -> Result<Inspection, Error> {
        let Some(attempt) = self.registry.current(agent, ResourceKind::Pod).await? else {
            return Ok(Inspection::Vacant);
        };
        let current = running && attempt.target_generation == generation;
        if attempt.phase == Phase::Planned {
            if !current {
                self.registry.cancel_planned(attempt.id).await?;
            }
            return Ok(Inspection::Vacant);
        }
        self.inspect_dispatched(attempt, current).await
    }
    async fn inspect_dispatched(
        &self,
        mut attempt: Attempt,
        current: bool,
    ) -> Result<Inspection, Error> {
        let proof = self.registry.retained_termination(attempt.id).await?;
        let observed = self.api.fetch_candidate(&attempt.object).await?;
        if proof.is_none()
            && let Some(object) = &observed
        {
            attempt = self.bind(attempt, object.clone()).await?;
        }
        let draining: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM agent_runtime_drains WHERE attempt_id = $1)",
        )
        .bind(attempt.id)
        .fetch_one(&self.pool)
        .await?;
        if !current || proof.is_some() || draining || observed.is_none() {
            return Ok(match self.settle_pod(attempt, proof, observed).await? {
                Some(outcome) => Inspection::Held(outcome),
                None => Inspection::Vacant,
            });
        }
        Ok(Inspection::Healthy(attempt.id))
    }
}

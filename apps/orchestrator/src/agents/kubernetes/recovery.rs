//! Inspect a physical attempt once before provisioning or recovery decisions.
use super::{
    Attempt, AttemptId, Error, Phase, ReconcileResult, Reconciler, ResourceKind, ResourceSpec,
};
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
        let wanted = running && attempt.target_generation == generation;
        let drifted = wanted && self.drifted(agent, &attempt).await?;
        let current = wanted && !drifted;
        if attempt.phase == Phase::Planned {
            if !current {
                self.registry.cancel_planned(attempt.id).await?;
            }
            return Ok(Inspection::Vacant);
        }
        if drifted {
            self.registry.request_drain(attempt.id).await?;
        }
        self.inspect_dispatched(attempt, current).await
    }
    /// The attempt was made from another pod spec than the configured one, as after a Helm image
    /// change; the next epoch takes the new spec, and the workspace carries over.
    async fn drifted(&self, agent: AgentId, attempt: &Attempt) -> Result<bool, Error> {
        let Some(workspace) = self
            .registry
            .current(agent, ResourceKind::Workspace)
            .await?
        else {
            return Ok(false);
        };
        let name = workspace
            .object
            .metadata()
            .name
            .as_deref()
            .ok_or(Error::InvalidConfiguration)?;
        let configured = ResourceSpec::Pod(Box::new(self.layout.pod(name)?));
        Ok(!configured.matches(&attempt.intent))
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
        let draining = sqlx::query_scalar!(
            r#"
            SELECT EXISTS (
                SELECT 1
                FROM agent_runtime_drains
                WHERE attempt_id = $1
            ) AS "draining!"
            "#,
            attempt.id.0,
        )
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

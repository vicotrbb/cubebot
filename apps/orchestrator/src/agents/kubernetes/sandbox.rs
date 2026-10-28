//! Kubernetes sandboxes: one `cubebot-sandbox` pod and one retained workspace PVC per agent.
use super::{Error, Phase, ReconcileResult, Reconciler, ResourceKind, observation::Observation};
use crate::agents::{
    AgentId,
    sandbox::{ExecChannel, Sandbox, Status},
};

impl Sandbox for Reconciler {
    async fn create(&self, agent: AgentId) -> Result<(), Error> {
        let (revision, _, _) = self.intent(agent).await?;
        match self.prepare_workspace(agent, revision).await? {
            Ok(_) => Ok(()),
            Err(_) => Err(Error::Conflict),
        }
    }
    /// Requires `create`; settles any stale pod before starting a new epoch.
    async fn start(&self, agent: AgentId) -> Result<(), Error> {
        let (revision, generation, _) = self.intent(agent).await?;
        let verified = match self.settle(agent, generation, true).await? {
            Ok(verified) => verified,
            Err(outcome) => return settled(outcome),
        };
        let workspace = self
            .registry
            .current(agent, ResourceKind::Workspace)
            .await?
            .filter(|workspace| workspace.phase == Phase::Bound)
            .ok_or(Error::Conflict)?;
        settled(
            self.provision(agent, revision, &workspace, verified)
                .await?,
        )
    }
    async fn stop(&self, agent: AgentId) -> Result<(), Error> {
        let (_, generation, _) = self.intent(agent).await?;
        match self.settle(agent, generation, false).await? {
            Ok(_) => Ok(()),
            Err(outcome) => settled(outcome),
        }
    }
    async fn exec_channel(&self, agent: AgentId, command: &[&str]) -> Result<ExecChannel, Error> {
        let name = self
            .current_name(agent, ResourceKind::Pod)
            .await?
            .ok_or(Error::Conflict)?;
        self.api.exec(&name, command).await
    }
    async fn workspace(&self, agent: AgentId) -> Result<Option<String>, Error> {
        self.current_name(agent, ResourceKind::Workspace).await
    }
    async fn status(&self, agent: AgentId) -> Result<Status, Error> {
        let Some(attempt) = self.registry.current(agent, ResourceKind::Pod).await? else {
            return Ok(Status::Absent);
        };
        let Some(observed) = self.api.fetch_candidate(&attempt.object).await? else {
            return Ok(Status::Absent);
        };
        let observed = Observation::of(&observed);
        Ok(match observed {
            Observation { deleting: true, .. } => Status::Stopping,
            Observation { recovery: true, .. } => Status::Stopped,
            Observation { ready: true, .. } => Status::Running,
            _ => Status::Starting,
        })
    }
}

/// Draining and stopping are progress; an object of unknown ownership must be reconciled first.
fn settled(result: ReconcileResult) -> Result<(), Error> {
    match result {
        ReconcileResult::Uncertain { .. } => Err(Error::Conflict),
        _ => Ok(()),
    }
}

impl Reconciler {
    async fn current_name(
        &self,
        agent: AgentId,
        kind: ResourceKind,
    ) -> Result<Option<String>, Error> {
        Ok(self
            .registry
            .current(agent, kind)
            .await?
            .and_then(|attempt| attempt.object.metadata().name.clone()))
    }
}

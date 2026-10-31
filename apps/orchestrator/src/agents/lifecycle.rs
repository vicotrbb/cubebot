use super::{AgentId, Agents, commands::Command};
use crate::identity::{Actor, IdentityError, Receipt};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleAction {
    Wake,
    Sleep,
    Restart,
    Retire,
}

impl LifecycleAction {
    fn desired(self) -> &'static str {
        match self {
            Self::Wake | Self::Restart => "running",
            Self::Sleep => "sleeping",
            Self::Retire => "retired",
        }
    }
    fn restart(self) -> bool {
        matches!(self, Self::Restart)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleCommand {
    pub request_id: uuid::Uuid,
    pub expected_revision: i64,
    pub action: LifecycleAction,
}

impl Agents {
    #[tracing::instrument(skip_all, fields(agent = %id.0))]
    pub async fn lifecycle(
        &self,
        actor: &Actor,
        id: AgentId,
        input: &LifecycleCommand,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(actor, Some(id), "agent.lifecycle", input.request_id, input)?;
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        let desired = input.action.desired();
        let revision = sqlx::query_scalar!(
            "
            UPDATE agents
            SET desired_state = $2,
                restart_generation = restart_generation
                    + CASE WHEN $3 OR (desired_state = 'sleeping' AND $2 = 'running') THEN 1 ELSE 0 END,
                revision = revision + 1
            WHERE id = $1
              AND revision = $4
              AND desired_state <> 'retired'
            RETURNING revision
            ",
            id.0,
            desired,
            input.action.restart(),
            input.expected_revision,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IdentityError::Conflict)?;
        sqlx::query!(
            "
            INSERT INTO agent_runtime_drains (attempt_id)
            SELECT r.id
            FROM agent_resources r
            JOIN agents a ON a.id = r.agent_id
            WHERE a.id = $1
              AND r.kind = 'pod'
              AND r.phase IN ('creating', 'bound')
              AND (a.desired_state <> 'running' OR a.restart_generation <> r.target_generation)
            ON CONFLICT (attempt_id) DO NOTHING
            ",
            id.0,
        )
        .execute(&mut *transaction)
        .await?;
        command
            .commit(transaction, Receipt { id: id.0, revision })
            .await
    }
}

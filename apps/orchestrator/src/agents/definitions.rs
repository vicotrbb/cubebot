use super::{
    Agents,
    commands::{Command, snapshot},
};
use crate::identity::{Actor, IdentityError, Receipt};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(transparent)]
#[sqlx(transparent)]
pub struct AgentId(pub Uuid);

#[derive(Debug, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Agent {
    pub id: AgentId,
    pub name: String,
    pub instructions: String,
    pub revision: i64,
    pub instructions_revision: i64,
    pub desired_state: String,
    pub idle_policy: super::IdlePolicy,
    pub restart_generation: i64,
    /// Seconds an idle agent waits before it dreams.
    pub idle_delay_seconds: i32,
    /// The dreaming model on the agent's connection; `None` inherits the connection's model.
    pub dreaming_model: Option<String>,
    /// `working`, `dreaming` or `idle` while running, otherwise the desired state such as
    /// `sleeping`. Dreaming needs a run whose sandbox still holds its credential.
    pub activity: String,
}

const COLUMNS: &str = "g.id, g.name, g.instructions, g.revision, g.instructions_revision, g.desired_state, g.idle_policy, g.restart_generation, g.idle_delay_seconds, g.dreaming_model, CASE WHEN g.desired_state <> 'running' THEN g.desired_state WHEN EXISTS(SELECT 1 FROM chat_turns t WHERE t.conversation_id = g.id AND t.status = 'running') THEN 'working' WHEN EXISTS(SELECT 1 FROM dream_runs d JOIN runtime_session_access s ON s.attempt_id = d.attempt_id AND s.active WHERE d.agent_id = g.id AND d.status = 'running') THEN 'dreaming' ELSE 'idle' END AS activity";

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAgent {
    pub request_id: Uuid,
    pub name: String,
    pub instructions: String,
}

impl Agents {
    pub async fn create(
        &self,
        actor: &Actor,
        input: &CreateAgent,
    ) -> Result<Receipt, IdentityError> {
        validate(input)?;
        let command = Command::new(actor, None, "agent.create", input.request_id, input)?;
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        let id = sqlx::query_scalar(
            "INSERT INTO agents (name, instructions, created_by) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(&input.name)
        .bind(&input.instructions)
        .bind(actor.id)
        .fetch_one(&mut *transaction)
        .await?;
        snapshot(&mut transaction, actor.id, AgentId(id)).await?;
        command
            .commit(transaction, Receipt { id, revision: 1 })
            .await
    }

    pub async fn list(
        &self,
        actor: &Actor,
        after: Option<AgentId>,
    ) -> Result<Vec<Agent>, IdentityError> {
        Ok(sqlx::query_as(&format!("SELECT {COLUMNS} FROM agents g JOIN human_accounts a ON a.id = $1 AND a.active WHERE ($2::uuid IS NULL OR g.id > $2) AND (a.role = 'administrator' OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id = a.id AND p.scope_kind = 'agent' AND p.scope_id = g.id AND p.capability = 'agent.manage')) ORDER BY g.id LIMIT 100"))
            .bind(actor.id).bind(after).fetch_all(&self.pool).await?)
    }

    pub async fn get(&self, actor: &Actor, id: AgentId) -> Result<Agent, IdentityError> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM agents g JOIN human_accounts a ON a.id = $1 AND a.active WHERE g.id = $2 AND (a.role = 'administrator' OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id = a.id AND p.scope_kind = 'agent' AND p.scope_id = g.id AND p.capability = 'agent.manage'))"))
            .bind(actor.id).bind(id).fetch_optional(&self.pool).await?.ok_or(IdentityError::NotFound)
    }
}

pub(super) fn validate(input: &CreateAgent) -> Result<(), IdentityError> {
    if input.request_id.is_nil()
        || input.name.trim().is_empty()
        || input.name.len() > 255
        || input.instructions.len() > 65536
        || input.instructions.contains('\0')
        || input.name.chars().any(char::is_control)
    {
        return Err(IdentityError::InvalidInput);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instructions_reject_null_bytes_before_postgresql_encoding() {
        let input = CreateAgent {
            request_id: Uuid::new_v4(),
            name: "Researcher".into(),
            instructions: "Evidence\0with null".into(),
        };
        assert!(matches!(validate(&input), Err(IdentityError::InvalidInput)));
    }
}

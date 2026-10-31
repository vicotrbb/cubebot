use super::{
    Agents, IdlePolicy,
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
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
    /// Delegated tasks that may run beside the main lane (ADR 0005), one to four.
    pub task_lanes: i16,
    /// `working`, `dreaming` or `idle` while running, otherwise the desired state such as
    /// `sleeping`. Dreaming needs a run whose sandbox still holds its credential.
    pub activity: String,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAgent {
    pub request_id: Uuid,
    pub name: String,
    pub instructions: String,
}

impl Agents {
    #[tracing::instrument(skip_all)]
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
        let id = sqlx::query_scalar!(
            "
            INSERT INTO agents (name, instructions, created_by)
            VALUES ($1, $2, $3)
            RETURNING id
            ",
            input.name,
            input.instructions,
            actor.id,
        )
        .fetch_one(&mut *transaction)
        .await?;
        snapshot(&mut transaction, actor.id, AgentId(id)).await?;
        command
            .commit(transaction, Receipt { id, revision: 1 })
            .await
    }

    #[tracing::instrument(skip_all)]
    pub async fn list(
        &self,
        actor: &Actor,
        after: Option<AgentId>,
    ) -> Result<Vec<Agent>, IdentityError> {
        Ok(sqlx::query_as!(
            Agent,
            r#"
        SELECT g.id AS "id: AgentId", g.name, g.instructions, g.revision,
               g.instructions_revision, g.desired_state, g.idle_policy AS "idle_policy: IdlePolicy",
               g.restart_generation, g.idle_delay_seconds, g.dreaming_model, g.task_lanes,
               CASE
                   WHEN g.desired_state <> 'running' THEN g.desired_state
                   WHEN EXISTS(
                       SELECT 1
                       FROM chat_turns t
                       WHERE t.conversation_id = g.id
                         AND t.status = 'running'
                   ) THEN 'working'
                   WHEN EXISTS(
                       SELECT 1
                       FROM dream_runs d
                       JOIN runtime_session_access s ON s.attempt_id = d.attempt_id AND s.active
                       WHERE d.agent_id = g.id
                         AND d.status = 'running'
                   ) THEN 'dreaming'
                   ELSE 'idle'
               END AS "activity!"
        FROM agents g
        JOIN human_accounts a ON a.id = $1 AND a.active
        WHERE ($2::uuid IS NULL OR g.id > $2)
          AND (
            a.role = 'administrator'
            OR EXISTS(
                SELECT 1
                FROM effective_human_grants p
                WHERE p.actor_id = a.id
                  AND p.scope_kind = 'agent'
                  AND p.scope_id = g.id
                  AND p.capability = 'agent.manage'
            )
          )
        ORDER BY g.id
        LIMIT 100
        "#,
            actor.id,
            after.map(|after| after.0),
        )
        .fetch_all(&self.pool)
        .await?)
    }

    #[tracing::instrument(skip_all, fields(agent = %id.0))]
    pub async fn get(&self, actor: &Actor, id: AgentId) -> Result<Agent, IdentityError> {
        sqlx::query_as!(
            Agent,
            r#"
        SELECT g.id AS "id: AgentId", g.name, g.instructions, g.revision,
               g.instructions_revision, g.desired_state, g.idle_policy AS "idle_policy: IdlePolicy",
               g.restart_generation, g.idle_delay_seconds, g.dreaming_model, g.task_lanes,
               CASE
                   WHEN g.desired_state <> 'running' THEN g.desired_state
                   WHEN EXISTS(
                       SELECT 1
                       FROM chat_turns t
                       WHERE t.conversation_id = g.id
                         AND t.status = 'running'
                   ) THEN 'working'
                   WHEN EXISTS(
                       SELECT 1
                       FROM dream_runs d
                       JOIN runtime_session_access s ON s.attempt_id = d.attempt_id AND s.active
                       WHERE d.agent_id = g.id
                         AND d.status = 'running'
                   ) THEN 'dreaming'
                   ELSE 'idle'
               END AS "activity!"
        FROM agents g
        JOIN human_accounts a ON a.id = $1 AND a.active
        WHERE g.id = $2
          AND (
            a.role = 'administrator'
            OR EXISTS(
                SELECT 1
                FROM effective_human_grants p
                WHERE p.actor_id = a.id
                  AND p.scope_kind = 'agent'
                  AND p.scope_id = g.id
                  AND p.capability = 'agent.manage'
            )
          )
        "#,
            actor.id,
            id.0,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::NotFound)
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

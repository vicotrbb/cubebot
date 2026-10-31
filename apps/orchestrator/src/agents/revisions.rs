use super::{
    AgentId, Agents, CreateAgent,
    commands::{Command, snapshot},
};
use crate::identity::{Actor, IdentityError, Receipt};
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum IdlePolicy {
    Sleep,
    KeepRunning,
}

impl IdlePolicy {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Sleep => "sleep",
            Self::KeepRunning => "keep_running",
        }
    }
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAgent {
    pub request_id: uuid::Uuid,
    pub expected_revision: i64,
    pub name: String,
    pub instructions: String,
    pub idle_policy: IdlePolicy,
    pub idle_delay_seconds: i32,
    /// `None` dreams with the connection's model.
    #[serde(default)]
    pub dreaming_model: Option<String>,
    /// Concurrent delegated tasks, one to four; `None` keeps the current setting.
    #[serde(default)]
    pub task_lanes: Option<i16>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct DefinitionRevision {
    pub revision: i64,
    pub instructions_revision: i64,
    pub name: String,
    pub instructions: String,
    pub idle_policy: IdlePolicy,
    pub idle_delay_seconds: i32,
    pub dreaming_model: Option<String>,
    pub task_lanes: i16,
    pub actor_id: uuid::Uuid,
}

impl Agents {
    pub async fn update(
        &self,
        actor: &Actor,
        id: AgentId,
        input: &UpdateAgent,
    ) -> Result<Receipt, IdentityError> {
        super::definitions::validate(&CreateAgent {
            request_id: input.request_id,
            name: input.name.clone(),
            instructions: input.instructions.clone(),
        })?;
        if !(60..=86400).contains(&input.idle_delay_seconds)
            || input
                .task_lanes
                .is_some_and(|lanes| !(1..=4).contains(&lanes))
            || input.dreaming_model.as_deref().is_some_and(|model| {
                model.trim().is_empty() || model.len() > 200 || model.chars().any(char::is_control)
            })
        {
            return Err(IdentityError::InvalidInput);
        }
        let command = Command::new(actor, Some(id), "agent.update", input.request_id, input)?;
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        let revision = sqlx::query_scalar("UPDATE agents SET name = $2, instructions_revision = instructions_revision + CASE WHEN instructions <> $3 THEN 1 ELSE 0 END, instructions = $3, idle_policy = $4, idle_delay_seconds = $6, dreaming_model = $7, task_lanes = COALESCE($8, task_lanes), revision = revision + 1 WHERE id = $1 AND revision = $5 AND desired_state <> 'retired' RETURNING revision")
            .bind(id).bind(&input.name).bind(&input.instructions).bind(input.idle_policy.as_str()).bind(input.expected_revision).bind(input.idle_delay_seconds).bind(&input.dreaming_model).bind(input.task_lanes).fetch_optional(&mut *transaction).await?.ok_or(IdentityError::Conflict)?;
        snapshot(&mut transaction, actor.id, id).await?;
        command
            .commit(transaction, Receipt { id: id.0, revision })
            .await
    }

    pub async fn revisions(
        &self,
        actor: &Actor,
        id: AgentId,
        after: i64,
    ) -> Result<Vec<DefinitionRevision>, IdentityError> {
        if after < 0 {
            return Err(IdentityError::InvalidInput);
        }
        // Recheck access within the history query too, so concurrent revocation cannot expose old text.
        self.get(actor, id).await?;
        Ok(sqlx::query_as("SELECT r.revision, r.instructions_revision, r.name, r.instructions, r.idle_policy, r.idle_delay_seconds, r.dreaming_model, r.task_lanes, r.actor_id FROM agent_definition_revisions r JOIN human_accounts a ON a.id = $1 AND a.active WHERE r.agent_id = $2 AND r.revision > $3 AND (a.role = 'administrator' OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id = a.id AND p.scope_kind = 'agent' AND p.scope_id = r.agent_id AND p.capability = 'agent.manage')) ORDER BY r.revision LIMIT 100")
            .bind(actor.id).bind(id).bind(after).fetch_all(&self.pool).await?)
    }
}

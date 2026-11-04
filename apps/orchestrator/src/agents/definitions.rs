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

/// The agent's cube (docs/design/console.md); mood is derived from activity, never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Avatar {
    pub paint: Paint,
    pub eyes: Eyes,
    pub accessory: Accessory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Paint {
    Orange,
    Sage,
    Paper,
    Wood,
    Sky,
    Rose,
    Lemon,
    Plum,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Eyes {
    Round,
    Wide,
    Sleepy,
    Dot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Accessory {
    None,
    Cap,
    Glasses,
    Bow,
    Antenna,
}

/// What the caller may know about an agent: everyone holding any capability on it sees it,
/// with their own capabilities; administrators see every agent with every capability.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Agent {
    pub id: AgentId,
    pub name: String,
    /// `None` without `agent_manage`.
    pub instructions: Option<String>,
    pub revision: i64,
    pub instructions_revision: i64,
    pub desired_state: String,
    pub idle_policy: super::IdlePolicy,
    pub restart_generation: i64,
    /// Seconds an idle agent waits before it dreams.
    pub idle_delay_seconds: i32,
    /// The dreaming model on the agent's connection; `None` inherits the connection's model.
    pub dreaming_model: Option<String>,
    /// Delegated tasks that may run beside the main lane (docs/adr/agents.md#agent-execution-lanes), one to four.
    pub task_lanes: i16,
    /// The agent may broadcast to every agent (migrations/0046_broadcasts.sql).
    pub broadcast: bool,
    /// The agent may use the knowledge base (migrations/0048_knowledge_base.sql), on by default.
    pub knowledge_base: bool,
    /// `working`, `dreaming`, `sleeping` (idle past its delay) or `idle` while running,
    /// otherwise the desired state, `stopped` or `retired`. Dreaming needs a run whose sandbox
    /// still holds its credential.
    pub activity: String,
    /// The caller's capabilities on this agent and its conversation as grant names, such as
    /// `conversation_send`.
    pub capabilities: Vec<String>,
    #[schema(value_type = Avatar)]
    pub avatar: sqlx::types::Json<Avatar>,
    /// A running turn called a desktop tool in the last 120 seconds.
    pub desktop_active: bool,
    /// The agent is waiting for a person to log in on a site (docs/adr/sandbox.md#live-browser-viewing-and-explicit-human-takeover).
    pub pending_login: bool,
    /// The tool its running turn is in now; `None` while it thinks, works on nothing, or the
    /// caller cannot read its conversation.
    pub current_tool: Option<String>,
    /// What that tool is working on, such as a file or a URL.
    pub current_target: Option<String>,
    /// The highest turn sequence in the agent's conversation, 0 without turns.
    pub last_sequence: i64,
    /// The caller's read cursor in that conversation, 0 until set.
    pub read_sequence: i64,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAgent {
    pub request_id: Uuid,
    pub name: String,
    pub instructions: String,
    /// Left out, the agent gets the default cube. Absent from the command digest when `None`,
    /// so receipts recorded before avatars still replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<Avatar>,
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
            INSERT INTO agents (name, instructions, created_by, avatar)
            VALUES ($1, $2, $3, COALESCE($4, '{\"paint\":\"orange\",\"eyes\":\"round\",\"accessory\":\"none\"}'::jsonb))
            RETURNING id
            ",
            input.name,
            input.instructions,
            actor.id,
            input.avatar.map(sqlx::types::Json) as Option<sqlx::types::Json<Avatar>>,
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
        self.summaries(actor, after, None, 100).await
    }

    /// Every agent the caller may see, for the installation stream.
    #[tracing::instrument(skip_all)]
    pub async fn visible(&self, actor: &Actor) -> Result<Vec<Agent>, IdentityError> {
        // shortcut: one page of 1000; page by id if an installation grows past it.
        self.summaries(actor, None, None, 1000).await
    }

    #[tracing::instrument(skip_all, fields(agent = %id.0))]
    pub async fn get(&self, actor: &Actor, id: AgentId) -> Result<Agent, IdentityError> {
        self.summaries(actor, None, Some(id), 1)
            .await?
            .pop()
            .ok_or(IdentityError::NotFound)
    }

    /// One query: visibility, the caller's capabilities and the live flags per agent.
    async fn summaries(
        &self,
        actor: &Actor,
        after: Option<AgentId>,
        only: Option<AgentId>,
        limit: i64,
    ) -> Result<Vec<Agent>, IdentityError> {
        Ok(sqlx::query_as!(
            Agent,
            r#"
        SELECT g.id AS "id: AgentId", g.name,
               CASE WHEN 'agent_manage' = ANY(c.names) THEN g.instructions END AS instructions,
               g.revision, g.instructions_revision, g.desired_state,
               g.idle_policy AS "idle_policy: IdlePolicy", g.restart_generation,
               g.idle_delay_seconds, g.dreaming_model, g.task_lanes, g.broadcast, g.knowledge_base,
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
                   -- Idle past its delay with nothing pending: the agent sleeps, and dreams.
                   WHEN NOT EXISTS(
                       SELECT 1
                       FROM chat_turns t
                       WHERE t.conversation_id = g.id
                         AND t.status IN ('queued', 'waiting', 'delegated')
                   ) AND GREATEST(
                       (SELECT max(COALESCE(t.completed_at, t.created_at)) FROM chat_turns t WHERE t.conversation_id = g.id),
                       (SELECT max(r.updated_at) FROM agent_resources r WHERE r.agent_id = g.id AND r.kind = 'pod'),
                       g.created_at
                   ) < clock_timestamp() - make_interval(secs => g.idle_delay_seconds) THEN 'sleeping'
                   ELSE 'idle'
               END AS "activity!",
               c.names AS "capabilities!",
               g.avatar AS "avatar: sqlx::types::Json<Avatar>",
               EXISTS(
                   SELECT 1
                   FROM chat_turns t
                   JOIN turn_events e ON e.turn_id = t.id
                   WHERE t.conversation_id = g.id
                     AND t.status = 'running'
                     AND e.created_at > clock_timestamp() - interval '120 seconds'
                     AND e.payload->>'kind' = 'tool_call'
                     AND (e.payload->>'name' IN ('computer', 'vault_fill', 'request_login', 'screenshot')
                          OR e.payload->>'name' LIKE 'browser\_%')
               ) AS "desktop_active!",
               EXISTS(
                   SELECT 1
                   FROM login_handoffs h
                   WHERE h.agent_id = g.id
                     AND h.status = 'pending'
               ) AS "pending_login!",
               CASE WHEN 'conversation_read' = ANY(c.names) THEN tool.name END AS "current_tool?",
               CASE WHEN 'conversation_read' = ANY(c.names) THEN tool.target END AS "current_target?",
               COALESCE((SELECT max(t.sequence) FROM chat_turns t WHERE t.conversation_id = g.id), 0)
                   AS "last_sequence!",
               COALESCE((
                   SELECT r.sequence
                   FROM read_cursors r
                   WHERE r.account_id = a.id
                     AND r.scope_kind = 'conversation'
                     AND r.scope_id = g.id
               ), 0) AS "read_sequence!"
        FROM agents g
        JOIN human_accounts a ON a.id = $1 AND a.active
        CROSS JOIN LATERAL (
            SELECT CASE
                WHEN a.role = 'administrator' THEN ARRAY[
                    'agent_manage', 'conversation_read', 'conversation_send', 'memory_read',
                    'memory_edit', 'workspace_read', 'workspace_write', 'browser_view',
                    'browser_control']
                ELSE COALESCE((
                    SELECT array_agg(DISTINCT replace(p.capability, '.', '_'))
                    FROM effective_human_grants p
                    WHERE p.actor_id = a.id
                      AND p.scope_id = g.id
                      AND p.scope_kind IN ('agent', 'conversation')
                ), '{}')
            END AS names
        ) c
        LEFT JOIN LATERAL (
            -- The latest step of the running turn: a result means the tool has finished.
            SELECT CASE WHEN e.payload->>'kind' = 'tool_call' THEN e.payload->>'name' END AS name,
                   CASE WHEN e.payload->>'kind' = 'tool_call' THEN left(COALESCE(
                       e.payload->'input'->>'path', e.payload->'input'->>'url',
                       e.payload->'input'->>'name', e.payload->'input'->>'query',
                       e.payload->'input'->>'title'), 120) END AS target
            FROM chat_turns t
            JOIN turn_events e ON e.turn_id = t.id
            WHERE t.conversation_id = g.id
              AND t.status = 'running'
              AND e.payload->>'kind' IN ('tool_call', 'tool_result')
            ORDER BY e.id DESC
            LIMIT 1
        ) tool ON true
        WHERE ($2::uuid IS NULL OR g.id > $2)
          AND ($3::uuid IS NULL OR g.id = $3)
          AND cardinality(c.names) > 0
        ORDER BY g.id
        LIMIT $4
        "#,
            actor.id,
            after.map(|after| after.0),
            only.map(|only| only.0),
            limit,
        )
        .fetch_all(&self.pool)
        .await?)
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
            avatar: None,
        };
        assert!(matches!(validate(&input), Err(IdentityError::InvalidInput)));
    }
}

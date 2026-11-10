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

/// The agent's cube; mood is derived from activity, never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Avatar {
    pub paint: Paint,
    pub eyes: Eyes,
    pub accessory: Accessory,
    /// A custom paint chosen with the color picker; when set it replaces `paint` on the cube.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
}

/// An opaque `#rrggbb` color, stored lowercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(try_from = "String", into = "String")]
#[schema(value_type = String, pattern = "^#[0-9a-f]{6}$", example = "#3fa7a0")]
pub struct Color([u8; 3]);

impl TryFrom<String> for Color {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let invalid = || format!("{value} is not a #rrggbb color");
        let hex = value.strip_prefix('#').filter(|hex| hex.len() == 6);
        let hex = hex.ok_or_else(invalid)?;
        let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2).unwrap_or("-"), 16);
        match (byte(0), byte(2), byte(4)) {
            (Ok(r), Ok(g), Ok(b)) => Ok(Self([r, g, b])),
            _ => Err(invalid()),
        }
    }
}

impl From<Color> for String {
    fn from(Color([r, g, b]): Color) -> Self {
        format!("#{r:02x}{g:02x}{b:02x}")
    }
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
    Coral,
    Mint,
    Ocean,
    Charcoal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Eyes {
    Round,
    Wide,
    Sleepy,
    Dot,
    Happy,
    Wink,
    Star,
    Heart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Accessory {
    None,
    Cap,
    Glasses,
    Bow,
    Antenna,
    Crown,
    Headphones,
    Sprout,
    PartyHat,
    Monocle,
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
    /// The agent may ask to create agents, rooms and teams (migrations/0071_creation_requests.sql), on by default.
    pub creates: bool,
    /// `working`, `dreaming`, `sleeping` (idle past its delay) or `idle` while running,
    /// otherwise the desired state, `stopped` or `retired`. Dreaming needs a run whose sandbox
    /// still holds its credential.
    pub activity: String,
    /// The caller's capabilities on this agent and its conversation as grant names, such as
    /// `conversation_send`.
    pub capabilities: Vec<String>,
    #[schema(value_type = Avatar)]
    pub avatar: sqlx::types::Json<Avatar>,
    /// `None` inherits the installation's speech settings.
    pub stt_provider: Option<crate::speech::Provider>,
    pub tts_provider: Option<crate::speech::Provider>,
    pub voice: Option<crate::speech::Voice>,
    /// A running turn called a desktop tool in the last 120 seconds.
    pub desktop_active: bool,
    /// The agent is waiting for a person to log in on a site (docs/adr/sandbox.md#live-browser-viewing-and-explicit-human-takeover).
    pub pending_login: bool,
    /// Its requests to create agents, rooms or teams that wait for a person.
    pub pending_creations: i64,
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
    /// Left out, the agent speaks in the installation's voice. Absent from the digest when
    /// `None`, like the avatar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<crate::speech::Voice>,
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
            INSERT INTO agents (name, instructions, created_by, avatar, voice)
            VALUES ($1, $2, $3, COALESCE($4, '{\"paint\":\"orange\",\"eyes\":\"round\",\"accessory\":\"none\"}'::jsonb), $5)
            RETURNING id
            ",
            input.name,
            input.instructions,
            actor.id,
            input.avatar.map(sqlx::types::Json) as Option<sqlx::types::Json<Avatar>>,
            input.voice.map(crate::speech::Voice::as_str),
        )
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query!(
            "
            INSERT INTO skill_installations (agent_id, skill_id, revision)
            SELECT $1, id, revision
            FROM skills
            WHERE installs_by_default
            ",
            id,
        )
        .execute(&mut *transaction)
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
               g.idle_delay_seconds, g.dreaming_model, g.task_lanes, g.broadcast, g.knowledge_base, g.creates,
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
               g.stt_provider AS "stt_provider: crate::speech::Provider",
               g.tts_provider AS "tts_provider: crate::speech::Provider",
               g.voice AS "voice: crate::speech::Voice",
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
               (SELECT count(*) FROM creation_requests q WHERE q.agent_id = g.id AND q.status = 'pending')
                   AS "pending_creations!",
               CASE WHEN 'conversation_read' = ANY(c.names) THEN tool.name END AS "current_tool?",
               CASE WHEN 'conversation_read' = ANY(c.names) THEN tool.target END AS "current_target?",
               COALESCE((SELECT max(t.sequence) FROM chat_turns t WHERE t.conversation_id = g.id AND t.room_message_id IS NULL), 0)
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
            voice: None,
        };
        assert!(matches!(validate(&input), Err(IdentityError::InvalidInput)));
    }
}

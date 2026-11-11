//! Durable text conversations. Human access and machine execution are separate boundaries.
mod access;
mod activity;
mod broadcasts;
mod calls;
mod collaboration;
mod coordinator;
mod creations;
mod cursors;
mod events;
mod handoffs;
mod retry;
mod rooms;
pub mod routines;
mod runtime;
mod situation;
pub mod tasks;
mod teams;
pub(crate) mod terminal;
pub(crate) mod usage;
use crate::{
    agents::AgentId,
    identity::{Actor, IdentityError},
};
pub use activity::ActivityItem;
pub use broadcasts::{BroadcastFeed, BroadcastMessage};
pub use calls::{CallRecord, Reply, ReplyEvent};
pub use collaboration::Steering;
pub(crate) use collaboration::cancel_open;
pub use coordinator::{coordinate, route};
pub use creations::{CreationRequest, DeclineCreation};
pub use cursors::ReadCursor;
pub use events::{Event, Page};
pub use handoffs::{DeclineLogin, LoginHandoff};
pub use retry::RetryMessage;
pub use rooms::{CreateRoom, Delivery, Member, Room, RoomMembers, RoomMessage, RoomSequence};
use serde::{Deserialize, Serialize};
pub use situation::{OpenWork, RecentEvent, Situation, SituationAgent};
use sqlx::PgPool;
pub use teams::{CreateTeam, SaveMember, SaveTeam, Team, TeamMember, UpdateTeam};
pub use usage::{
    AgentUsage, DaySpend, ModelSpend, SetBudget, Spend, keep_prices, prices_due, store_prices,
};
use uuid::Uuid;

#[derive(Clone)]
pub struct Work {
    pub(crate) pool: PgPool,
}

#[derive(Serialize)]
pub struct Conversation {
    pub id: Uuid,
    pub agent_id: AgentId,
    pub created_by: Uuid,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SendMessage {
    pub request_id: Uuid,
    pub message: String,
    #[serde(default)]
    pub attachments: Vec<Uuid>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Turn {
    pub id: Uuid,
    pub sequence: i64,
    pub message: String,
    pub status: String,
    pub response: Option<String>,
    pub error: Option<String>,
    pub terminal: Vec<terminal::Progress>,
    /// Where the request came from when a person did not send it here directly.
    pub origin: Option<terminal::Origin>,
    /// Work this turn delegated or sent to other agents.
    pub children: Vec<terminal::Child>,
    pub steering: Vec<Steering>,
    /// Files the agent sent with this reply.
    pub attachments: Vec<terminal::ReplyAttachment>,
    pub message_attachments: Vec<terminal::ReplyAttachment>,
    /// Set when this message is a whole voice call, whose turns it holds.
    pub call: Option<CallRecord>,
}

pub(super) struct TurnRow {
    pub id: Uuid,
    pub sequence: i64,
    pub message: String,
    pub status: String,
    pub response: Option<String>,
    pub error: Option<String>,
}

impl From<TurnRow> for Turn {
    fn from(row: TurnRow) -> Self {
        Self {
            id: row.id,
            sequence: row.sequence,
            message: row.message,
            status: row.status,
            response: row.response,
            error: row.error,
            terminal: Vec::new(),
            origin: None,
            children: Vec::new(),
            steering: Vec::new(),
            attachments: Vec::new(),
            message_attachments: Vec::new(),
            call: None,
        }
    }
}

impl Work {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, agent = %agent.0))]
    pub async fn conversation(
        &self,
        actor: &Actor,
        agent: AgentId,
    ) -> Result<Conversation, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let canonical = access::conversation(&mut tx, actor, agent.0, "conversation.read").await?;
        if canonical != agent {
            return Err(IdentityError::NotFound);
        }
        let conversation = sqlx::query_as!(
            Conversation,
            r#"
            SELECT id AS "id!", agent_id AS "agent_id!: AgentId", created_by AS "created_by!"
            FROM conversations
            WHERE id=$1
            "#,
            agent.0,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(conversation)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, conversation = %conversation))]
    pub async fn history(
        &self,
        actor: &Actor,
        conversation: Uuid,
        after: i64,
    ) -> Result<Vec<Turn>, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let canonical =
            access::conversation(&mut tx, actor, conversation, "conversation.read").await?;
        let mut turns: Vec<Turn> = sqlx::query_as!(
            TurnRow,
            "
            SELECT id, sequence, message, status, response, error
            FROM chat_turns
            WHERE conversation_id=$1 AND room_message_id IS NULL AND call_id IS NULL AND sequence>$2
            ORDER BY sequence
            LIMIT 100
            ",
            canonical.0,
            after,
        )
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(Turn::from)
        .collect();
        Self::progress(&mut tx, &mut turns).await?;
        turns.extend(Self::calls_after(&mut tx, canonical.0, after).await?);
        turns.sort_by_key(|turn| turn.sequence);
        turns.truncate(100);
        tx.commit().await?;
        Ok(turns)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, conversation = %conversation))]
    pub async fn send(
        &self,
        actor: &Actor,
        conversation: Uuid,
        input: &SendMessage,
    ) -> Result<Turn, IdentityError> {
        self.send_in(actor, conversation, input, None).await
    }

    async fn send_in(
        &self,
        actor: &Actor,
        conversation: Uuid,
        input: &SendMessage,
        call: Option<Uuid>,
    ) -> Result<Turn, IdentityError> {
        validate(input)?;
        let mut tx = self.pool.begin().await?;
        access::lock_actor(&mut tx, actor).await?;
        let agent = access::conversation(&mut tx, actor, conversation, "conversation.send").await?;
        let conversation = agent.0;
        lock_agent(&mut tx, agent).await?;
        if let Some(previous) = replayed(&mut tx, actor, conversation, input).await? {
            return Ok(previous);
        }
        wake(&mut tx, agent).await?;
        let turn = sqlx::query_as!(
            TurnRow,
            "
            INSERT INTO chat_turns (conversation_id, actor_id, request_id, message, call_id)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, sequence, message, status, response, error
            ",
            conversation,
            actor.id,
            input.request_id,
            &input.message,
            call,
        )
        .fetch_one(&mut *tx)
        .await?;
        let mut turn = Turn::from(turn);
        turn.message_attachments = attach_sent(&mut tx, actor, turn.id, &input.attachments).await?;
        tx.commit().await?;
        Ok(turn)
    }
}

async fn lock_agent(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    agent: AgentId,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "
        SELECT id
        FROM agents
        WHERE id=$1
        FOR UPDATE
        ",
        agent.0,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(())
}

async fn attach_sent(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: &Actor,
    turn: Uuid,
    files: &[Uuid],
) -> Result<Vec<terminal::ReplyAttachment>, IdentityError> {
    owned(tx, actor, files).await?;
    sqlx::query!(
        "
        INSERT INTO turn_attachments (turn_id, file_id, inbound)
        SELECT $1, file, true FROM unnest($2::uuid[]) WITH ORDINALITY AS a(file, position)
        ORDER BY position
        ",
        turn,
        files,
    )
    .execute(&mut **tx)
    .await?;
    Ok(terminal::sent(tx, &[turn])
        .await?
        .remove(&turn)
        .unwrap_or_default())
}

pub(super) async fn owned(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: &Actor,
    files: &[Uuid],
) -> Result<(), IdentityError> {
    let owned = sqlx::query_scalar!(
        r#"
        SELECT count(*) AS "owned!"
        FROM files
        WHERE id=ANY($1) AND owner_account_id=$2 AND blob_id IS NOT NULL
        "#,
        files,
        actor.id,
    )
    .fetch_one(&mut **tx)
    .await?;
    (owned == files.len() as i64)
        .then_some(())
        .ok_or(IdentityError::InvalidInput)
}

async fn replayed(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: &Actor,
    conversation: Uuid,
    input: &SendMessage,
) -> Result<Option<Turn>, IdentityError> {
    let previous = sqlx::query!(
        "
        SELECT conversation_id, message, id
        FROM chat_turns
        WHERE actor_id=$1 AND request_id=$2
        ",
        actor.id,
        input.request_id,
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(previous) = previous else {
        return Ok(None);
    };
    if previous.conversation_id != conversation || previous.message != input.message {
        return Err(IdentityError::Conflict);
    }
    let turn = sqlx::query_as!(
        TurnRow,
        "
        SELECT id, sequence, message, status, response, error
        FROM chat_turns
        WHERE id=$1
        ",
        previous.id,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(Some(turn.into()))
}

pub(super) async fn wake(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    agent: AgentId,
) -> Result<(), IdentityError> {
    let eligible = sqlx::query_scalar!(
        "
        UPDATE agents
        SET restart_generation=restart_generation + CASE WHEN desired_state='stopped' THEN 1 ELSE 0 END,
            revision=revision + CASE WHEN desired_state='stopped' THEN 1 ELSE 0 END,
            desired_state='running'
        WHERE id=$1 AND desired_state<>'retired'
        RETURNING id
        ",
        agent.0,
    )
    .fetch_optional(&mut **tx)
    .await?;
    eligible.ok_or(IdentityError::Conflict)?;
    Ok(())
}

pub(super) fn validate(input: &SendMessage) -> Result<(), IdentityError> {
    let mut seen = std::collections::HashSet::new();
    let files = &input.attachments;
    if input.request_id.is_nil()
        || (input.message.trim().is_empty() && files.is_empty())
        || files.len() > terminal::ATTACHMENTS as usize
        || !files.iter().all(|file| seen.insert(file))
        || input.message.len() > 16384
        || input.message.contains('\0')
    {
        return Err(IdentityError::InvalidInput);
    }
    Ok(())
}

pub(super) fn text_only(input: &SendMessage) -> Result<(), IdentityError> {
    validate(input)?;
    input
        .attachments
        .is_empty()
        .then_some(())
        .ok_or(IdentityError::InvalidInput)
}

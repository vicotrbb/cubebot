//! Durable text conversations. Human access and machine execution are separate boundaries.
mod access;
mod collaboration;
mod coordinator;
mod events;
mod retry;
mod rooms;
mod runtime;
pub(crate) mod terminal;
use crate::{
    agents::AgentId,
    identity::{Actor, IdentityError},
};
pub use collaboration::Steering;
pub use coordinator::{coordinate, route};
pub use events::{Event, Page};
pub use retry::RetryMessage;
pub use rooms::{CreateRoom, Delivery, Member, Room, RoomMembers, RoomMessage};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct Work {
    pub(crate) pool: PgPool,
}

#[derive(Serialize, sqlx::FromRow)]
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
}

#[derive(Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Turn {
    pub id: Uuid,
    pub sequence: i64,
    pub message: String,
    pub status: String,
    pub response: Option<String>,
    pub error: Option<String>,
    #[sqlx(skip)]
    pub terminal: Vec<terminal::Progress>,
    /// Where the request came from when a person did not send it here directly.
    #[sqlx(skip)]
    pub origin: Option<terminal::Origin>,
    /// Work this turn delegated or sent to other agents.
    #[sqlx(skip)]
    pub children: Vec<terminal::Child>,
    #[sqlx(skip)]
    pub steering: Vec<Steering>,
}

impl Work {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

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
        let conversation =
            sqlx::query_as("SELECT id,agent_id,created_by FROM conversations WHERE id=$1")
                .bind(agent)
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        Ok(conversation)
    }

    pub async fn history(
        &self,
        actor: &Actor,
        conversation: Uuid,
        after: i64,
    ) -> Result<Vec<Turn>, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let canonical =
            access::conversation(&mut tx, actor, conversation, "conversation.read").await?;
        let mut turns = sqlx::query_as("SELECT id,sequence,message,status,response,error FROM chat_turns WHERE conversation_id=$1 AND sequence>$2 ORDER BY sequence LIMIT 100")
            .bind(canonical).bind(after).fetch_all(&mut *tx).await?;
        Self::progress(&mut tx, &mut turns).await?;
        tx.commit().await?;
        Ok(turns)
    }

    pub async fn send(
        &self,
        actor: &Actor,
        conversation: Uuid,
        input: &SendMessage,
    ) -> Result<Turn, IdentityError> {
        validate(input)?;
        let mut tx = self.pool.begin().await?;
        access::lock_actor(&mut tx, actor).await?;
        let agent = access::conversation(&mut tx, actor, conversation, "conversation.send").await?;
        let conversation = agent.0;
        sqlx::query("SELECT id FROM agents WHERE id=$1 FOR UPDATE")
            .bind(agent)
            .execute(&mut *tx)
            .await?;
        let previous: Option<(Uuid, String, Uuid)> = sqlx::query_as(
            "SELECT conversation_id,message,id FROM chat_turns WHERE actor_id=$1 AND request_id=$2",
        )
        .bind(actor.id)
        .bind(input.request_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((old_conversation, message, id)) = previous {
            if old_conversation != conversation || message != input.message {
                return Err(IdentityError::Conflict);
            }
            return Ok(sqlx::query_as(
                "SELECT id,sequence,message,status,response,error FROM chat_turns WHERE id=$1",
            )
            .bind(id)
            .fetch_one(&mut *tx)
            .await?);
        }
        wake(&mut tx, agent).await?;
        let turn = sqlx::query_as("INSERT INTO chat_turns (conversation_id,actor_id,request_id,message) VALUES ($1,$2,$3,$4) RETURNING id,sequence,message,status,response,error")
            .bind(conversation).bind(actor.id).bind(input.request_id).bind(&input.message).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(turn)
    }
}

pub(super) async fn wake(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    agent: AgentId,
) -> Result<(), IdentityError> {
    let eligible: Option<Uuid> = sqlx::query_scalar("UPDATE agents SET restart_generation=restart_generation + CASE WHEN desired_state='sleeping' THEN 1 ELSE 0 END, revision=revision + CASE WHEN desired_state='sleeping' THEN 1 ELSE 0 END, desired_state='running' WHERE id=$1 AND desired_state<>'retired' RETURNING id")
        .bind(agent).fetch_optional(&mut **tx).await?;
    eligible.ok_or(IdentityError::Conflict)?;
    Ok(())
}

pub(super) fn validate(input: &SendMessage) -> Result<(), IdentityError> {
    if input.request_id.is_nil()
        || input.message.trim().is_empty()
        || input.message.len() > 16384
        || input.message.contains('\0')
    {
        return Err(IdentityError::InvalidInput);
    }
    Ok(())
}

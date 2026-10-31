use super::{Turn, TurnRow, Work, access};
use crate::identity::{Actor, IdentityError};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RetryMessage {
    pub request_id: Uuid,
}

impl Work {
    #[tracing::instrument(skip_all, fields(actor = %actor.id, conversation = %conversation, turn = %turn))]
    pub async fn retry(
        &self,
        actor: &Actor,
        conversation: Uuid,
        turn: Uuid,
        input: &RetryMessage,
    ) -> Result<Turn, IdentityError> {
        if input.request_id.is_nil() {
            return Err(IdentityError::InvalidInput);
        }
        let mut tx = self.pool.begin().await?;
        access::lock_actor(&mut tx, actor).await?;
        let agent = access::conversation(&mut tx, actor, conversation, "conversation.send").await?;
        let conversation = agent.0;
        sqlx::query!(
            "
            SELECT id
            FROM agents
            WHERE id=$1
            FOR UPDATE
            ",
            agent.0,
        )
        .fetch_all(&mut *tx)
        .await?;
        let previous = sqlx::query_scalar!(
            "
            SELECT turn_id
            FROM chat_retries
            WHERE actor_id=$1 AND request_id=$2
            ",
            actor.id,
            input.request_id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        match previous {
            Some(id) if id == turn => (),
            Some(_) => return Err(IdentityError::Conflict),
            None => {
                requeue(&mut tx, actor, agent, turn, input.request_id).await?;
            }
        }
        let turn = read_turn(&mut tx, conversation, turn).await?;
        tx.commit().await?;
        Ok(turn)
    }
}

async fn requeue(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: &Actor,
    agent: crate::agents::AgentId,
    turn: Uuid,
    request: Uuid,
) -> Result<(), IdentityError> {
    // Never rerun a turn whose tool effects are unknown.
    let updated = sqlx::query_scalar!(
        "
        UPDATE chat_turns
        SET status='queued', error=NULL
        WHERE id=$1 AND conversation_id=$2 AND status='waiting'
            AND NOT EXISTS(SELECT 1 FROM turn_events c WHERE c.turn_id=chat_turns.id AND c.payload->>'kind'='tool_call' AND NOT EXISTS(SELECT 1 FROM turn_events r WHERE r.turn_id=c.turn_id AND r.payload->>'kind'='tool_result' AND r.payload->>'id'=c.payload->>'id' AND r.payload->'outcome'->>'status'<>'uncertain'))
        RETURNING id
        ",
        turn,
        agent.0,
    )
    .fetch_optional(&mut **tx)
    .await?;
    updated.ok_or(IdentityError::Conflict)?;
    super::wake(tx, agent).await?;
    sqlx::query!(
        "
        INSERT INTO chat_retries (actor_id, request_id, turn_id)
        VALUES ($1, $2, $3)
        ",
        actor.id,
        request,
        turn,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn read_turn(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    conversation: Uuid,
    turn: Uuid,
) -> Result<Turn, IdentityError> {
    sqlx::query_as!(
        TurnRow,
        "
        SELECT id, sequence, message, status, response, error
        FROM chat_turns
        WHERE id=$1 AND conversation_id=$2
        ",
        turn,
        conversation,
    )
    .fetch_optional(&mut **tx)
    .await?
    .map(Turn::from)
    .ok_or(IdentityError::NotFound)
}

use super::{Turn, Work, access};
use crate::identity::{Actor, IdentityError};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryMessage {
    pub request_id: Uuid,
    pub expected_authority: Option<Uuid>,
}

impl Work {
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
        sqlx::query("SELECT id FROM agents WHERE id=$1 FOR UPDATE")
            .bind(agent)
            .execute(&mut *tx)
            .await?;
        let previous: Option<(Uuid,Option<Uuid>)> = sqlx::query_as("SELECT turn_id,previous_authority FROM chat_retries WHERE actor_id=$1 AND request_id=$2")
            .bind(actor.id).bind(input.request_id).fetch_optional(&mut *tx).await?;
        match previous {
            Some((id, authority)) if id == turn && authority == input.expected_authority => (),
            Some(_) => return Err(IdentityError::Conflict),
            None => {
                requeue(&mut tx, actor, agent, turn, input).await?;
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
    input: &RetryMessage,
) -> Result<(), IdentityError> {
    let updated: Option<Uuid> = sqlx::query_scalar("UPDATE chat_turns SET status='queued',error=NULL,authority_id=NULL WHERE id=$1 AND conversation_id=$2 AND status='waiting' AND authority_id IS NOT DISTINCT FROM $3 RETURNING id")
                    .bind(turn).bind(agent).bind(input.expected_authority).fetch_optional(&mut **tx).await?;
    updated.ok_or(IdentityError::Conflict)?;
    super::wake(tx, agent).await?;
    sqlx::query("INSERT INTO chat_retries (actor_id,request_id,turn_id,previous_authority) VALUES ($1,$2,$3,$4)")
                    .bind(actor.id).bind(input.request_id).bind(turn).bind(input.expected_authority).execute(&mut **tx).await?;
    Ok(())
}

async fn read_turn(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    conversation: Uuid,
    turn: Uuid,
) -> Result<Turn, IdentityError> {
    sqlx::query_as("SELECT id,sequence,message,status,response,error,authority_id FROM chat_turns WHERE id=$1 AND conversation_id=$2")
        .bind(turn).bind(conversation).fetch_optional(&mut **tx).await?.ok_or(IdentityError::NotFound)
}

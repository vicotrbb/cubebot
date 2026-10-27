//! Immutable command admission and outcomes under a single text-turn authority.
use super::Work;
use crate::{
    agents::{kubernetes::Error, runtime::RuntimeActor},
    identity::{Actor, IdentityError},
};
use cubebot_protocol::terminal::{Authorization, Outcome};
use serde::Serialize;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Clone, Serialize, sqlx::FromRow)]
pub struct Progress {
    pub id: Uuid,
    pub turn_id: Uuid,
    pub command: String,
    pub status: String,
    pub outcome: Option<Value>,
}
pub(crate) async fn authorize(
    tx: &mut Transaction<'_, Postgres>,
    actor: &RuntimeActor,
    input: &Authorization,
) -> Result<u64, Error> {
    renew(
        tx,
        actor,
        &cubebot_protocol::terminal::Renewal {
            turn: input.turn,
            authority: input.invocation.authority,
        },
    )
    .await?;
    let request =
        serde_json::to_value(&input.invocation).map_err(|_| Error::InvalidConfiguration)?;
    let prior: Option<Value> =
        sqlx::query_scalar("SELECT request FROM terminal_invocations WHERE id=$1")
            .bind(input.invocation.id)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some(prior) = prior {
        if prior != request {
            return Err(Error::Conflict);
        }
        let renewed = sqlx::query("UPDATE terminal_invocations SET expires_at=(SELECT expires_at FROM chat_authorities WHERE id=$2) WHERE id=$1 AND expires_at>clock_timestamp() AND outcome IS NULL")
            .bind(input.invocation.id).bind(input.invocation.authority).execute(&mut **tx).await?;
        if renewed.rows_affected() != 1 {
            return Err(Error::Unauthorized);
        }
    } else {
        sqlx::query("INSERT INTO terminal_invocations(id,turn_id,authority_id,request,expires_at) SELECT $1,$2,id,$4,expires_at FROM chat_authorities WHERE id=$3")
            .bind(input.invocation.id).bind(input.turn).bind(input.invocation.authority).bind(request).execute(&mut **tx).await?;
    }
    remaining(tx, input.invocation.authority).await
}

pub(crate) async fn renew(
    tx: &mut Transaction<'_, Postgres>,
    actor: &RuntimeActor,
    input: &cubebot_protocol::terminal::Renewal,
) -> Result<u64, Error> {
    let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM chat_turns t JOIN chat_authorities g ON g.id=t.authority_id JOIN provider_connection p ON p.revision=g.provider_revision JOIN runtime_session_access s ON s.attempt_id=g.attempt_id WHERE t.id=$1 AND t.conversation_id=$2 AND g.id=$3 AND g.attempt_id=$4 AND g.expires_at>clock_timestamp() AND t.status IN ('running','retrying') AND g.terminal_enabled AND s.active AND (p.access->>'mode'='all_agents' OR p.access->'agents' @> jsonb_build_array($2::uuid)))")
        .bind(input.turn).bind(actor.agent).bind(input.authority).bind(actor.attempt).fetch_one(&mut **tx).await?;
    if !allowed || !super::runtime::sender_allowed(tx, input.turn).await? {
        return Err(Error::Unauthorized);
    }
    let renewed = sqlx::query("UPDATE chat_authorities SET expires_at=clock_timestamp()+interval '60 seconds' WHERE id=$1 AND expires_at>clock_timestamp()")
        .bind(input.authority).execute(&mut **tx).await?;
    if renewed.rows_affected() != 1 {
        return Err(Error::Unauthorized);
    }
    remaining(tx, input.authority).await
}
async fn remaining(tx: &mut Transaction<'_, Postgres>, authority: Uuid) -> Result<u64, Error> {
    let milliseconds: i64 = sqlx::query_scalar("SELECT floor(extract(epoch FROM (expires_at-clock_timestamp()))*1000)::bigint FROM chat_authorities WHERE id=$1")
        .bind(authority).fetch_one(&mut **tx).await?;
    u64::try_from(milliseconds).map_err(|_| Error::Unauthorized)
}
pub(crate) async fn outcome(
    tx: &mut Transaction<'_, Postgres>,
    authority: Uuid,
    id: Uuid,
    outcome: &Outcome,
) -> Result<(), Error> {
    outcome
        .validate()
        .map_err(|_| Error::InvalidConfiguration)?;
    let value = serde_json::to_value(outcome).map_err(|_| Error::InvalidConfiguration)?;
    if serde_json::to_vec(&value)
        .map_err(|_| Error::InvalidConfiguration)?
        .len()
        > 60 * 1024
    {
        return Err(Error::InvalidConfiguration);
    }
    let old: Option<Value> = sqlx::query_scalar(
        "SELECT outcome FROM terminal_invocations WHERE id=$1 AND authority_id=$2 FOR UPDATE",
    )
    .bind(id)
    .bind(authority)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(Error::Unauthorized)?;
    if let Some(old) = old {
        if old != value {
            return Err(Error::Conflict);
        }
        return Ok(());
    }
    sqlx::query("UPDATE terminal_invocations SET outcome=$2 WHERE id=$1")
        .bind(id)
        .bind(value)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
impl Work {
    pub async fn cancel(
        &self,
        actor: &Actor,
        conversation: Uuid,
        turn: Uuid,
    ) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        let canonical =
            super::access::conversation(&mut tx, actor, conversation, "conversation.send").await?;
        let changed = sqlx::query("UPDATE chat_turns SET status='cancelled',error='cancelled' WHERE id=$1 AND conversation_id=$2 AND status<>'complete'")
            .bind(turn).bind(canonical).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(IdentityError::Conflict);
        }
        tx.commit().await?;
        Ok(())
    }
    pub(super) async fn progress(
        tx: &mut Transaction<'_, Postgres>,
        turns: &mut [super::Turn],
    ) -> Result<(), IdentityError> {
        let ids: Vec<_> = turns.iter().map(|turn| turn.id).collect();
        let rows: Vec<Progress> = sqlx::query_as("SELECT id,turn_id,request->>'command' AS command, CASE WHEN outcome IS NOT NULL THEN outcome->>'status' WHEN expires_at<=clock_timestamp() THEN 'uncertain' ELSE 'running' END AS status,outcome FROM terminal_invocations WHERE turn_id=ANY($1) ORDER BY created_at,id").bind(ids).fetch_all(&mut **tx).await?;
        let mut grouped = std::collections::HashMap::<Uuid, Vec<Progress>>::new();
        for row in rows {
            grouped.entry(row.turn_id).or_default().push(row);
        }
        for turn in turns {
            turn.terminal = grouped.remove(&turn.id).unwrap_or_default();
        }
        Ok(())
    }
}

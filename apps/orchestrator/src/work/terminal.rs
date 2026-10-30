//! Terminal progress derived from turn events. A call without a result after its run ended is
//! uncertain: its effects may have happened.
use super::Work;
use crate::identity::{Actor, IdentityError};
use serde::Serialize;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Progress {
    pub id: Uuid,
    pub turn_id: Uuid,
    pub command: String,
    pub status: String,
    #[schema(value_type = Option<cubebot_protocol::terminal::Outcome>)]
    pub outcome: Option<Value>,
    /// The agent's first screenshot after a person handed its desktop back.
    pub reobserved: bool,
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
        let rows: Vec<Progress> = sqlx::query_as("SELECT (c.payload->>'id')::uuid AS id,c.turn_id,CASE WHEN c.payload->>'name' IN ('bash','terminal') THEN c.payload->'input'->>'command' ELSE (c.payload->>'name')||' '||(c.payload->'input')::text END AS command, CASE WHEN r.outcome IS NOT NULL THEN r.outcome->>'status' WHEN t.status='running' THEN 'running' ELSE 'uncertain' END AS status,r.outcome - 'image' AS outcome,EXISTS(SELECT 1 FROM desktop_reobservations o WHERE o.agent_id=t.conversation_id AND o.turn_id=c.turn_id AND o.call_id=c.payload->>'id') AS reobserved FROM turn_events c JOIN chat_turns t ON t.id=c.turn_id LEFT JOIN LATERAL (SELECT e.payload->'outcome' AS outcome FROM turn_events e WHERE e.turn_id=c.turn_id AND e.payload->>'kind'='tool_result' AND e.payload->>'id'=c.payload->>'id' ORDER BY e.sequence LIMIT 1) r ON true WHERE c.turn_id=ANY($1) AND c.payload->>'kind'='tool_call' ORDER BY c.turn_id,c.sequence").bind(ids).fetch_all(&mut **tx).await?;
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

use super::{Work, access};
use crate::identity::{Actor, IdentityError};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize, utoipa::ToSchema)]
pub struct Trace {
    pub turn: Uuid,
    pub status: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub events: Vec<TraceEvent>,
    pub memories: Vec<TraceMemory>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct TraceEvent {
    pub sequence: i64,
    pub at: String,
    #[schema(value_type = cubebot_protocol::runtime::TurnEvent)]
    pub payload: serde_json::Value,
    pub cost_usd: Option<f64>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct TraceMemory {
    pub id: Uuid,
    pub revision: i64,
    pub content: String,
    pub deleted: bool,
}

impl Work {
    #[tracing::instrument(skip_all, fields(actor = %actor.id, conversation = %conversation, turn = %turn))]
    pub async fn trace(
        &self,
        actor: &Actor,
        conversation: Uuid,
        turn: Uuid,
    ) -> Result<Trace, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let agent = access::conversation(&mut tx, actor, conversation, "conversation.read").await?;
        let head = sqlx::query!(
            r#"
            SELECT status, provider, model,
                (to_jsonb(created_at) #>> '{}') AS "created_at!",
                (to_jsonb(completed_at) #>> '{}') AS completed_at
            FROM chat_turns
            WHERE id=$1 AND conversation_id=$2
            "#,
            turn,
            agent.0,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        let events = sqlx::query_as!(
            TraceEvent,
            r#"
            SELECT e.sequence, (to_jsonb(e.created_at) #>> '{}') AS "at!",
                e.payload #- '{outcome,image}' AS "payload!",
                u.cost_usd::float8 AS cost_usd
            FROM turn_events e
            LEFT JOIN usage_events u ON u.turn_id=e.turn_id AND u.event_sequence=e.sequence
            WHERE e.turn_id=$1 AND e.payload->>'kind' NOT IN ('text_delta','done')
            ORDER BY e.sequence
            "#,
            turn,
        )
        .fetch_all(&mut *tx)
        .await?;
        let memories = sqlx::query_as!(
            TraceMemory,
            r#"
            SELECT r.memory_id AS id, r.revision, r.content, r.deleted
            FROM memory_revisions r
            JOIN human_accounts a ON a.id=$2 AND a.active
            WHERE r.turn_id=$1
                AND (a.role='administrator' OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id=a.id AND p.scope_kind='agent' AND p.scope_id=$3 AND p.capability='memory.read'))
            ORDER BY r.created_at
            "#,
            turn,
            actor.id,
            agent.0,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Trace {
            turn,
            status: head.status,
            provider: head.provider,
            model: head.model,
            created_at: head.created_at,
            completed_at: head.completed_at,
            events,
            memories,
        })
    }
}

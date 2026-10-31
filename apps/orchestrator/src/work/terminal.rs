//! Terminal progress derived from turn events. A call without a result after its run ended is
//! uncertain: its effects may have happened.
use super::Work;
use crate::identity::{Actor, IdentityError};
use serde::Serialize;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Clone, Serialize, utoipa::ToSchema)]
pub struct Progress {
    pub id: Uuid,
    pub turn_id: Uuid,
    /// The tool's name, such as `bash` or `delegate`.
    pub name: String,
    pub command: String,
    pub status: String,
    #[schema(value_type = Option<cubebot_protocol::terminal::Outcome>)]
    pub outcome: Option<Value>,
    /// The agent's first screenshot after a person handed its desktop back.
    pub reobserved: bool,
}

#[derive(Clone, Serialize, utoipa::ToSchema)]
pub struct Origin {
    #[serde(skip)]
    pub turn_id: Uuid,
    /// `room`, `delegation` or `message`.
    pub kind: String,
    /// The room's name, or the sending agent's.
    pub name: String,
    pub parent_turn_id: Option<Uuid>,
}

#[derive(Clone, Serialize, utoipa::ToSchema)]
pub struct Child {
    pub id: Uuid,
    #[serde(skip)]
    pub parent_turn_id: Uuid,
    pub agent_id: Uuid,
    pub agent: String,
    pub status: String,
    /// The parent yields until this child settles.
    pub awaited: bool,
    pub response: Option<String>,
    pub error: Option<String>,
}

struct TurnSteering {
    turn_id: Uuid,
    steering: super::Steering,
}
impl Work {
    #[tracing::instrument(skip_all, fields(actor = %actor.id, conversation = %conversation, turn = %turn))]
    pub async fn cancel(
        &self,
        actor: &Actor,
        conversation: Uuid,
        turn: Uuid,
    ) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        let canonical =
            super::access::conversation(&mut tx, actor, conversation, "conversation.send").await?;
        super::collaboration::cancel_subtree(&mut tx, canonical, turn).await?;
        tx.commit().await?;
        Ok(())
    }
    pub(super) async fn progress(
        tx: &mut Transaction<'_, Postgres>,
        turns: &mut [super::Turn],
    ) -> Result<(), IdentityError> {
        let ids: Vec<_> = turns.iter().map(|turn| turn.id).collect();
        let rows = sqlx::query_as!(
            Progress,
            r#"
            SELECT (c.payload->>'id')::uuid AS "id!",
                c.turn_id,
                c.payload->>'name' AS "name!",
                CASE WHEN c.payload->>'name' IN ('bash','terminal') THEN c.payload->'input'->>'command' ELSE (c.payload->>'name')||' '||(c.payload->'input')::text END AS "command!",
                CASE WHEN r.outcome IS NOT NULL THEN r.outcome->>'status' WHEN t.status='running' THEN 'running' ELSE 'uncertain' END AS "status!",
                r.outcome - 'image' AS "outcome?",
                EXISTS(SELECT 1 FROM desktop_reobservations o WHERE o.agent_id=t.conversation_id AND o.turn_id=c.turn_id AND o.call_id=c.payload->>'id') AS "reobserved!"
            FROM turn_events c
            JOIN chat_turns t ON t.id=c.turn_id
            LEFT JOIN LATERAL (SELECT e.payload->'outcome' AS outcome FROM turn_events e WHERE e.turn_id=c.turn_id AND e.payload->>'kind'='tool_result' AND e.payload->>'id'=c.payload->>'id' ORDER BY e.sequence LIMIT 1) r ON true
            WHERE c.turn_id=ANY($1) AND c.payload->>'kind'='tool_call'
            ORDER BY c.turn_id, c.sequence
            "#,
            &ids,
        )
        .fetch_all(&mut **tx)
        .await?;
        let mut grouped = std::collections::HashMap::<Uuid, Vec<Progress>>::new();
        for row in rows {
            grouped.entry(row.turn_id).or_default().push(row);
        }
        for turn in turns.iter_mut() {
            turn.terminal = grouped.remove(&turn.id).unwrap_or_default();
        }
        Self::collaboration(tx, turns, &ids).await
    }

    async fn collaboration(
        tx: &mut Transaction<'_, Postgres>,
        turns: &mut [super::Turn],
        ids: &[Uuid],
    ) -> Result<(), IdentityError> {
        let origins = sqlx::query_as!(
            Origin,
            r#"
            SELECT t.id AS turn_id,
                CASE WHEN r.id IS NOT NULL THEN 'room' WHEN t.awaited THEN 'delegation' ELSE 'message' END AS "kind!",
                COALESCE(r.name,a.name) AS "name!",
                t.parent_turn_id
            FROM chat_turns t
            LEFT JOIN room_messages m ON m.id=t.room_message_id
            LEFT JOIN rooms r ON r.id=m.room_id
            LEFT JOIN chat_turns p ON p.id=t.parent_turn_id
            LEFT JOIN agents a ON a.id=p.conversation_id
            WHERE t.id=ANY($1) AND (r.id IS NOT NULL OR p.id IS NOT NULL)
            "#,
            ids,
        )
        .fetch_all(&mut **tx)
        .await?;
        let children = sqlx::query_as!(
            Child,
            r#"
            SELECT c.id, c.parent_turn_id AS "parent_turn_id!", c.conversation_id AS agent_id, a.name AS agent, c.status, c.awaited, c.response, c.error
            FROM chat_turns c
            JOIN agents a ON a.id=c.conversation_id
            WHERE c.parent_turn_id=ANY($1)
            ORDER BY c.sequence
            "#,
            ids,
        )
        .fetch_all(&mut **tx)
        .await?;
        let steering: Vec<TurnSteering> = sqlx::query!(
            r#"
            SELECT turn_id, id, message, incorporated_at IS NOT NULL AS "incorporated!", source_steering_id IS NOT NULL AS "propagated!"
            FROM turn_steering
            WHERE turn_id=ANY($1)
            ORDER BY created_at, id
            "#,
            ids,
        )
        .fetch_all(&mut **tx)
        .await?
        .into_iter()
        .map(|row| TurnSteering {
            turn_id: row.turn_id,
            steering: super::Steering {
                id: row.id,
                message: row.message,
                incorporated: row.incorporated,
                propagated: row.propagated,
            },
        })
        .collect();
        for turn in turns {
            turn.origin = origins
                .iter()
                .find(|origin| origin.turn_id == turn.id)
                .cloned();
            turn.children = children
                .iter()
                .filter(|child| child.parent_turn_id == turn.id)
                .cloned()
                .collect();
            turn.steering = steering
                .iter()
                .filter(|row| row.turn_id == turn.id)
                .map(|row| row.steering.clone())
                .collect();
        }
        Ok(())
    }
}

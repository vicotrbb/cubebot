//! Terminal progress derived from turn events. A call without a result after its run ended is
//! uncertain: its effects may have happened.
use super::Work;
use crate::{
    agents::AgentId,
    identity::{Actor, IdentityError},
};
use serde::Serialize;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use std::collections::HashMap;
use uuid::Uuid;

/// Files one reply may carry.
const ATTACHMENTS: i64 = 10;

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
    /// `broadcast`, `room`, `routine`, `delegation` or `message`.
    pub kind: String,
    /// The room's or routine's name, the sending agent's, or a broadcast's sender.
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
/// A file the agent sent with its reply. Its bytes are at
/// `GET /api/attachments/{turn}/{file}/content`, for whoever may read the reply.
#[derive(Clone, Serialize, utoipa::ToSchema)]
pub struct ReplyAttachment {
    pub file: Uuid,
    pub name: String,
    pub content_type: Option<String>,
    pub size: i64,
}

/// Each turn's attachments, oldest first.
pub(super) async fn attachments(
    tx: &mut Transaction<'_, Postgres>,
    turns: &[Uuid],
) -> Result<HashMap<Uuid, Vec<ReplyAttachment>>, sqlx::Error> {
    let rows = sqlx::query!(
        "
        SELECT a.turn_id, f.id, f.name, f.content_type, b.size
        FROM turn_attachments a
        JOIN files f ON f.id=a.file_id
        JOIN blobs b ON b.id=f.blob_id
        WHERE a.turn_id=ANY($1)
        ORDER BY a.created_at
        ",
        turns,
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut grouped = HashMap::<Uuid, Vec<ReplyAttachment>>::new();
    for row in rows {
        grouped
            .entry(row.turn_id)
            .or_default()
            .push(ReplyAttachment {
                file: row.id,
                name: row.name,
                content_type: row.content_type,
                size: row.size,
            });
    }
    Ok(grouped)
}

/// Each turn's tool calls so far, in order.
pub(super) async fn tool_progress(
    tx: &mut Transaction<'_, Postgres>,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<Progress>>, sqlx::Error> {
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
        ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut grouped = HashMap::<Uuid, Vec<Progress>>::new();
    for row in rows {
        grouped.entry(row.turn_id).or_default().push(row);
    }
    Ok(grouped)
}

impl Work {
    /// Sends one of the agent's own files with its running turn's reply. The inner error is the
    /// refusal the model reads.
    #[tracing::instrument(skip_all, fields(agent = %actor.agent.0, turn = %turn, file = %file))]
    pub(crate) async fn attach_file(
        &self,
        actor: &crate::agents::runtime::RuntimeActor,
        turn: Uuid,
        file: Uuid,
    ) -> Result<Result<(), String>, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let running = sqlx::query_scalar!(
            "
            SELECT id
            FROM chat_turns
            WHERE id=$1 AND conversation_id=$2 AND status='running' AND attempt_id=$3
            FOR UPDATE
            ",
            turn,
            actor.agent.0,
            actor.attempt.0,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if running.is_none() {
            return Ok(Err("this turn is no longer running".into()));
        }
        let held = sqlx::query_scalar!(
            r#"SELECT count(*) AS "held!" FROM turn_attachments WHERE turn_id=$1"#,
            turn,
        )
        .fetch_one(&mut *tx)
        .await?;
        if held >= ATTACHMENTS {
            return Ok(Err(format!("a reply carries at most {ATTACHMENTS} files")));
        }
        let attached = sqlx::query!(
            "
            INSERT INTO turn_attachments (turn_id, file_id)
            SELECT $1, f.id
            FROM files f
            WHERE f.id=$2 AND f.owner_agent_id=$3 AND f.blob_id IS NOT NULL
            ON CONFLICT DO NOTHING
            ",
            turn,
            file,
            actor.agent.0,
        )
        .execute(&mut *tx)
        .await?;
        let known = attached.rows_affected() == 1
            || sqlx::query_scalar!(
                r#"SELECT EXISTS(SELECT 1 FROM turn_attachments WHERE turn_id=$1 AND file_id=$2) AS "known!""#,
                turn,
                file,
            )
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(known
            .then_some(())
            .ok_or_else(|| "that is not a file of yours".to_owned()))
    }

    /// The agent whose file this is, when the caller may read the reply it came with: in the
    /// agent's conversation, or in the room the turn belongs to.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, turn = %turn, file = %file))]
    pub async fn attachment_owner(
        &self,
        actor: &Actor,
        turn: Uuid,
        file: Uuid,
    ) -> Result<AgentId, IdentityError> {
        sqlx::query_scalar!(
            r#"
            SELECT t.conversation_id AS "agent: AgentId"
            FROM turn_attachments x
            JOIN chat_turns t ON t.id=x.turn_id
            LEFT JOIN room_messages m ON m.id=t.room_message_id
            LEFT JOIN tasks k ON k.room_id=m.room_id
            JOIN human_accounts a ON a.id=$1 AND a.active
            WHERE x.turn_id=$2 AND x.file_id=$3
                AND (a.role='administrator'
                    OR k.created_by=a.id
                    OR k.id IS NULL AND EXISTS(
                    SELECT 1 FROM effective_human_grants g
                    WHERE g.actor_id=a.id AND g.capability='conversation.read'
                        AND ((g.scope_kind='conversation' AND g.scope_id=t.conversation_id)
                            OR (g.scope_kind='room' AND g.scope_id=m.room_id))
                ))
            "#,
            actor.id,
            turn,
            file,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::NotFound)
    }

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
        super::tasks::guard(&mut tx, actor, turn).await?;
        super::collaboration::cancel_subtree(&mut tx, canonical, turn).await?;
        tx.commit().await?;
        Ok(())
    }
    pub(super) async fn progress(
        tx: &mut Transaction<'_, Postgres>,
        turns: &mut [super::Turn],
    ) -> Result<(), IdentityError> {
        let ids: Vec<_> = turns.iter().map(|turn| turn.id).collect();
        let mut grouped = tool_progress(tx, &ids).await?;
        let mut attachments = attachments(tx, &ids).await?;
        for turn in turns.iter_mut() {
            turn.terminal = grouped.remove(&turn.id).unwrap_or_default();
            turn.attachments = attachments.remove(&turn.id).unwrap_or_default();
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
                CASE WHEN b.id IS NOT NULL THEN 'broadcast' WHEN r.id IS NOT NULL THEN 'room' WHEN t.origin IS NOT NULL THEN 'routine' WHEN t.awaited THEN 'delegation' ELSE 'message' END AS "kind!",
                COALESCE(ba.name,bh.login,r.name,u.name,CASE WHEN t.origin IS NOT NULL THEN 'a deleted routine' END,a.name) AS "name!",
                t.parent_turn_id
            FROM chat_turns t
            LEFT JOIN room_messages m ON m.id=t.room_message_id
            LEFT JOIN rooms r ON r.id=m.room_id
            LEFT JOIN routines u ON u.id=t.routine_id
            LEFT JOIN chat_turns p ON p.id=t.parent_turn_id
            LEFT JOIN agents a ON a.id=p.conversation_id
            LEFT JOIN broadcasts b ON b.id=t.broadcast_id
            LEFT JOIN agents ba ON ba.id=b.agent_id
            LEFT JOIN human_accounts bh ON bh.id=b.actor_id AND b.agent_id IS NULL
            WHERE t.id=ANY($1) AND (r.id IS NOT NULL OR p.id IS NOT NULL OR t.origin IS NOT NULL OR b.id IS NOT NULL)
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

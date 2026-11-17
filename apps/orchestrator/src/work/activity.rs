use super::{Work, access};
use crate::identity::{Actor, IdentityError};
use serde::Serialize;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Serialize, utoipa::ToSchema)]
pub struct ActivityItem {
    /// `approval_pending`, `attention`, `login_pending`, `turn_waiting`, `turn_failed`,
    /// `invitation_pending`, `dream_published_skill` or `agent_published_skill`.
    pub kind: String,
    pub agent: Option<Uuid>,
    pub room: Option<Uuid>,
    /// The task whose room this is; such work is answered on the Tasks page.
    pub task: Option<Uuid>,
    pub turn: Option<Uuid>,
    /// RFC 3339.
    pub at: String,
    pub text: String,
    /// It reached the inbox after the caller last opened it.
    pub unread: bool,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct UnreadReply {
    pub agent: Uuid,
    pub turn: Uuid,
    pub sequence: i64,
    pub at: String,
    pub excerpt: String,
    pub unread: i64,
}

impl Work {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn activity(&self, actor: &Actor) -> Result<Vec<ActivityItem>, IdentityError> {
        Ok(sqlx::query_as!(
            ActivityItem,
            r#"
            SELECT i.kind AS "kind!", i.agent AS "agent?", i.room AS "room?",
                   (SELECT k.id FROM tasks k WHERE k.room_id=i.room) AS "task?", i.turn AS "turn?",
                   (to_jsonb(i.at) #>> '{}') AS "at!", i.text AS "text!",
                   i.at > COALESCE(a.activity_read_at, '-infinity') AS "unread!"
            FROM human_accounts a
            CROSS JOIN LATERAL (
                SELECT 'login_pending' AS kind, g.id AS agent, NULL::uuid AS room, h.turn_id AS turn,
                       h.created_at AS at, format('Asks you to log in on %s', h.site) AS text
                FROM login_handoffs h
                JOIN agents g ON g.id=h.agent_id
                WHERE h.status='pending'
                  AND (a.role='administrator' OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id=a.id AND p.scope_id=g.id AND p.scope_kind IN ('agent','conversation')))
                UNION ALL
                SELECT CASE t.status WHEN 'waiting' THEN 'turn_waiting' WHEN 'failed' THEN 'turn_failed' ELSE 'attention' END,
                       g.id, m.room_id, t.id, t.status_at,
                       CASE
                           WHEN t.status NOT IN ('waiting','failed') THEN t.attention
                           WHEN t.error='budget_exhausted' THEN 'Spent its budget; work is paused'
                           WHEN t.status='waiting' THEN 'Stopped and waits for you to retry'
                           ELSE format('Failed: %s', left(COALESCE(t.error, ''), 200))
                       END
                FROM chat_turns t
                JOIN agents g ON g.id=t.conversation_id
                LEFT JOIN room_messages m ON m.id=t.room_message_id
                LEFT JOIN tasks k ON k.room_id=m.room_id
                WHERE (t.status IN ('waiting','failed') OR (t.attention IS NOT NULL AND t.attention_settled_at IS NULL AND t.status<>'cancelled'))
                  AND (a.role='administrator' OR k.created_by=a.id OR (k.id IS NULL AND EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id=a.id AND p.scope_kind='conversation' AND p.scope_id=g.id AND p.capability='conversation.read')
                      AND (m.room_id IS NULL OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id=a.id AND p.scope_kind='room' AND p.scope_id=m.room_id AND p.capability='conversation.read'))))
                UNION ALL
                SELECT 'approval_pending', g.id, NULL, q.turn_id, q.created_at,
                       CASE WHEN q.kind='change' THEN format('Asks to change the agent %s', q.input->>'agent')
                            ELSE format('Asks to create the %s %s', q.kind, q.input->>'name') END
                FROM creation_requests q
                JOIN agents g ON g.id=q.agent_id
                WHERE q.status='pending'
                  AND (a.role='administrator' OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id=a.id AND p.scope_kind='conversation' AND p.scope_id=g.id AND p.capability='conversation.send'))
                UNION ALL
                SELECT 'invitation_pending', NULL, NULL, NULL, v.created_at, format('Invitation for %s is pending', v.login)
                FROM invitations v
                WHERE a.role='administrator' AND v.accepted_by IS NULL AND v.revoked_by IS NULL AND v.expires_at > clock_timestamp()
                UNION ALL
                SELECT CASE WHEN r.turn_id IS NULL THEN 'dream_published_skill' ELSE 'agent_published_skill' END,
                       g.id, NULL, r.turn_id, r.created_at,
                       format('%s skill %s', CASE WHEN r.revision=1 THEN 'Published' ELSE 'Improved' END, s.name)
                FROM skill_revisions r
                LEFT JOIN dream_runs d ON d.id=r.dream_run_id
                LEFT JOIN chat_turns t ON t.id=r.turn_id
                JOIN agents g ON g.id=COALESCE(d.agent_id, t.conversation_id)
                JOIN skills s ON s.id=r.skill_id
                WHERE r.created_at > COALESCE(a.activity_read_at, '-infinity')
                  AND (a.role='administrator' OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id=a.id AND p.scope_kind='agent' AND p.scope_id=g.id AND p.capability IN ('memory.read','agent.manage')))
            ) i
            WHERE a.id=$1 AND a.active
            ORDER BY i.at DESC
            LIMIT 100
            "#,
            actor.id,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn unread_replies(&self, actor: &Actor) -> Result<Vec<UnreadReply>, IdentityError> {
        Ok(sqlx::query_as!(
            UnreadReply,
            r#"
            SELECT g.id AS agent, t.id AS turn, t.sequence, (to_jsonb(t.status_at) #>> '{}') AS "at!",
                   left(t.response, 600) AS "excerpt!", n.unread AS "unread!"
            FROM agents g
            JOIN human_accounts a ON a.id=$1 AND a.active
            CROSS JOIN LATERAL (
                SELECT COALESCE((SELECT r.sequence FROM read_cursors r WHERE r.account_id=a.id AND r.scope_kind='conversation' AND r.scope_id=g.id), 0) AS seen
            ) c
            JOIN LATERAL (
                SELECT u.id, u.sequence, u.status_at, u.response
                FROM chat_turns u
                WHERE u.conversation_id=g.id AND u.room_message_id IS NULL AND u.sequence > c.seen
                  AND u.status='complete' AND u.response <> ''
                ORDER BY u.sequence DESC
                LIMIT 1
            ) t ON true
            CROSS JOIN LATERAL (
                SELECT count(*) AS unread
                FROM chat_turns u
                WHERE u.conversation_id=g.id AND u.room_message_id IS NULL AND u.sequence > c.seen
                  AND u.status='complete' AND u.response <> ''
            ) n
            WHERE g.desired_state<>'retired'
              AND (a.role='administrator' OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id=a.id AND p.scope_kind='conversation' AND p.scope_id=g.id AND p.capability='conversation.read'))
            ORDER BY t.status_at DESC
            "#,
            actor.id,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn read_activity(&self, actor: &Actor) -> Result<(), IdentityError> {
        sqlx::query!(
            "
            UPDATE human_accounts
            SET activity_read_at=clock_timestamp()
            WHERE id=$1 AND active
            ",
            actor.id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, turn = %turn))]
    pub async fn settle_attention(&self, actor: &Actor, turn: Uuid) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        let conversation = sqlx::query_scalar!(
            "SELECT conversation_id FROM chat_turns WHERE id=$1 AND attention IS NOT NULL",
            turn,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        access::conversation(&mut tx, actor, conversation, "conversation.send").await?;
        sqlx::query!(
            "UPDATE chat_turns SET attention_settled_at=clock_timestamp() WHERE id=$1 AND attention_settled_at IS NULL",
            turn,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

pub(crate) async fn attention(
    tx: &mut Transaction<'_, Postgres>,
    turn: Uuid,
    input: &serde_json::Value,
) -> Result<Result<String, String>, IdentityError> {
    let summary = input["summary"]
        .as_str()
        .map(str::trim)
        .filter(|summary| !summary.is_empty() && summary.len() <= 280 && !summary.contains('\0'));
    let Some(summary) = summary else {
        return Ok(Err("summary is one line of 1 to 280 bytes".into()));
    };
    sqlx::query!(
        "UPDATE chat_turns SET attention=$2, attention_settled_at=NULL WHERE id=$1",
        turn,
        summary,
    )
    .execute(&mut **tx)
    .await?;
    Ok(Ok("It shows under Needs you in their Activity until they answer or mark it done. Ask for what you need in your reply, then end your turn.".into()))
}

pub(super) async fn settle_conversation(
    tx: &mut Transaction<'_, Postgres>,
    conversation: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "
        UPDATE chat_turns
        SET attention_settled_at=clock_timestamp()
        WHERE conversation_id=$1 AND room_message_id IS NULL AND attention IS NOT NULL AND attention_settled_at IS NULL
        ",
        conversation,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(super) async fn settle_room(
    tx: &mut Transaction<'_, Postgres>,
    room: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "
        UPDATE chat_turns t
        SET attention_settled_at=clock_timestamp()
        FROM room_messages m
        WHERE m.id=t.room_message_id AND m.room_id=$1 AND t.attention IS NOT NULL AND t.attention_settled_at IS NULL
        ",
        room,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

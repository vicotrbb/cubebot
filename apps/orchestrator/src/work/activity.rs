//! The activity inbox: what needs a person, derived from the tables that already record it.
use super::Work;
use crate::identity::{Actor, IdentityError};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize, utoipa::ToSchema)]
pub struct ActivityItem {
    /// `login_pending`, `turn_waiting`, `turn_failed`, `invitation_pending` or
    /// `dream_published_skill`.
    pub kind: String,
    pub agent: Option<Uuid>,
    pub room: Option<Uuid>,
    pub turn: Option<Uuid>,
    /// RFC 3339.
    pub at: String,
    pub text: String,
}

impl Work {
    /// Newest first, at most 100: pending logins on agents the caller may see, waiting and
    /// failed turns in conversations they may read, pending invitations for administrators,
    /// and skills dreams published on agents whose memory they may read or manage.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn activity(&self, actor: &Actor) -> Result<Vec<ActivityItem>, IdentityError> {
        Ok(sqlx::query_as!(
            ActivityItem,
            r#"
            SELECT i.kind AS "kind!", i.agent AS "agent?", i.room AS "room?", i.turn AS "turn?",
                   (to_jsonb(i.at) #>> '{}') AS "at!", i.text AS "text!"
            FROM human_accounts a
            CROSS JOIN LATERAL (
                SELECT 'login_pending' AS kind, g.id AS agent, NULL::uuid AS room, h.turn_id AS turn,
                       h.created_at AS at, format('%s asks you to log in on %s', g.name, h.site) AS text
                FROM login_handoffs h
                JOIN agents g ON g.id=h.agent_id
                WHERE h.status='pending'
                  AND (a.role='administrator' OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id=a.id AND p.scope_id=g.id AND p.scope_kind IN ('agent','conversation')))
                UNION ALL
                SELECT CASE t.status WHEN 'waiting' THEN 'turn_waiting' ELSE 'turn_failed' END, g.id, m.room_id, t.id,
                       t.created_at,
                       CASE t.status
                           WHEN 'waiting' THEN format('%s is waiting for your reply', g.name)
                           ELSE format('%s failed: %s', g.name, left(COALESCE(t.error, ''), 200))
                       END
                FROM chat_turns t
                JOIN agents g ON g.id=t.conversation_id
                LEFT JOIN room_messages m ON m.id=t.room_message_id
                WHERE t.status IN ('waiting','failed')
                  AND (a.role='administrator' OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id=a.id AND p.scope_kind='conversation' AND p.scope_id=g.id AND p.capability='conversation.read'))
                UNION ALL
                SELECT 'invitation_pending', NULL, NULL, NULL, v.created_at, format('Invitation for %s is pending', v.login)
                FROM invitations v
                WHERE a.role='administrator' AND v.accepted_by IS NULL AND v.revoked_by IS NULL AND v.expires_at > clock_timestamp()
                UNION ALL
                SELECT 'dream_published_skill', g.id, NULL, NULL, r.created_at, format('%s published skill %s', g.name, s.name)
                FROM skill_revisions r
                JOIN dream_runs d ON d.id=r.dream_run_id
                JOIN agents g ON g.id=d.agent_id
                JOIN skills s ON s.id=r.skill_id
                WHERE a.role='administrator' OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id=a.id AND p.scope_kind='agent' AND p.scope_id=g.id AND p.capability IN ('memory.read','agent.manage'))
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
}

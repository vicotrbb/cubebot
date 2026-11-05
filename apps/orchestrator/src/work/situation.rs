use super::{
    Work,
    usage::{self, DaySpend, ModelSpend, Spend},
};
use crate::identity::{Actor, IdentityError};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize, utoipa::ToSchema)]
pub struct Situation {
    pub today: Spend,
    pub week: Spend,
    pub month: Spend,
    pub days: Vec<DaySpend>,
    pub models: Vec<ModelSpend>,
    pub agents: Vec<SituationAgent>,
    /// Queued, running, delegated and waiting turns, oldest first, at most 50.
    pub work: Vec<OpenWork>,
    /// The latest settled turns, dreams and published skills, newest first.
    pub recent: Vec<RecentEvent>,
    /// The provider and model in use; administrators only.
    pub provider: Option<String>,
    pub model: Option<String>,
    /// When prices last refreshed, RFC 3339; `None` means every cost is unknown.
    pub prices_at: Option<String>,
    pub stored_bytes: i64,
    pub people: i64,
    pub rooms: i64,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct SituationAgent {
    pub id: Uuid,
    pub open_turns: i64,
    pub failed_today: i64,
    pub turns_week: i64,
    pub tokens_week: i64,
    pub cost_week_usd: f64,
    pub cost_month_usd: f64,
    pub monthly_budget_usd: Option<f64>,
    pub last_turn_at: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct OpenWork {
    pub turn: Uuid,
    pub agent: Uuid,
    pub room: Option<Uuid>,
    pub status: String,
    pub error: Option<String>,
    /// The start of the request.
    pub message: String,
    pub since: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct RecentEvent {
    /// `turn_complete`, `turn_failed`, `turn_cancelled`, `dream` or `skill`.
    pub kind: String,
    pub agent: Uuid,
    pub turn: Option<Uuid>,
    pub room: Option<Uuid>,
    pub text: String,
    pub at: String,
}

impl Work {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn situation(&self, actor: &Actor) -> Result<Situation, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let viewer = sqlx::query!(
            r#"
            SELECT a.role='administrator' AS "administrator!",
                COALESCE((
                    SELECT array_agg(g.id)
                    FROM agents g
                    WHERE a.role='administrator' OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id=a.id AND p.scope_kind='conversation' AND p.scope_id=g.id AND p.capability='conversation.read')
                ), '{}') AS "agents!"
            FROM human_accounts a
            WHERE a.id=$1 AND a.active
            "#,
            actor.id,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::Unauthorized)?;
        let (scope, administrator) = (viewer.agents, viewer.administrator);
        let window = usage::windows(&mut tx, &scope, administrator).await?;
        let days = usage::days(&mut tx, &scope, administrator).await?;
        let models = usage::models(&mut tx, &scope, administrator).await?;
        let agents = sqlx::query_as!(
            SituationAgent,
            r#"
            SELECT g.id,
                (SELECT count(*) FROM chat_turns t WHERE t.conversation_id=g.id AND t.status IN ('queued','running','delegated','waiting')) AS "open_turns!",
                (SELECT count(*) FROM chat_turns t WHERE t.conversation_id=g.id AND t.status='failed' AND t.status_at >= date_trunc('day', clock_timestamp(), 'UTC')) AS "failed_today!",
                (SELECT count(*) FROM chat_turns t WHERE t.conversation_id=g.id AND t.created_at > clock_timestamp() - interval '7 days') AS "turns_week!",
                COALESCE((SELECT sum(u.input_tokens + u.output_tokens) FROM usage_events u WHERE u.agent_id=g.id AND u.created_at > clock_timestamp() - interval '7 days'), 0)::int8 AS "tokens_week!",
                COALESCE((SELECT sum(u.cost_usd) FROM usage_events u WHERE u.agent_id=g.id AND u.created_at > clock_timestamp() - interval '7 days'), 0) AS "cost_week_usd!",
                COALESCE((SELECT sum(u.cost_usd) FROM usage_events u WHERE u.agent_id=g.id AND u.created_at >= date_trunc('month', clock_timestamp(), 'UTC')), 0) AS "cost_month_usd!",
                g.monthly_budget_usd,
                (SELECT to_jsonb(max(t.created_at)) #>> '{}' FROM chat_turns t WHERE t.conversation_id=g.id) AS last_turn_at
            FROM agents g
            WHERE g.id=ANY($1)
            ORDER BY g.name
            "#,
            &scope,
        )
        .fetch_all(&mut *tx)
        .await?;
        let work = sqlx::query_as!(
            OpenWork,
            r#"
            SELECT t.id AS turn, t.conversation_id AS agent, CASE WHEN r.task THEN NULL ELSE m.room_id END AS "room?", t.status, t.error,
                CASE WHEN r.task THEN 'On a task' ELSE left(COALESCE(m.message, t.message), 160) END AS "message!",
                (to_jsonb(t.status_at) #>> '{}') AS "since!"
            FROM chat_turns t
            LEFT JOIN room_messages m ON m.id=t.room_message_id
            LEFT JOIN rooms r ON r.id=m.room_id
            WHERE t.conversation_id=ANY($1) AND t.status IN ('queued','running','delegated','waiting')
            ORDER BY t.status_at
            LIMIT 50
            "#,
            &scope,
        )
        .fetch_all(&mut *tx)
        .await?;
        let recent = sqlx::query_as!(
            RecentEvent,
            r#"
            SELECT kind AS "kind!", agent AS "agent!", turn, room, text AS "text!", (to_jsonb(at) #>> '{}') AS "at!"
            FROM (
                (SELECT 'turn_' || t.status AS kind, t.conversation_id AS agent, t.id AS turn,
                    CASE WHEN r.task THEN NULL ELSE m.room_id END AS room,
                    CASE WHEN r.task THEN 'On a task' ELSE left(COALESCE(m.message, t.message), 160) END AS text, t.status_at AS at
                FROM chat_turns t
                LEFT JOIN room_messages m ON m.id=t.room_message_id
                LEFT JOIN rooms r ON r.id=m.room_id
                WHERE t.conversation_id=ANY($1) AND t.status IN ('complete','failed','cancelled')
                ORDER BY t.status_at DESC
                LIMIT 12)
                UNION ALL
                (SELECT 'dream', d.agent_id, NULL, NULL, left(d.report, 160), d.ended_at
                FROM dream_runs d
                WHERE d.agent_id=ANY($1) AND d.outcome IN ('completed','failed')
                ORDER BY d.ended_at DESC
                LIMIT 6)
                UNION ALL
                (SELECT 'skill', COALESCE(d.agent_id, t.conversation_id), r.turn_id, NULL, s.name, r.created_at
                FROM skill_revisions r
                JOIN skills s ON s.id=r.skill_id
                LEFT JOIN dream_runs d ON d.id=r.dream_run_id
                LEFT JOIN chat_turns t ON t.id=r.turn_id
                WHERE COALESCE(d.agent_id, t.conversation_id)=ANY($1)
                ORDER BY r.created_at DESC
                LIMIT 6)
            ) e
            ORDER BY at DESC
            LIMIT 14
            "#,
            &scope,
        )
        .fetch_all(&mut *tx)
        .await?;
        let platform = sqlx::query!(
            r#"
            SELECT (SELECT provider FROM provider_connection LIMIT 1) AS provider,
                (SELECT model FROM provider_connection LIMIT 1) AS model,
                (SELECT to_jsonb(max(fetched_at)) #>> '{}' FROM model_prices) AS prices_at,
                COALESCE((SELECT sum(size) FROM blobs), 0)::int8 AS "stored!",
                (SELECT count(*) FROM human_accounts WHERE active) AS "people!",
                (SELECT count(*) FROM rooms WHERE NOT task) AS "rooms!"
            "#
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Situation {
            today: window.today,
            week: window.week,
            month: window.month,
            days,
            models,
            agents,
            work,
            recent,
            provider: platform.provider.filter(|_| administrator),
            model: platform.model.filter(|_| administrator),
            prices_at: platform.prices_at,
            stored_bytes: if administrator { platform.stored } else { 0 },
            people: if administrator { platform.people } else { 0 },
            rooms: if administrator { platform.rooms } else { 0 },
        })
    }
}

use super::{Work, access};
use crate::{
    agents::AgentId,
    identity::{
        Actor, IdentityError,
        audit::{Event, record},
    },
};
use cubebot_protocol::runtime::ProviderKind;
use cubebot_providers::{Anthropic, Error, Gemini, OpenAi, OpenRouter, Price};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use std::time::Duration;
use tokio::sync::watch;
use uuid::Uuid;

/// Tokens and estimated cost over a period. `unpriced` responses are in the tokens, not the cost.
#[derive(Serialize, utoipa::ToSchema)]
pub struct Spend {
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// Estimated from the model's listed price, or as the provider reported it.
    pub cost_usd: f64,
    pub unpriced: i64,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct DaySpend {
    /// `YYYY-MM-DD`, UTC.
    pub day: String,
    pub tokens: i64,
    pub cost_usd: f64,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ModelSpend {
    pub provider: String,
    pub model: String,
    pub tokens: i64,
    pub cost_usd: f64,
}

/// One agent's usage: its windows, this month against its budget, the last 30 days by day.
#[derive(Serialize, utoipa::ToSchema)]
pub struct AgentUsage {
    pub today: Spend,
    pub week: Spend,
    pub month: Spend,
    pub monthly_budget_usd: Option<f64>,
    pub days: Vec<DaySpend>,
    pub models: Vec<ModelSpend>,
    /// Turns paused because this agent's budget ran out.
    pub paused_turns: i64,
    /// The share of the agent's responses that failed or waited for a person, last 30 days.
    pub turns: i64,
    pub failed_turns: i64,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetBudget {
    /// US dollars per calendar month in UTC; `null` removes the limit.
    pub monthly_budget_usd: Option<f64>,
}

pub(super) struct Window {
    pub today: Spend,
    pub week: Spend,
    pub month: Spend,
}

impl Work {
    #[tracing::instrument(skip_all, fields(actor = %actor.id, agent = %agent.0))]
    pub async fn agent_usage(
        &self,
        actor: &Actor,
        agent: AgentId,
    ) -> Result<AgentUsage, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let canonical = access::conversation(&mut tx, actor, agent.0, "conversation.read").await?;
        if canonical != agent {
            return Err(IdentityError::NotFound);
        }
        let usage = read(&mut tx, agent).await?;
        tx.commit().await?;
        Ok(usage)
    }

    /// Turns whose tool effects are unknown stay waiting: only a person can say they are safe to rerun.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, agent = %agent.0))]
    pub async fn set_budget(
        &self,
        actor: &Actor,
        agent: AgentId,
        input: &SetBudget,
    ) -> Result<AgentUsage, IdentityError> {
        if input
            .monthly_budget_usd
            .is_some_and(|budget| !budget.is_finite() || budget <= 0.0 || budget > 1_000_000.0)
        {
            return Err(IdentityError::InvalidInput);
        }
        let mut tx = self.pool.begin().await?;
        access::lock_actor(&mut tx, actor).await?;
        authorize_manage(&mut tx, actor, agent).await?;
        sqlx::query!(
            "
            UPDATE agents
            SET monthly_budget_usd=$2
            WHERE id=$1
            ",
            agent.0,
            input.monthly_budget_usd,
        )
        .execute(&mut *tx)
        .await?;
        let event = Event::by(actor.id, "agent.budget_set")
            .on(agent.0)
            .detail(serde_json::json!({ "monthly_budget_usd": input.monthly_budget_usd }));
        record(&mut *tx, &event).await?;
        if !exhausted(&mut tx, agent).await? {
            requeue_paused(&mut tx, agent).await?;
        }
        let usage = read(&mut tx, agent).await?;
        tx.commit().await?;
        Ok(usage)
    }
}

async fn authorize_manage(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    agent: AgentId,
) -> Result<(), IdentityError> {
    let allowed = sqlx::query_scalar!(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM agents g
            JOIN human_accounts a ON a.id=$1 AND a.active
            WHERE g.id=$2
                AND (a.role='administrator' OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id=a.id AND p.scope_kind='agent' AND p.scope_id=g.id AND p.capability='agent.manage'))
        ) AS "allowed!"
        "#,
        actor.id,
        agent.0,
    )
    .fetch_one(&mut **tx)
    .await?;
    allowed.then_some(()).ok_or(IdentityError::NotFound)
}

async fn read(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
) -> Result<AgentUsage, IdentityError> {
    let scope = [agent.0];
    let window = windows(tx, &scope, false).await?;
    let days = days(tx, &scope, false).await?;
    let models = models(tx, &scope, false).await?;
    let row = sqlx::query!(
        r#"
        SELECT g.monthly_budget_usd,
            (SELECT count(*) FROM chat_turns t WHERE t.conversation_id=g.id AND t.status='waiting' AND t.error='budget_exhausted') AS "paused!",
            (SELECT count(*) FROM chat_turns t WHERE t.conversation_id=g.id AND t.created_at > clock_timestamp() - interval '30 days') AS "turns!",
            (SELECT count(*) FROM chat_turns t WHERE t.conversation_id=g.id AND t.created_at > clock_timestamp() - interval '30 days' AND (t.status='failed' OR (t.status='waiting' AND t.error<>'budget_exhausted'))) AS "failed!"
        FROM agents g
        WHERE g.id=$1
        "#,
        agent.0,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(AgentUsage {
        today: window.today,
        week: window.week,
        month: window.month,
        monthly_budget_usd: row.monthly_budget_usd,
        days,
        models,
        paused_turns: row.paused,
        turns: row.turns,
        failed_turns: row.failed,
    })
}

pub(super) async fn days(
    tx: &mut Transaction<'_, Postgres>,
    agents: &[Uuid],
    coordinator: bool,
) -> Result<Vec<DaySpend>, sqlx::Error> {
    sqlx::query_as!(
        DaySpend,
        r#"
        SELECT to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD') AS "day!",
            COALESCE(sum(input_tokens + output_tokens), 0)::int8 AS "tokens!",
            COALESCE(sum(cost_usd), 0) AS "cost_usd!"
        FROM usage_events
        WHERE (agent_id=ANY($1) OR ($2 AND agent_id IS NULL))
            AND created_at > clock_timestamp() - interval '30 days'
        GROUP BY 1
        ORDER BY 1
        "#,
        agents,
        coordinator,
    )
    .fetch_all(&mut **tx)
    .await
}

pub(super) async fn models(
    tx: &mut Transaction<'_, Postgres>,
    agents: &[Uuid],
    coordinator: bool,
) -> Result<Vec<ModelSpend>, sqlx::Error> {
    sqlx::query_as!(
        ModelSpend,
        r#"
        SELECT provider, model,
            COALESCE(sum(input_tokens + output_tokens), 0)::int8 AS "tokens!",
            COALESCE(sum(cost_usd), 0) AS "cost_usd!"
        FROM usage_events
        WHERE (agent_id=ANY($1) OR ($2 AND agent_id IS NULL))
            AND created_at > clock_timestamp() - interval '30 days'
        GROUP BY provider, model
        ORDER BY 3 DESC
        LIMIT 8
        "#,
        agents,
        coordinator,
    )
    .fetch_all(&mut **tx)
    .await
}

pub(super) async fn windows(
    tx: &mut Transaction<'_, Postgres>,
    agents: &[Uuid],
    coordinator: bool,
) -> Result<Window, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT
            COALESCE(sum(input_tokens) FILTER (WHERE created_at >= date_trunc('day', clock_timestamp(), 'UTC')), 0)::int8 AS "day_input!",
            COALESCE(sum(output_tokens) FILTER (WHERE created_at >= date_trunc('day', clock_timestamp(), 'UTC')), 0)::int8 AS "day_output!",
            COALESCE(sum(cost_usd) FILTER (WHERE created_at >= date_trunc('day', clock_timestamp(), 'UTC')), 0) AS "day_cost!",
            count(*) FILTER (WHERE cost_usd IS NULL AND created_at >= date_trunc('day', clock_timestamp(), 'UTC')) AS "day_unpriced!",
            COALESCE(sum(input_tokens) FILTER (WHERE created_at > clock_timestamp() - interval '7 days'), 0)::int8 AS "week_input!",
            COALESCE(sum(output_tokens) FILTER (WHERE created_at > clock_timestamp() - interval '7 days'), 0)::int8 AS "week_output!",
            COALESCE(sum(cost_usd) FILTER (WHERE created_at > clock_timestamp() - interval '7 days'), 0) AS "week_cost!",
            count(*) FILTER (WHERE cost_usd IS NULL AND created_at > clock_timestamp() - interval '7 days') AS "week_unpriced!",
            COALESCE(sum(input_tokens) FILTER (WHERE created_at >= date_trunc('month', clock_timestamp(), 'UTC')), 0)::int8 AS "month_input!",
            COALESCE(sum(output_tokens) FILTER (WHERE created_at >= date_trunc('month', clock_timestamp(), 'UTC')), 0)::int8 AS "month_output!",
            COALESCE(sum(cost_usd) FILTER (WHERE created_at >= date_trunc('month', clock_timestamp(), 'UTC')), 0) AS "month_cost!",
            count(*) FILTER (WHERE cost_usd IS NULL AND created_at >= date_trunc('month', clock_timestamp(), 'UTC')) AS "month_unpriced!"
        FROM usage_events
        WHERE (agent_id=ANY($1) OR ($2 AND agent_id IS NULL))
            AND created_at >= LEAST(date_trunc('month', clock_timestamp(), 'UTC'), clock_timestamp() - interval '7 days')
        "#,
        agents,
        coordinator,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(Window {
        today: Spend {
            input_tokens: row.day_input,
            output_tokens: row.day_output,
            cost_usd: row.day_cost,
            unpriced: row.day_unpriced,
        },
        week: Spend {
            input_tokens: row.week_input,
            output_tokens: row.week_output,
            cost_usd: row.week_cost,
            unpriced: row.week_unpriced,
        },
        month: Spend {
            input_tokens: row.month_input,
            output_tokens: row.month_output,
            cost_usd: row.month_cost,
            unpriced: row.month_unpriced,
        },
    })
}

pub(super) async fn resume_paused(tx: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    let paused = sqlx::query_scalar!(
        "
        SELECT DISTINCT conversation_id
        FROM chat_turns
        WHERE status='waiting' AND error='budget_exhausted'
        ",
    )
    .fetch_all(&mut **tx)
    .await?;
    for agent in paused {
        if !exhausted(tx, AgentId(agent)).await? {
            requeue_paused(tx, AgentId(agent)).await?;
        }
    }
    Ok(())
}

async fn requeue_paused(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "
        UPDATE chat_turns t
        SET status='queued', error=NULL
        WHERE t.conversation_id=$1 AND t.status='waiting' AND t.error='budget_exhausted'
            AND NOT EXISTS(SELECT 1 FROM turn_events c WHERE c.turn_id=t.id AND c.payload->>'kind'='tool_call' AND NOT EXISTS(SELECT 1 FROM turn_events r WHERE r.turn_id=c.turn_id AND r.payload->>'kind'='tool_result' AND r.payload->>'id'=c.payload->>'id' AND r.payload->'outcome'->>'status'<>'uncertain'))
        ",
        agent.0,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(crate) async fn exhausted(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"
        SELECT COALESCE(g.monthly_budget_usd <= (
            SELECT COALESCE(sum(u.cost_usd), 0)
            FROM usage_events u
            WHERE u.agent_id=g.id AND u.created_at >= date_trunc('month', clock_timestamp(), 'UTC')
        ), false) AS "exhausted!"
        FROM agents g
        WHERE g.id=$1
        "#,
        agent.0,
    )
    .fetch_one(&mut **tx)
    .await
}

pub(super) async fn record_turn(
    tx: &mut Transaction<'_, Postgres>,
    turn: Uuid,
    sequence: i64,
    (input, cached, output): (i64, i64, i64),
    reported: Option<f64>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "
        INSERT INTO usage_events (agent_id, turn_id, event_sequence, kind, provider, model, input_tokens, cached_input_tokens, output_tokens, cost_usd)
        SELECT t.conversation_id, t.id, $2, 'turn', COALESCE(t.provider, 'unknown'), COALESCE(t.model, 'unknown'), $3, $4, $5,
            COALESCE($6, usage_cost(COALESCE(t.provider, 'unknown'), COALESCE(t.model, 'unknown'), $3, $4, $5))
        FROM chat_turns t
        WHERE t.id=$1
        ON CONFLICT (turn_id, event_sequence) DO NOTHING
        ",
        turn,
        sequence,
        input,
        cached,
        output,
        reported,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(crate) async fn record_duty<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    agent: Option<AgentId>,
    dream: Option<Uuid>,
    (provider, model): (&str, &str),
    usage: &cubebot_protocol::runtime::TokenUsage,
) -> Result<(), sqlx::Error> {
    let (input, cached, output) = (
        usage.input_tokens as i64,
        usage.cached_input_tokens as i64,
        usage.output_tokens as i64,
    );
    sqlx::query!(
        "
        INSERT INTO usage_events (agent_id, dream_run_id, kind, provider, model, input_tokens, cached_input_tokens, output_tokens, cost_usd)
        VALUES ($1, $2, CASE WHEN $2::uuid IS NULL THEN 'coordinator' ELSE 'dream' END, $3, $4, $5, $6, $7,
            COALESCE($8, usage_cost($3, $4, $5, $6, $7)))
        ON CONFLICT (dream_run_id) DO NOTHING
        ",
        agent.map(|agent| agent.0),
        dream,
        provider,
        model,
        input,
        cached,
        output,
        usage.cost_usd,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn keep_prices(pool: PgPool, mut stop: watch::Receiver<bool>) {
    loop {
        if let Err(error) = refresh(&pool).await {
            tracing::warn!(?error, "model prices not refreshed");
        }
        tokio::select! {
            _ = stop.changed() => return,
            () = tokio::time::sleep(Duration::from_secs(60 * 60)) => (),
        }
    }
}

async fn refresh(pool: &PgPool) -> Result<(), sqlx::Error> {
    for provider in prices_due(pool).await? {
        match listed(&provider).await {
            Ok(prices) => store_prices(pool, &provider, &prices).await?,
            Err(error) => tracing::warn!(?error, provider, "provider prices unavailable"),
        }
    }
    Ok(())
}

pub async fn prices_due(pool: &PgPool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"
        SELECT p.provider AS "provider!"
        FROM (SELECT provider FROM provider_connection UNION SELECT provider FROM provider_saved) p
        WHERE NOT EXISTS(
            SELECT 1 FROM model_prices m
            WHERE m.provider=p.provider AND m.fetched_at > clock_timestamp() - interval '1 day'
        )
        "#
    )
    .fetch_all(pool)
    .await
}

async fn listed(provider: &str) -> Result<Vec<Price>, Error> {
    match ProviderKind::parse(provider) {
        Some(ProviderKind::Openai) => OpenAi::new()?.prices().await,
        Some(ProviderKind::Anthropic) => Anthropic::new()?.prices().await,
        Some(ProviderKind::Openrouter) => OpenRouter::new()?.prices().await,
        Some(ProviderKind::Gemini) => Gemini::new()?.prices().await,
        Some(ProviderKind::Minimax) => Anthropic::minimax()?.prices().await,
        // A plan has no per-token price, and Grok reports its own cost.
        // shortcut: mainland MiniMax bills in yuan, so its turns stay unpriced; convert if anyone needs those estimates.
        Some(
            ProviderKind::Grok
            | ProviderKind::Chatgpt
            | ProviderKind::Supergrok
            | ProviderKind::MinimaxCn
            | ProviderKind::MinimaxPlan
            | ProviderKind::MinimaxPlanCn,
        ) => Ok(Vec::new()),
        None => Err(Error::Unavailable),
    }
}

/// shortcut: dated model versions share their family's price; store version prices if a provider
/// starts charging them differently.
pub async fn store_prices(
    pool: &PgPool,
    provider: &str,
    prices: &[Price],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for price in prices {
        sqlx::query!(
            "
            INSERT INTO model_prices (provider, key, input_per_token, cached_per_token, output_per_token)
            VALUES ($1, price_key($2), $3, $4, $5)
            ON CONFLICT (provider, key) DO UPDATE
            SET input_per_token=EXCLUDED.input_per_token, cached_per_token=EXCLUDED.cached_per_token,
                output_per_token=EXCLUDED.output_per_token, fetched_at=clock_timestamp()
            ",
            provider,
            price.model,
            price.input_per_token,
            price.cached_per_token,
            price.output_per_token,
        )
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query!(
        "
        UPDATE usage_events
        SET cost_usd=usage_cost(provider, model, input_tokens, cached_input_tokens, output_tokens)
        WHERE cost_usd IS NULL AND provider=$1
        ",
        provider,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

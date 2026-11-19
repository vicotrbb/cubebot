use super::Work;
use crate::{
    agents::{AgentId, runtime::RuntimeActor},
    connections::{host, same_site},
    identity::{
        IdentityError,
        audit::{Event, record},
    },
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

const APPROVAL_MINUTES: i32 = 15;

#[derive(Debug, PartialEq, Eq)]
pub enum CardGate {
    Approved,
    Asked,
}

pub struct CardAsk<'a> {
    pub item: Uuid,
    pub name: &'a str,
    pub turn: Uuid,
    pub call: Uuid,
    pub page: &'a str,
    pub amount: &'a str,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclineCardFill {
    pub reason: String,
}

impl Work {
    #[tracing::instrument(skip_all, fields(agent = %actor.agent.0, item = %ask.item, call = %ask.call))]
    pub async fn card_gate(
        &self,
        actor: &RuntimeActor,
        ask: &CardAsk<'_>,
    ) -> Result<Result<CardGate, String>, IdentityError> {
        let domain = host(ask.page);
        let mut tx = self.pool.begin().await?;
        sqlx::query!(
            "SELECT id FROM agents WHERE id=$1 FOR NO KEY UPDATE",
            actor.agent.0
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        let approved = sqlx::query_scalar!(
            "
            SELECT domain
            FROM card_fills
            WHERE agent_id=$1 AND item_id=$2 AND status='approved' AND approved_until > clock_timestamp()
            ",
            actor.agent.0,
            ask.item,
        )
        .fetch_all(&mut *tx)
        .await?;
        if approved.iter().any(|granted| same_site(granted, &domain)) {
            return Ok(Ok(CardGate::Approved));
        }
        let previous =
            sqlx::query_scalar!("SELECT agent_id FROM card_fills WHERE call_id=$1", ask.call,)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(agent) = previous {
            return Ok((agent == actor.agent.0)
                .then_some(CardGate::Asked)
                .ok_or_else(|| "this call id was already used".into()));
        }
        let running = sqlx::query_scalar!(
            "
            SELECT id
            FROM chat_turns
            WHERE id=$1 AND conversation_id=$2 AND status='running' AND attempt_id=$3
            ",
            ask.turn,
            actor.agent.0,
            actor.attempt.0,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if running.is_none() {
            return Ok(Err("this turn is no longer running".into()));
        }
        let asked = sqlx::query_scalar!(
            "
            INSERT INTO card_fills (agent_id, item_id, turn_id, call_id, card_name, domain, amount)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (agent_id, item_id) WHERE status='pending' DO NOTHING
            RETURNING id
            ",
            actor.agent.0,
            ask.item,
            ask.turn,
            ask.call,
            ask.name,
            domain,
            ask.amount,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(id) = asked else {
            return Ok(Err(format!(
                "a payment with the card {} already waits for a person to approve it",
                ask.name
            )));
        };
        let detail = json!({"agent": actor.agent.0, "name": ask.name, "domain": domain, "amount": ask.amount, "request": id});
        record(
            &mut *tx,
            &Event::by_agent(actor.agent.0, "vault.card_request")
                .on(ask.item)
                .detail(detail),
        )
        .await?;
        tx.commit().await?;
        Ok(Ok(CardGate::Asked))
    }

    pub async fn card_fill_item(
        &self,
        agent: AgentId,
        request: Uuid,
    ) -> Result<Uuid, IdentityError> {
        sqlx::query_scalar!(
            "SELECT item_id FROM card_fills WHERE id=$1 AND agent_id=$2",
            request,
            agent.0,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::NotFound)
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0, request = %request))]
    pub async fn settle_card_fill(
        &self,
        person: Uuid,
        agent: AgentId,
        request: Uuid,
        declined: Option<&str>,
    ) -> Result<(), IdentityError> {
        if declined.is_some_and(|reason| reason.len() > 2048) {
            return Err(IdentityError::InvalidInput);
        }
        let mut tx = self.pool.begin().await?;
        let settled = sqlx::query!(
            "
            UPDATE card_fills f
            SET status=CASE WHEN $4::text IS NULL THEN 'approved' ELSE 'declined' END,
                settled_by=$3, settled_at=clock_timestamp(), reason=$4,
                approved_until=CASE WHEN $4::text IS NULL THEN clock_timestamp() + make_interval(mins => $5) END
            FROM chat_turns t, vault_items v
            WHERE f.id=$1 AND f.agent_id=$2 AND f.status='pending' AND t.id=f.turn_id AND v.id=f.item_id
            RETURNING f.item_id, f.card_name, f.domain, f.amount, t.actor_id, v.card_last4
            ",
            request,
            agent.0,
            person,
            declined,
            APPROVAL_MINUTES,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        let card = match settled.card_last4 {
            Some(last4) => format!("{} ending {last4}", settled.card_name),
            None => settled.card_name.clone(),
        };
        let (kind, message) = match declined {
            None => (
                "vault.card_approve",
                format!(
                    "A person approved paying {} with the card {card} on {}. Fill it now with vault_fill, one field per call: click each field first, then take a screenshot to check. The approval covers this card on {} for {APPROVAL_MINUTES} minutes. If the bank asks for a 3-D Secure check, call request_login for {} so a person completes it.",
                    settled.amount, settled.domain, settled.domain, settled.domain
                ),
            ),
            Some(reason) => (
                "vault.card_decline",
                format!(
                    "A person declined paying {} with the card {card} on {}: {reason}. Do not try this payment again unless they ask.",
                    settled.amount, settled.domain
                ),
            ),
        };
        let detail = json!({"agent": agent.0, "name": settled.card_name, "domain": settled.domain, "amount": settled.amount, "request": request, "reason": declined});
        record(
            &mut *tx,
            &Event::by(person, kind).on(settled.item_id).detail(detail),
        )
        .await?;
        super::wake(&mut tx, agent).await?;
        sqlx::query!(
            "
            INSERT INTO chat_turns (conversation_id, actor_id, request_id, message)
            VALUES ($1, $2, gen_random_uuid(), $3)
            ",
            agent.0,
            settled.actor_id,
            message,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

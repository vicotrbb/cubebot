//! Login handoffs (ADR 0039). An agent's running turn asks a person to log in on a site through
//! its desktop and ends; a person takes control for it. Handing control back completes it and
//! declining refuses it, and either queues a message telling the agent. The message is sent as
//! the person whose turn asked, since delivery needs `conversation.send`, which the person who
//! settled it may not hold; `settled_by` records who that was.
use super::Work;
use crate::{
    agents::{AgentId, kubernetes::Error, runtime::RuntimeActor},
    identity::IdentityError,
};
use cubebot_protocol::runtime::RequestLogin;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, utoipa::ToSchema)]
pub struct LoginHandoff {
    pub id: Uuid,
    pub site: String,
    /// The agent's words: untrusted text to show, never markup.
    pub reason: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclineLogin {
    /// Returned to the agent; empty when the person gave none.
    pub reason: String,
}

impl Work {
    /// Records the handoff for one tool call, or nothing for a resent one. The inner error is
    /// the refusal the model reads.
    #[tracing::instrument(skip_all, fields(agent = %actor.agent.0, turn = %turn, call = %input.call))]
    pub(crate) async fn request_login(
        &self,
        actor: &RuntimeActor,
        turn: Uuid,
        input: &RequestLogin,
    ) -> Result<Result<(), String>, Error> {
        if !valid(input) {
            return Err(Error::InvalidConfiguration);
        }
        let mut tx = self.pool.begin().await?;
        // Serializes the agent's requests, so at most one is pending.
        sqlx::query!(
            "
            SELECT id
            FROM agents
            WHERE id=$1
            FOR UPDATE
            ",
            actor.agent.0,
        )
        .fetch_all(&mut *tx)
        .await?;
        let previous = sqlx::query_scalar!(
            "
            SELECT agent_id
            FROM login_handoffs
            WHERE call_id=$1
            ",
            input.call,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(agent) = previous {
            return Ok((agent == actor.agent.0)
                .then_some(())
                .ok_or_else(|| "this call id was already used".into()));
        }
        let running = sqlx::query_scalar!(
            "
            SELECT id
            FROM chat_turns
            WHERE id=$1 AND conversation_id=$2 AND status='running' AND attempt_id=$3
            ",
            turn,
            actor.agent.0,
            actor.attempt.0,
        )
        .fetch_optional(&mut *tx)
        .await?;
        running.ok_or(Error::Conflict)?;
        let pending = sqlx::query_scalar!(
            "
            SELECT site
            FROM login_handoffs
            WHERE agent_id=$1 AND status='pending'
            ",
            actor.agent.0,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(site) = pending {
            return Ok(Err(format!(
                "a login on {site} is already waiting for a person"
            )));
        }
        sqlx::query!(
            "
            INSERT INTO login_handoffs (agent_id, turn_id, call_id, site, reason)
            VALUES ($1, $2, $3, $4, $5)
            ",
            actor.agent.0,
            turn,
            input.call,
            input.site,
            input.reason,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Ok(()))
    }

    /// The caller has authorized viewing the agent's desktop.
    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub async fn pending_login(
        &self,
        agent: AgentId,
    ) -> Result<Option<LoginHandoff>, IdentityError> {
        Ok(sqlx::query_as!(
            LoginHandoff,
            "
            SELECT id, site, reason
            FROM login_handoffs
            WHERE agent_id=$1 AND status='pending'
            ",
            agent.0,
        )
        .fetch_optional(&self.pool)
        .await?)
    }

    /// The caller has given `person` control of the agent's desktop.
    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub async fn take_login(&self, agent: AgentId, person: Uuid) -> Result<(), IdentityError> {
        sqlx::query!(
            "
            UPDATE login_handoffs
            SET taken_by=$2, taken_at=clock_timestamp()
            WHERE agent_id=$1 AND status='pending'
            ",
            agent.0,
            person,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Completes the pending handoff, or declines it with `declined`, and queues the message
    /// that tells the agent; false when none was pending. The caller has authorized `person`
    /// to control the agent's desktop.
    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub async fn settle_login(
        &self,
        agent: AgentId,
        person: Uuid,
        declined: Option<&str>,
    ) -> Result<bool, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let settled = sqlx::query!(
            "
            UPDATE login_handoffs h
            SET status=CASE WHEN $3::text IS NULL THEN 'completed' ELSE 'declined' END,
                settled_by=$2, settled_at=clock_timestamp(), declined_because=$3
            FROM chat_turns t
            WHERE h.agent_id=$1 AND h.status='pending' AND t.id=h.turn_id
            RETURNING h.site, t.actor_id
            ",
            agent.0,
            person,
            declined,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(settled) = settled else {
            return Ok(false);
        };
        let site = settled.site;
        let message = match declined {
            None => format!("Login completed on {site}, take a screenshot."),
            Some(reason) => format!("A person declined to log in on {site}: {reason}"),
        };
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
        Ok(true)
    }
}

fn valid(input: &RequestLogin) -> bool {
    let text = |value: &str, limit: usize| {
        !value.trim().is_empty() && value.len() <= limit && !value.contains('\0')
    };
    text(&input.site, 255) && text(&input.reason, 2048)
}

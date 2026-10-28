//! One active turn per agent. The ledger delivers conversation records; the agent assembles
//! its own context.
use super::Work;
use crate::{
    agents::{kubernetes::Error, runtime::RuntimeActor},
    connections::Connections,
};
use cubebot_protocol::runtime::{EVENT_BYTES, Exchange, Turn, TurnEvent};
use uuid::Uuid;

impl Work {
    /// Caller authenticates machine identity and holds the agent/resource transaction locks.
    pub(crate) async fn claim(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        actor: &RuntimeActor,
        connections: Option<&Connections>,
    ) -> Result<Option<Turn>, Error> {
        // Only this attempt's sandbox can still be running the agent's turn.
        sqlx::query("UPDATE chat_turns SET status='waiting',error='execution_interrupted' WHERE conversation_id=$1 AND status='running' AND attempt_id IS DISTINCT FROM $2")
            .bind(actor.agent).bind(actor.attempt).execute(&mut **tx).await?;
        let busy: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM chat_turns WHERE conversation_id=$1 AND status IN ('running','waiting'))")
            .bind(actor.agent).fetch_one(&mut **tx).await?;
        if busy {
            return Ok(None);
        }
        let pending: Option<(Uuid,String)> = sqlx::query_as("SELECT id,message FROM chat_turns WHERE conversation_id=$1 AND status='queued' ORDER BY sequence LIMIT 1 FOR UPDATE")
            .bind(actor.agent).fetch_optional(&mut **tx).await?;
        let Some((turn, message)) = pending else {
            return Ok(None);
        };
        if !sender_allowed(tx, turn).await? {
            wait(tx, turn, "sender_access_revoked").await?;
            return Ok(None);
        }
        let Some(inference) = provider(tx, connections, actor).await else {
            wait(tx, turn, "provider_unavailable").await?;
            return Ok(None);
        };
        sqlx::query("UPDATE chat_turns SET status='running',attempt_id=$2,error=NULL WHERE id=$1")
            .bind(turn)
            .bind(actor.attempt)
            .execute(&mut **tx)
            .await?;
        let instructions: String =
            sqlx::query_scalar("SELECT instructions FROM agents WHERE id=$1")
                .bind(actor.agent)
                .fetch_one(&mut **tx)
                .await?;
        let (summary, history) = history(tx, actor).await?;
        let prior: Vec<serde_json::Value> = sqlx::query_scalar("SELECT payload FROM (SELECT sequence,payload FROM turn_events WHERE turn_id=$1 AND payload->>'kind' IN ('tool_call','tool_result') ORDER BY sequence DESC LIMIT 16) p ORDER BY sequence")
            .bind(turn).fetch_all(&mut **tx).await?;
        let last: i64 = sqlx::query_scalar(
            "SELECT COALESCE(max(sequence),0) FROM turn_events WHERE turn_id=$1",
        )
        .bind(turn)
        .fetch_one(&mut **tx)
        .await?;
        Ok(Some(Turn {
            turn,
            sequence: last as u64 + 1,
            message,
            instructions,
            provider: inference.provider,
            model: inference.model,
            api_key: inference.api_key,
            context_window: inference.context_window,
            summary,
            history,
            prior: prior
                .into_iter()
                .map(serde_json::from_value)
                .collect::<Result<_, _>>()
                .map_err(|_| Error::InvalidConfiguration)?,
        }))
    }

    /// Appends an event idempotently. Only the running attempt's done or error settles the turn;
    /// late events remain evidence. Returns whether this attempt still owns the turn's run,
    /// including a cancelled run that has not stopped yet.
    pub(crate) async fn record(
        &self,
        actor: &RuntimeActor,
        turn: Uuid,
        sequence: u64,
        event: &TurnEvent,
    ) -> Result<bool, Error> {
        let payload = payload(sequence, event)?;
        let mut tx = self.pool.begin().await?;
        let (status, attempt): (String, Option<Uuid>) = sqlx::query_as(
            "SELECT status,attempt_id FROM chat_turns WHERE id=$1 AND conversation_id=$2 FOR UPDATE",
        )
        .bind(turn)
        .bind(actor.agent)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::Conflict)?;
        // A resent event keeps its first recorded form.
        sqlx::query("INSERT INTO turn_events (turn_id,sequence,payload) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING")
            .bind(turn).bind(sequence as i64).bind(&payload).execute(&mut *tx).await?;
        let owned = attempt == Some(actor.attempt.0);
        if status == "running" && owned {
            settle(&mut tx, turn, event).await?;
        }
        tx.commit().await?;
        Ok(owned && matches!(status.as_str(), "running" | "cancelled"))
    }

    pub(crate) async fn cancelled(&self, turn: Uuid) -> Result<bool, Error> {
        Ok(
            sqlx::query_scalar("SELECT status='cancelled' FROM chat_turns WHERE id=$1")
                .bind(turn)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// Owner-driven recovery does not depend on a lost sandbox returning to report.
    pub async fn recover_interrupted(&self) -> Result<(), Error> {
        sqlx::query("UPDATE chat_turns t SET status='waiting',error='execution_interrupted' WHERE status='running' AND NOT EXISTS(SELECT 1 FROM runtime_session_access s WHERE s.attempt_id=t.attempt_id AND s.recovery)")
            .execute(&self.pool).await?;
        Ok(())
    }
}

fn payload(sequence: u64, event: &TurnEvent) -> Result<serde_json::Value, Error> {
    let payload = serde_json::to_value(event).map_err(|_| Error::InvalidConfiguration)?;
    if sequence == 0
        || sequence > i64::MAX as u64
        || payload.to_string().len() > EVENT_BYTES
        || !valid(event)
    {
        return Err(Error::InvalidConfiguration);
    }
    Ok(payload)
}

async fn settle(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    turn: Uuid,
    event: &TurnEvent,
) -> Result<(), Error> {
    match event {
        TurnEvent::Done { response } => {
            sqlx::query("UPDATE chat_turns SET status='complete',response=$2,error=NULL,completed_at=clock_timestamp() WHERE id=$1")
                .bind(turn).bind(response).execute(&mut **tx).await?;
        }
        TurnEvent::Error { code } => wait(tx, turn, code).await?,
        _ => (),
    }
    Ok(())
}

fn valid(event: &TurnEvent) -> bool {
    // Length is bounded by the encoded event as a whole.
    let text = |value: &str| !value.contains('\0');
    match event {
        TurnEvent::TextDelta { text: delta } => text(delta),
        TurnEvent::Done { response } => !response.is_empty() && text(response),
        TurnEvent::Summary { text: summary } => !summary.is_empty() && text(summary),
        TurnEvent::Error { code } => matches!(
            code.as_str(),
            "provider_unavailable"
                | "provider_rejected"
                | "provider_invalid_response"
                | "journal_unavailable"
                | "cancelled"
        ),
        TurnEvent::ToolResult { outcome, .. } => outcome.validate().is_ok(),
        TurnEvent::ToolCall { name, .. } => !name.is_empty() && name.len() <= 128,
        TurnEvent::Usage { .. } => true,
    }
}

async fn wait(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    turn: Uuid,
    error: &str,
) -> Result<(), Error> {
    sqlx::query("UPDATE chat_turns SET status='waiting',error=$2 WHERE id=$1")
        .bind(turn)
        .bind(error)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn sender_allowed(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    turn: Uuid,
) -> Result<bool, Error> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM chat_turns t JOIN human_accounts a ON a.id=t.actor_id AND a.active WHERE t.id=$1 AND (a.role='administrator' OR EXISTS(SELECT 1 FROM effective_human_grants g WHERE g.actor_id=a.id AND g.scope_kind='conversation' AND g.scope_id=t.conversation_id AND g.capability='conversation.send')))")
        .bind(turn).fetch_one(&mut **tx).await?)
}

/// The latest summary and every completed exchange from the turn it was written in onward,
/// since a summary can precede its own turn's response.
async fn history(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: &RuntimeActor,
) -> Result<(Option<String>, Vec<Exchange>), Error> {
    // shortcut: scans the conversation's summary events per claim; index payload kinds if
    // long conversations make claims slow.
    let summary: Option<(String, i64)> = sqlx::query_as("SELECT e.payload->>'text', t.sequence FROM turn_events e JOIN chat_turns t ON t.id=e.turn_id WHERE t.conversation_id=$1 AND e.payload->>'kind'='summary' ORDER BY t.sequence DESC, e.sequence DESC LIMIT 1")
        .bind(actor.agent).fetch_optional(&mut **tx).await?;
    let from = summary.as_ref().map_or(0, |(_, sequence)| *sequence);
    let history: Vec<(String, String)> = sqlx::query_as("SELECT message,response FROM chat_turns WHERE conversation_id=$1 AND status='complete' AND sequence>=$2 ORDER BY sequence")
        .bind(actor.agent).bind(from).fetch_all(&mut **tx).await?;
    Ok((
        summary.map(|(text, _)| text),
        history
            .into_iter()
            .map(|(message, response)| Exchange { message, response })
            .collect(),
    ))
}

async fn provider(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    connections: Option<&Connections>,
    actor: &RuntimeActor,
) -> Option<crate::connections::Inference> {
    connections?.inference(tx, actor.agent).await.ok()
}

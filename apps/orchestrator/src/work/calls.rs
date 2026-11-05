//! A voice call in the ledger (docs/adr/console.md#voice-calls-over-the-chat-pipeline). Its turns are
//! ordinary chat turns; the conversation shows one `calls` row in their place.
use super::{SendMessage, Turn, TurnRow, Work, access};
use crate::{
    agents::AgentId,
    identity::{Actor, IdentityError},
    speech::{Provider, Route, Voice},
};
use cubebot_protocol::runtime::TurnEvent;
use serde::Serialize;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// What a voice call's message shows.
#[derive(Serialize, utoipa::ToSchema)]
pub struct CallRecord {
    /// Unix seconds.
    pub started_at: i64,
    /// Seconds; `None` while the call is live.
    pub duration_seconds: Option<i64>,
    pub stt_provider: Provider,
    pub tts_provider: Provider,
    pub voice: Voice,
    /// The transcript: each turn's message is what the person said, its response the agent's reply.
    #[schema(no_recursion)]
    pub turns: Vec<Turn>,
}

/// New progress on a turn a call is waiting for.
pub struct Reply {
    pub events: Vec<ReplyEvent>,
    pub status: String,
    pub response: Option<String>,
    pub error: Option<String>,
}

pub enum ReplyEvent {
    Text(String),
    Tool(String),
}

/// A reply is heard, not read.
const SPOKEN: &str = "\n\nYou are on a voice call. What you write is read aloud and what you receive was transcribed from speech, so it may contain mistakes. Answer briefly in plain spoken sentences, without markdown, lists or code. Use your tools as usual, and say what you are doing before long work.";

pub(super) fn spoken(instructions: String, on_call: bool) -> String {
    if on_call {
        instructions + SPOKEN
    } else {
        instructions
    }
}

struct CallRow {
    id: Uuid,
    sequence: i64,
    stt_provider: Provider,
    tts_provider: Provider,
    voice: Voice,
    started_at: i64,
    duration_seconds: Option<i64>,
}

impl Work {
    /// Whether the actor may speak in this agent's conversation, and so call it.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, agent = %agent.0))]
    pub async fn may_call(&self, actor: &Actor, agent: AgentId) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        access::conversation(&mut tx, actor, agent.0, "conversation.send").await?;
        tx.commit().await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, agent = %agent.0))]
    pub async fn start_call(
        &self,
        actor: &Actor,
        agent: AgentId,
        route: Route,
    ) -> Result<Uuid, IdentityError> {
        let mut tx = self.pool.begin().await?;
        access::conversation(&mut tx, actor, agent.0, "conversation.send").await?;
        let call = sqlx::query_scalar!(
            "
            INSERT INTO calls (conversation_id, actor_id, stt_provider, tts_provider, voice)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id
            ",
            agent.0,
            actor.id,
            route.stt.as_str(),
            route.tts.as_str(),
            route.voice.as_str(),
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(call)
    }

    #[tracing::instrument(skip_all, fields(call = %call))]
    pub async fn end_call(&self, call: Uuid) -> Result<(), IdentityError> {
        sqlx::query!(
            "UPDATE calls SET ended_at=clock_timestamp() WHERE id=$1 AND ended_at IS NULL",
            call,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Runs at startup: one orchestrator is active, so a call still open lost its socket when
    /// the last one stopped. It ends at the last thing said.
    #[tracing::instrument(skip_all)]
    pub async fn close_open_calls(&self) -> Result<(), IdentityError> {
        sqlx::query!(
            "
            UPDATE calls c
            SET ended_at=GREATEST(c.started_at, COALESCE(
                (SELECT max(COALESCE(t.completed_at, t.created_at)) FROM chat_turns t WHERE t.call_id=c.id),
                c.started_at))
            WHERE c.ended_at IS NULL
            ",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// A finished sentence, sent as any message is: the same access check, wake and queue.
    pub async fn say_in_call(
        &self,
        actor: &Actor,
        agent: AgentId,
        call: Uuid,
        text: &str,
    ) -> Result<Uuid, IdentityError> {
        let input = SendMessage {
            request_id: Uuid::new_v4(),
            message: text.to_owned(),
        };
        let turn = self.send_in(actor, agent.0, &input, Some(call)).await?;
        Ok(turn.id)
    }

    /// The turn's state and its events after `after`, which moves past them.
    #[tracing::instrument(skip_all, fields(turn = %turn))]
    pub async fn reply(&self, turn: Uuid, after: &mut i64) -> Result<Reply, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query!(
            r#"
            SELECT id, payload AS "payload!"
            FROM turn_events
            WHERE turn_id=$1 AND id>$2 AND payload->>'kind' IN ('text_delta','tool_call')
            ORDER BY id
            "#,
            turn,
            *after,
        )
        .fetch_all(&mut *tx)
        .await?;
        let state = sqlx::query!(
            "SELECT status, response, error FROM chat_turns WHERE id=$1",
            turn,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            *after = row.id;
            match serde_json::from_value::<TurnEvent>(row.payload) {
                Ok(TurnEvent::TextDelta { text }) => events.push(ReplyEvent::Text(text)),
                Ok(TurnEvent::ToolCall { name, .. }) => events.push(ReplyEvent::Tool(name)),
                Ok(_) => {}
                Err(error) => tracing::warn!(?error, "unreadable turn event skipped"),
            }
        }
        Ok(Reply {
            events,
            status: state.status,
            response: state.response,
            error: state.error,
        })
    }

    /// Calls as messages, after a history cursor.
    pub(super) async fn calls_after(
        tx: &mut Transaction<'_, Postgres>,
        conversation: Uuid,
        after: i64,
    ) -> Result<Vec<Turn>, IdentityError> {
        let rows = sqlx::query_as!(
            CallRow,
            r#"
            SELECT id, sequence,
                stt_provider AS "stt_provider: Provider", tts_provider AS "tts_provider: Provider",
                voice AS "voice: Voice",
                extract(epoch FROM started_at)::int8 AS "started_at!",
                extract(epoch FROM ended_at - started_at)::int8 AS duration_seconds
            FROM calls
            WHERE conversation_id=$1 AND sequence>$2
            ORDER BY sequence
            LIMIT 100
            "#,
            conversation,
            after,
        )
        .fetch_all(&mut **tx)
        .await?;
        Self::assemble(tx, rows).await
    }

    /// Calls whose message may have changed: live ones, those with unsettled work, and any the
    /// stream last sent as unfinished.
    pub(super) async fn calls_changing(
        tx: &mut Transaction<'_, Postgres>,
        conversation: Uuid,
        touched: &[Uuid],
    ) -> Result<Vec<Turn>, IdentityError> {
        let rows = sqlx::query_as!(
            CallRow,
            r#"
            SELECT c.id, c.sequence,
                c.stt_provider AS "stt_provider: Provider", c.tts_provider AS "tts_provider: Provider",
                c.voice AS "voice: Voice",
                extract(epoch FROM c.started_at)::int8 AS "started_at!",
                extract(epoch FROM c.ended_at - c.started_at)::int8 AS duration_seconds
            FROM calls c
            WHERE c.conversation_id=$1 AND (
                c.ended_at IS NULL OR c.id=ANY($2) OR EXISTS(
                    SELECT 1 FROM chat_turns t
                    WHERE t.call_id=c.id AND t.status IN ('queued','running','waiting','delegated')
                ))
            ORDER BY c.sequence
            "#,
            conversation,
            touched,
        )
        .fetch_all(&mut **tx)
        .await?;
        Self::assemble(tx, rows).await
    }

    async fn assemble(
        tx: &mut Transaction<'_, Postgres>,
        rows: Vec<CallRow>,
    ) -> Result<Vec<Turn>, IdentityError> {
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let held = sqlx::query!(
            "
            SELECT call_id AS \"call_id!\", id, sequence, message, status, response, error
            FROM chat_turns
            WHERE call_id=ANY($1)
            ORDER BY sequence
            ",
            &ids,
        )
        .fetch_all(&mut **tx)
        .await?;
        let owners: Vec<Uuid> = held.iter().map(|turn| turn.call_id).collect();
        let mut turns: Vec<Turn> = held
            .into_iter()
            .map(|turn| {
                Turn::from(TurnRow {
                    id: turn.id,
                    sequence: turn.sequence,
                    message: turn.message,
                    status: turn.status,
                    response: turn.response,
                    error: turn.error,
                })
            })
            .collect();
        Self::progress(tx, &mut turns).await?;
        let mut held: Vec<(Uuid, Turn)> = owners.into_iter().zip(turns).collect();
        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            let (own, rest) = held.into_iter().partition(|(call, _)| *call == row.id);
            held = rest;
            messages.push(message(
                row,
                own.into_iter().map(|(_, turn)| turn).collect(),
            ));
        }
        Ok(messages)
    }
}

/// One conversation message for a whole call, unfinished while it is live or its work is.
fn message(row: CallRow, turns: Vec<Turn>) -> Turn {
    let settled = turns
        .iter()
        .all(|turn| matches!(turn.status.as_str(), "complete" | "failed" | "cancelled"));
    let status = if row.duration_seconds.is_some() && settled {
        "complete"
    } else {
        "running"
    };
    let mut call = Turn::from(TurnRow {
        id: row.id,
        sequence: row.sequence,
        message: String::new(),
        status: status.to_owned(),
        response: None,
        error: None,
    });
    call.call = Some(CallRecord {
        started_at: row.started_at,
        duration_seconds: row.duration_seconds,
        stt_provider: row.stt_provider,
        tts_provider: row.tts_provider,
        voice: row.voice,
        turns,
    });
    call
}

#[cfg(test)]
mod tests {
    use super::spoken;

    #[test]
    fn only_a_turn_on_a_call_is_told_it_is_one() {
        assert_eq!(spoken("Be brief".to_owned(), false), "Be brief");
        assert!(spoken("Be brief".to_owned(), true).contains("voice call"));
    }
}

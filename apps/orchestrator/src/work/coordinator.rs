//! The tool-free room coordinator (ADR 0014). It takes one unaddressed room message, asks the
//! installation's model to pick one owner, ask a question or do nothing, and validates the answer
//! against current membership before any turn exists. The model call happens outside any
//! transaction; the message stays `routing` until a valid result commits.
use super::rooms;
use crate::{agents::kubernetes::Error, connections::Connections};
use cubebot_protocol::runtime::ProviderKind;
use cubebot_providers::{Anthropic, Event, Message, OpenAi, Provider, Request};
use futures_util::StreamExt;
use serde::Deserialize;
use sqlx::PgPool;
use std::time::Duration;
use tokio::sync::watch;
use uuid::Uuid;

/// Failed coordinator calls before the message is reported as failed.
const ATTEMPTS: i16 = 3;

const SYSTEM: &str = "You route one message in a team chat room to the single agent best placed to own it. You have no tools and do no work yourself. Reply with JSON only, one of: {\"action\":\"assign\",\"agent\":\"<member id>\"}, {\"action\":\"continue\",\"task\":\"<open task id>\"} when the message corrects or adds to that open task, {\"action\":\"clarify\",\"question\":\"<one short question for the sender>\"} or {\"action\":\"none\"} when the message needs no agent, such as thanks or chatter.";

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Decision {
    Assign { agent: Uuid },
    Continue { task: Uuid },
    Clarify { question: String },
    None,
}

struct Pending {
    id: Uuid,
    prompt: String,
}

/// Routes waiting room messages until stopped.
pub async fn coordinate(pool: PgPool, connections: Connections, mut stop: watch::Receiver<bool>) {
    loop {
        if !routed(&pool, &connections).await {
            tokio::select! {
                _ = stop.changed() => return,
                _ = tokio::time::sleep(Duration::from_secs(2)) => (),
            }
        }
    }
}

/// Whether a message was settled, so another may be waiting.
async fn routed(pool: &PgPool, connections: &Connections) -> bool {
    route(pool, connections).await.unwrap_or_else(|error| {
        tracing::warn!(?error, "room coordination failed");
        false
    })
}

/// Settles at most one waiting message; `false` when none was waiting.
pub async fn route(pool: &PgPool, connections: &Connections) -> Result<bool, Error> {
    let mut tx = pool.begin().await?;
    let Some(pending) = pending(&mut tx).await? else {
        return Ok(false);
    };
    let inference = connections.coordinator(&mut tx).await.ok();
    tx.commit().await?;
    let reply = match inference {
        Some(inference) => ask(&inference, &pending.prompt).await,
        None => Err("provider_unavailable"),
    };
    let decision = reply.and_then(|text| decision(&text).ok_or("invalid_coordinator_result"));
    settle(pool, &pending, decision).await?;
    Ok(true)
}

async fn pending(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<Option<Pending>, Error> {
    let row: Option<(Uuid, Uuid, String, String, i64)> = sqlx::query_as("SELECT m.id,m.room_id,r.name,m.message,m.sequence FROM room_messages m JOIN rooms r ON r.id=m.room_id WHERE m.routing='routing' ORDER BY m.sequence LIMIT 1")
        .fetch_optional(&mut **tx).await?;
    let Some((id, room, name, message, sequence)) = row else {
        return Ok(None);
    };
    let members = rooms::members(tx, room).await?;
    let described: Vec<(Uuid, String, String)> = sqlx::query_as("SELECT a.id,a.name,left(a.instructions,300) FROM agents a WHERE a.id=ANY($1) ORDER BY a.name")
        .bind(members.iter().map(|member| member.id).collect::<Vec<_>>()).fetch_all(&mut **tx).await?;
    let recent: Vec<(String, String)> = sqlx::query_as("SELECT a.login,left(m.message,500) FROM room_messages m JOIN human_accounts a ON a.id=m.actor_id WHERE m.room_id=$1 AND m.sequence<$2 ORDER BY m.sequence DESC LIMIT 10")
        .bind(room).bind(sequence).fetch_all(&mut **tx).await?;
    let open: Vec<String> = sqlx::query_scalar("SELECT format('- task %s: %s, on: %s',t.id,a.name,left(d.message,200)) FROM chat_turns t JOIN agents a ON a.id=t.conversation_id JOIN room_messages d ON d.id=t.room_message_id WHERE d.room_id=$1 AND t.status IN ('queued','running','delegated') ORDER BY t.sequence DESC LIMIT 5")
        .bind(room).fetch_all(&mut **tx).await?;
    let members_text: Vec<String> = described
        .iter()
        .map(|(id, name, about)| format!("- id {id}: {name}. {about}"))
        .collect();
    let recent_text: Vec<String> = recent
        .iter()
        .rev()
        .map(|(author, text)| format!("{author}: {text}"))
        .collect();
    let prompt = format!(
        "Room: {name}\n\nMembers:\n{}\n\nOpen tasks:\n{}\n\nEarlier messages, oldest first:\n{}\n\nMessage to route:\n{message}",
        members_text.join("\n"),
        open.join("\n"),
        recent_text.join("\n")
    );
    Ok(Some(Pending { id, prompt }))
}

async fn ask(
    inference: &crate::connections::Inference,
    prompt: &str,
) -> Result<String, &'static str> {
    let messages = [Message::User(prompt.into())];
    let request = Request {
        model: &inference.model,
        key: &inference.api_key,
        system: SYSTEM,
        messages: &messages,
        tools: &[],
        tools_disabled: false,
        max_output_tokens: Some(512),
    };
    let reply = match inference.provider {
        ProviderKind::Openai => {
            complete(
                configured(OpenAi::new, OpenAi::for_loopback_tests)?,
                &request,
            )
            .await
        }
        ProviderKind::Anthropic => {
            complete(
                configured(Anthropic::new, Anthropic::for_loopback_tests)?,
                &request,
            )
            .await
        }
    };
    reply.map_err(|error| error.code())
}

async fn complete<P: Provider>(
    provider: P,
    request: &Request<'_>,
) -> Result<String, cubebot_providers::Error> {
    let stream = provider.stream(request);
    let mut stream = std::pin::pin!(stream);
    let deadline = tokio::time::sleep(Duration::from_secs(60));
    let mut deadline = std::pin::pin!(deadline);
    loop {
        tokio::select! {
            event = stream.next() => match event {
                Some(Ok(Event::Done { reply, .. })) => return Ok(reply.text),
                Some(Ok(Event::Text(_))) => (),
                Some(Err(error)) => return Err(error),
                None => return Err(cubebot_providers::Error::Unavailable),
            },
            _ = &mut deadline => return Err(cubebot_providers::Error::Unavailable),
        }
    }
}

/// The same provider seam as the agent's: a literal loopback origin only under test.
fn configured<P>(
    new: fn() -> Result<P, cubebot_providers::Error>,
    loopback: fn(&str) -> Result<P, cubebot_providers::Error>,
) -> Result<P, &'static str> {
    match std::env::var("CUBEBOT_TEST_PROVIDER_ORIGIN") {
        Ok(origin) => loopback(&origin),
        Err(std::env::VarError::NotPresent) => new(),
        Err(_) => Err(cubebot_providers::Error::Configuration),
    }
    .map_err(|error| error.code())
}

/// The first JSON object in the reply, so a fenced or explained answer still parses.
fn decision(text: &str) -> Option<Decision> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(text.get(start..=end)?).ok()
}

/// Commits a valid result, or counts a failed attempt. A message already settled, by a retry
/// or an earlier run, is left alone.
async fn settle(
    pool: &PgPool,
    pending: &Pending,
    decision: Result<Decision, &'static str>,
) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    let routing: Option<String> =
        sqlx::query_scalar("SELECT routing FROM room_messages WHERE id=$1 FOR UPDATE")
            .bind(pending.id)
            .fetch_optional(&mut *tx)
            .await?;
    if routing.as_deref() != Some("routing") {
        return Ok(());
    }
    let room: Uuid = sqlx::query_scalar("SELECT room_id FROM room_messages WHERE id=$1")
        .bind(pending.id)
        .fetch_one(&mut *tx)
        .await?;
    let (routing, note) = outcome(&mut tx, pending.id, room, decision).await?;
    sqlx::query("UPDATE room_messages SET routing=$2,note=$3 WHERE id=$1")
        .bind(pending.id)
        .bind(routing)
        .bind(note)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// The routing state a coordinator result leads to, delivering a valid assignment.
async fn outcome(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    room: Uuid,
    decision: Result<Decision, &'static str>,
) -> Result<(&'static str, Option<String>), Error> {
    Ok(match decision {
        Ok(Decision::Assign { agent }) => assign(tx, id, room, agent).await?,
        Ok(Decision::Continue { task }) => {
            match super::collaboration::continue_task(tx, id, task).await? {
                Some(agent) => ("continued", Some(agent)),
                None => attempt(tx, id, "coordinator_chose_no_open_task").await?,
            }
        }
        Ok(Decision::Clarify { question }) => ("clarify", Some(bounded(&question))),
        Ok(Decision::None) => ("no_action", None),
        Err(code) => attempt(tx, id, code).await?,
    })
}

/// Delivers to the chosen agent if it is still a member: membership may have changed during
/// the model call.
async fn assign(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    room: Uuid,
    agent: Uuid,
) -> Result<(&'static str, Option<String>), Error> {
    let current = rooms::members(tx, room).await?;
    if !current.iter().any(|member| member.id == agent) {
        return attempt(tx, id, "coordinator_chose_no_member").await;
    }
    // Members are not retired, so waking the agent cannot conflict here.
    rooms::deliver(tx, id, agent)
        .await
        .map_err(|_| Error::Conflict)?;
    Ok(("routed", None))
}

/// Stays routing for another try until the attempts are spent.
async fn attempt(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    code: &str,
) -> Result<(&'static str, Option<String>), Error> {
    let attempts: i16 = sqlx::query_scalar(
        "UPDATE room_messages SET attempts=attempts+1 WHERE id=$1 RETURNING attempts",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    let routing = if attempts >= ATTEMPTS {
        "failed"
    } else {
        "routing"
    };
    Ok((routing, Some(code.into())))
}

fn bounded(text: &str) -> String {
    let mut end = text.len().min(4096);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].replace('\0', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinator_replies_parse_only_into_the_three_results() {
        let id = Uuid::new_v4();
        assert_eq!(
            decision(&format!(
                "```json\n{{\"action\":\"assign\",\"agent\":\"{id}\"}}\n```"
            )),
            Some(Decision::Assign { agent: id })
        );
        assert_eq!(
            decision(r#"{"action":"clarify","question":"Which region?"}"#),
            Some(Decision::Clarify {
                question: "Which region?".into()
            })
        );
        assert_eq!(decision(r#"{"action":"none"}"#), Some(Decision::None));
        assert_eq!(
            decision(&format!(r#"{{"action":"continue","task":"{id}"}}"#)),
            Some(Decision::Continue { task: id })
        );
        for invalid in [
            "Ada should do it",
            r#"{"action":"assign","agent":"Ada"}"#,
            r#"{"action":"run","command":"rm -rf /"}"#,
        ] {
            assert_eq!(decision(invalid), None, "{invalid}");
        }
        assert_eq!(bounded(&"é".repeat(3000)).len(), 4096);
    }
}

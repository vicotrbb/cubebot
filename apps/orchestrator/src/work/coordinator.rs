//! The model call happens outside any transaction; the message stays `routing` until a valid
//! result commits.
use super::rooms;
use crate::{
    agents::kubernetes::Error,
    connections::{Connections, configured},
};
use cubebot_protocol::runtime::{ProviderKind, TokenUsage};
use cubebot_providers::{
    Anthropic, Event, Gemini, Grok, Message, OpenAi, OpenRouter, Provider, Request, Usage,
};
use futures_util::StreamExt;
use serde::Deserialize;
use sqlx::PgPool;
use std::time::Duration;
use tokio::sync::watch;
use uuid::Uuid;

const ATTEMPTS: i16 = 3;

const SYSTEM: &str = "You route one message in a team chat room to the single agent best placed to answer it. You have no tools and do no work yourself. Every message gets an owner, greetings and small talk included: pick the member whose description fits best, or the one the conversation was already with. Reply with JSON only, one of: {\"action\":\"assign\",\"agent\":\"<member id>\"} or {\"action\":\"continue\",\"task\":\"<open task id>\"} when the message corrects or adds to that open task.";

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Decision {
    Assign { agent: Uuid },
    Continue { task: Uuid },
}

struct Pending {
    id: Uuid,
    /// Who sent the message; a personal subscription routes only its owner's.
    sender: Uuid,
    prompt: String,
}

pub async fn coordinate(pool: PgPool, connections: Connections, mut stop: watch::Receiver<bool>) {
    loop {
        if !routed(&pool, &connections).await {
            tokio::select! {
                _ = stop.changed() => return,
                () = tokio::time::sleep(Duration::from_secs(2)) => (),
            }
        }
    }
}

async fn routed(pool: &PgPool, connections: &Connections) -> bool {
    route(pool, connections).await.unwrap_or_else(|error| {
        tracing::warn!(?error, "room coordination failed");
        false
    })
}

pub async fn route(pool: &PgPool, connections: &Connections) -> Result<bool, Error> {
    let mut tx = pool.begin().await?;
    let Some(pending) = pending(&mut tx).await? else {
        return Ok(false);
    };
    let inference = connections.coordinator(&mut tx, pending.sender).await.ok();
    tx.commit().await?;
    dispatch(pool, &pending, inference).await
}

#[tracing::instrument(skip_all, fields(message = %pending.id))]
async fn dispatch(
    pool: &PgPool,
    pending: &Pending,
    inference: Option<crate::connections::Inference>,
) -> Result<bool, Error> {
    let reply = match inference {
        Some(inference) => ask(pool, &inference, SYSTEM, &pending.prompt, 512).await,
        None => Err("provider_unavailable"),
    };
    let decision = reply.and_then(|text| decision(&text).ok_or("invalid_coordinator_result"));
    settle(pool, pending, decision).await
}

async fn pending(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<Option<Pending>, Error> {
    let row = sqlx::query!(
        "
        SELECT m.id, m.room_id, r.name, m.message, m.sequence, m.actor_id
        FROM room_messages m
        JOIN rooms r ON r.id=m.room_id
        WHERE m.routing='routing' AND NOT r.task
        ORDER BY m.sequence
        LIMIT 1
        "
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let (id, room, name, message) = (row.id, row.room_id, row.name, row.message);
    let sender = row.actor_id;
    let members = rooms::members(tx, room).await?;
    let described = sqlx::query!(
        r#"
        SELECT a.id, a.name, left(a.instructions,300) AS "about!"
        FROM agents a
        WHERE a.id=ANY($1)
        ORDER BY a.name
        "#,
        &members.iter().map(|member| member.id).collect::<Vec<_>>(),
    )
    .fetch_all(&mut **tx)
    .await?;
    let recent = sqlx::query!(
        r#"
        SELECT a.login, left(m.message,500) AS "text!"
        FROM room_messages m
        JOIN human_accounts a ON a.id=m.actor_id
        WHERE m.room_id=$1 AND m.sequence<$2
        ORDER BY m.sequence DESC
        LIMIT 10
        "#,
        room,
        row.sequence,
    )
    .fetch_all(&mut **tx)
    .await?;
    let open = sqlx::query_scalar!(
        r#"
        SELECT format('- task %s: %s, on: %s',t.id,a.name,left(d.message,200)) AS "line!"
        FROM chat_turns t
        JOIN agents a ON a.id=t.conversation_id
        JOIN room_messages d ON d.id=t.room_message_id
        WHERE d.room_id=$1 AND t.status IN ('queued','running','delegated')
        ORDER BY t.sequence DESC
        LIMIT 5
        "#,
        room,
    )
    .fetch_all(&mut **tx)
    .await?;
    let members_text: Vec<String> = described
        .iter()
        .map(|agent| format!("- id {}: {}. {}", agent.id, agent.name, agent.about))
        .collect();
    let recent_text: Vec<String> = recent
        .iter()
        .rev()
        .map(|recent| format!("{}: {}", recent.login, recent.text))
        .collect();
    let prompt = format!(
        "Room: {name}\n\nMembers:\n{}\n\nOpen tasks:\n{}\n\nEarlier messages, oldest first:\n{}\n\nMessage to route:\n{message}",
        members_text.join("\n"),
        open.join("\n"),
        recent_text.join("\n")
    );
    Ok(Some(Pending { id, sender, prompt }))
}

pub(super) async fn ask(
    pool: &PgPool,
    inference: &crate::connections::Inference,
    system: &str,
    prompt: &str,
    max_output_tokens: u64,
) -> Result<String, &'static str> {
    let messages = [Message::User(prompt.into())];
    let request = Request {
        model: &inference.model,
        key: &inference.api_key,
        system,
        messages: &messages,
        tools: &[],
        tools_disabled: false,
        max_output_tokens: Some(max_output_tokens),
    };
    let (text, usage) = through(inference.provider, &request)
        .await
        .map_err(|error| error.code())?;
    if let Some(usage) = usage {
        let usage = TokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cached_input_tokens: usage.cached_input_tokens,
            cost_usd: usage.cost_usd,
        };
        let model = (inference.provider.as_str(), inference.model.as_str());
        if let Err(error) = super::usage::record_duty(pool, None, None, model, &usage).await {
            tracing::warn!(?error, "coordinator usage not recorded");
        }
    }
    Ok(text)
}

async fn through(
    provider: ProviderKind,
    request: &Request<'_>,
) -> Result<(String, Option<Usage>), cubebot_providers::Error> {
    match provider {
        ProviderKind::Openai => {
            complete(configured(OpenAi::new, OpenAi::for_loopback_tests), request).await
        }
        ProviderKind::Anthropic => {
            complete(
                configured(Anthropic::new, Anthropic::for_loopback_tests),
                request,
            )
            .await
        }
        ProviderKind::Openrouter => {
            complete(
                configured(OpenRouter::new, OpenRouter::for_loopback_tests),
                request,
            )
            .await
        }
        ProviderKind::Gemini => {
            complete(configured(Gemini::new, Gemini::for_loopback_tests), request).await
        }
        ProviderKind::Grok => {
            complete(configured(Grok::new, Grok::for_loopback_tests), request).await
        }
        ProviderKind::Chatgpt => {
            complete(
                configured(OpenAi::chatgpt, OpenAi::chatgpt_for_loopback_tests),
                request,
            )
            .await
        }
        ProviderKind::Supergrok => {
            complete(
                configured(OpenAi::supergrok, OpenAi::supergrok_for_loopback_tests),
                request,
            )
            .await
        }
        ProviderKind::Minimax | ProviderKind::MinimaxPlan => {
            complete(
                configured(Anthropic::minimax, Anthropic::minimax_for_loopback_tests),
                request,
            )
            .await
        }
        ProviderKind::MinimaxCn | ProviderKind::MinimaxPlanCn => {
            complete(
                configured(Anthropic::minimax_cn, Anthropic::minimax_for_loopback_tests),
                request,
            )
            .await
        }
    }
}

async fn complete<P: Provider>(
    provider: Result<P, cubebot_providers::Error>,
    request: &Request<'_>,
) -> Result<(String, Option<Usage>), cubebot_providers::Error> {
    let provider = provider?;
    let stream = provider.stream(request);
    let mut stream = std::pin::pin!(stream);
    let deadline = tokio::time::sleep(Duration::from_secs(60));
    let mut deadline = std::pin::pin!(deadline);
    loop {
        tokio::select! {
            event = stream.next() => match event {
                Some(Ok(Event::Done { reply, usage })) => return Ok((reply.text, usage)),
                Some(Ok(Event::Text(_))) => (),
                Some(Err(error)) => return Err(error),
                None => return Err(cubebot_providers::Error::Unavailable),
            },
            () = &mut deadline => return Err(cubebot_providers::Error::Unavailable),
        }
    }
}

fn decision(text: &str) -> Option<Decision> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(text.get(start..=end)?).ok()
}

async fn settle(
    pool: &PgPool,
    pending: &Pending,
    decision: Result<Decision, &'static str>,
) -> Result<bool, Error> {
    let mut tx = pool.begin().await?;
    let routing = sqlx::query_scalar!(
        "
        SELECT routing
        FROM room_messages
        WHERE id=$1
        FOR UPDATE
        ",
        pending.id,
    )
    .fetch_optional(&mut *tx)
    .await?;
    if routing.as_deref() != Some("routing") {
        return Ok(true);
    }
    let room = sqlx::query_scalar!(
        "
        SELECT room_id
        FROM room_messages
        WHERE id=$1
        ",
        pending.id,
    )
    .fetch_one(&mut *tx)
    .await?;
    let (routing, note) = outcome(&mut tx, pending.id, room, decision).await?;
    sqlx::query!(
        "
        UPDATE room_messages
        SET routing=$2, note=$3
        WHERE id=$1
        ",
        pending.id,
        routing,
        note,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(routing != "routing")
}

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
                None => attempt(tx, id, room, "coordinator_chose_no_open_task").await?,
            }
        }
        Err(code) => attempt(tx, id, room, code).await?,
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
        return attempt(tx, id, room, "coordinator_chose_no_member").await;
    }
    rooms::deliver(tx, id, agent)
        .await
        .map_err(|_| Error::Conflict)?;
    Ok(("routed", None))
}

async fn attempt(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    room: Uuid,
    code: &str,
) -> Result<(&'static str, Option<String>), Error> {
    let attempts = sqlx::query_scalar!(
        "
        UPDATE room_messages
        SET attempts=attempts+1
        WHERE id=$1
        RETURNING attempts
        ",
        id,
    )
    .fetch_one(&mut **tx)
    .await?;
    if attempts < ATTEMPTS {
        return Ok(("routing", Some(code.into())));
    }
    let Some(first) = rooms::members(tx, room).await?.into_iter().next() else {
        return Ok(("failed", Some(code.into())));
    };
    tracing::warn!(
        code,
        "room coordinator gave no valid owner; the first member answers"
    );
    rooms::deliver(tx, id, first.id)
        .await
        .map_err(|_| Error::Conflict)?;
    Ok(("routed", None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinator_replies_parse_only_into_an_owner_or_an_open_task() {
        let id = Uuid::new_v4();
        assert_eq!(
            decision(&format!(
                "```json\n{{\"action\":\"assign\",\"agent\":\"{id}\"}}\n```"
            )),
            Some(Decision::Assign { agent: id })
        );
        assert_eq!(
            decision(&format!(r#"{{"action":"continue","task":"{id}"}}"#)),
            Some(Decision::Continue { task: id })
        );
        for invalid in [
            "Ada should do it",
            r#"{"action":"assign","agent":"Ada"}"#,
            r#"{"action":"run","command":"rm -rf /"}"#,
            r#"{"action":"clarify","question":"Which region?"}"#,
            r#"{"action":"none"}"#,
        ] {
            assert_eq!(decision(invalid), None, "{invalid}");
        }
    }
}

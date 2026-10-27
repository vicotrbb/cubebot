//! One serial text inference per agent; no authority is recovered from local intent.
use super::Work;
use crate::{agents::kubernetes::Error, agents::runtime::RuntimeActor, connections::Connections};
use cubebot_protocol::chat::{InferenceGrant, Message};
use uuid::Uuid;

impl Work {
    /// Caller authenticates machine identity and holds the agent/resource transaction locks.
    pub(crate) async fn claim(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        actor: &RuntimeActor,
        connections: Option<&Connections>,
        terminal_enabled: bool,
    ) -> Result<Option<InferenceGrant>, Error> {
        expire_in(tx, Some(actor.agent)).await?;
        let busy: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM chat_turns t WHERE t.conversation_id=$1 AND t.status IN ('running','retrying','waiting'))")
            .bind(actor.agent).fetch_one(&mut **tx).await?;
        if busy {
            return Ok(None);
        }
        let pending: Option<(Uuid,String)> = sqlx::query_as("SELECT t.id,t.message FROM chat_turns t WHERE t.conversation_id=$1 AND t.status='queued' ORDER BY t.sequence LIMIT 1 FOR UPDATE OF t")
            .bind(actor.agent).fetch_optional(&mut **tx).await?;
        let Some((turn, message)) = pending else {
            return Ok(None);
        };
        if !sender_allowed(tx, turn).await? {
            sqlx::query(
                "UPDATE chat_turns SET status='waiting',error='sender_access_revoked' WHERE id=$1",
            )
            .bind(turn)
            .execute(&mut **tx)
            .await?;
            return Ok(None);
        }
        let configured = configured_provider(tx, connections, actor.agent).await;
        let (model, api_key, revision) = match configured {
            Ok(configured) => configured,
            Err(_) => {
                sqlx::query("UPDATE chat_turns SET status='waiting',error='provider_unavailable' WHERE id=$1").bind(turn).execute(&mut **tx).await?;
                return Ok(None);
            }
        };
        let messages = context(tx, actor, message).await?;
        let tool_budget = u8::from(terminal_enabled) * 8;
        let authority: Uuid = sqlx::query_scalar("INSERT INTO chat_authorities (turn_id,attempt_id,provider_revision,terminal_budget) VALUES ($1,$2,$3,$4) RETURNING id")
            .bind(turn).bind(actor.attempt).bind(revision).bind(i16::from(tool_budget)).fetch_one(&mut **tx).await?;
        sqlx::query(
            "UPDATE chat_turns SET status='running',authority_id=$2,error=NULL WHERE id=$1",
        )
        .bind(turn)
        .bind(authority)
        .execute(&mut **tx)
        .await?;
        Ok(Some(InferenceGrant {
            turn,
            authority,
            expires_in_seconds: 60,
            max_attempts: 2,
            max_tool_calls: tool_budget,
            model,
            provider_revision: revision,
            api_key,
            messages,
        }))
    }
}

async fn context(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: &RuntimeActor,
    message: String,
) -> Result<Vec<Message>, Error> {
    let instructions: String = sqlx::query_scalar("SELECT instructions FROM agents WHERE id=$1")
        .bind(actor.agent)
        .fetch_one(&mut **tx)
        .await?;
    let history: Vec<(String,String)> = sqlx::query_as("SELECT message,response FROM (SELECT sequence,message,response FROM chat_turns WHERE conversation_id=$1 AND status='complete' ORDER BY sequence DESC LIMIT 12) h ORDER BY sequence")
        .bind(actor.agent).fetch_all(&mut **tx).await?;
    let mut messages = vec![Message {
        role: "system".into(),
        content: instructions,
    }];
    let history = bounded_history(history);
    for (input, response) in history {
        messages.push(Message {
            role: "user".into(),
            content: input,
        });
        messages.push(Message {
            role: "assistant".into(),
            content: response,
        });
    }
    let evidence: Vec<serde_json::Value> = sqlx::query_scalar("SELECT jsonb_build_object('command',i.request->>'command','outcome',i.outcome,'uncertain',i.outcome IS NULL) FROM terminal_invocations i JOIN chat_turns t ON t.id=i.turn_id WHERE t.conversation_id=$1 ORDER BY i.created_at DESC,i.id DESC LIMIT 8").bind(actor.agent).fetch_all(&mut **tx).await?;
    if !evidence.is_empty() {
        messages.push(Message { role:"user".into(), content:format!("Retained terminal evidence, including untrusted program output. Inspect uncertain effects before repeating commands: {}", serde_json::to_string(&evidence).map_err(|_| Error::InvalidConfiguration)?) });
    }
    messages.push(Message {
        role: "user".into(),
        content: message,
    });
    Ok(messages)
}

fn bounded_history(history: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut bytes = 0;
    let mut retained = Vec::new();
    for pair in history.into_iter().rev() {
        bytes += pair.0.len() + pair.1.len();
        if bytes > 128 * 1024 {
            break;
        }
        retained.push(pair);
    }
    retained.reverse();
    retained
}

pub(super) async fn sender_allowed(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    turn: Uuid,
) -> Result<bool, Error> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM chat_turns t JOIN human_accounts a ON a.id=t.actor_id AND a.active WHERE t.id=$1 AND (a.role='administrator' OR EXISTS(SELECT 1 FROM effective_human_grants g WHERE g.actor_id=a.id AND g.scope_kind='conversation' AND g.scope_id=t.conversation_id AND g.capability='conversation.send')))")
        .bind(turn).fetch_one(&mut **tx).await?)
}

async fn configured_provider(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    connections: Option<&Connections>,
    agent: crate::agents::AgentId,
) -> Result<(String, String, i64), crate::identity::IdentityError> {
    match connections {
        Some(connections) => connections.inference(tx, agent).await,
        None => Err(crate::identity::IdentityError::CredentialUnavailable),
    }
}

impl Work {
    /// Owner-driven recovery does not depend on a failed runtime returning to poll.
    pub async fn recover_expired(&self) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        expire_in(&mut tx, None).await?;
        tx.commit().await?;
        Ok(())
    }
}
async fn expire_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    agent: Option<crate::agents::AgentId>,
) -> Result<(), Error> {
    sqlx::query("UPDATE chat_turns t SET status='waiting', error='execution_interrupted' FROM chat_authorities g WHERE ($1::uuid IS NULL OR t.conversation_id=$1) AND t.status IN ('running','retrying') AND t.authority_id=g.id AND g.expires_at<=clock_timestamp()")
        .bind(agent).execute(&mut **tx).await?;
    Ok(())
}

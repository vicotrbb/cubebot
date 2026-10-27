//! Reconcile typed outcomes before recovery custody releases runtime admission.
use crate::agents::{AgentId, kubernetes::Error};
use cubebot_protocol::chat::{ChatEvidence, Outcome};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(crate) async fn reconcile(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    attempt: crate::agents::kubernetes::AttemptId,
    was_active: bool,
    payload: &[u8],
) -> Result<bool, Error> {
    // Other journal formats retain custody without claiming semantic acceptance.
    let evidence = match serde_json::from_slice::<ChatEvidence>(payload) {
        Ok(evidence) => evidence,
        Err(_) => return Ok(false),
    };
    let (turn, authority) = match &evidence {
        ChatEvidence::TerminalOutcome {
            turn, authority, ..
        }
        | ChatEvidence::TextIntent { turn, authority }
        | ChatEvidence::TextRetry { turn, authority }
        | ChatEvidence::TextOutcome {
            turn, authority, ..
        } => (*turn, *authority),
    };
    let known: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM chat_authorities g JOIN chat_turns t ON t.id=g.turn_id WHERE g.id=$1 AND t.id=$2 AND t.conversation_id=$3)")
        .bind(authority).bind(turn).bind(agent).fetch_one(&mut **tx).await?;
    if !known {
        return Err(Error::Unauthorized);
    }
    if matches!(evidence, ChatEvidence::TextRetry { .. }) {
        sqlx::query("UPDATE chat_turns SET status='retrying' WHERE id=$1 AND authority_id=$2 AND status='running'").bind(turn).bind(authority).execute(&mut **tx).await?;
    }
    if let ChatEvidence::TerminalOutcome {
        invocation,
        ref outcome,
        ..
    } = evidence
    {
        super::terminal::outcome(tx, authority, invocation, outcome).await?;
    }
    if let ChatEvidence::TextOutcome { outcome, .. } = evidence {
        commit(tx, turn, authority, &outcome, attempt, was_active).await?;
    }
    Ok(true)
}

async fn commit(
    tx: &mut Transaction<'_, Postgres>,
    turn: Uuid,
    authority: Uuid,
    outcome: &Outcome,
    attempt: crate::agents::kubernetes::AttemptId,
    was_active: bool,
) -> Result<(), Error> {
    let value = serde_json::to_value(outcome).map_err(|_| Error::Conflict)?;
    let old: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT outcome FROM chat_authorities WHERE id=$1 FOR UPDATE")
            .bind(authority)
            .fetch_one(&mut **tx)
            .await?;
    if let Some(old) = old {
        if old != value {
            return Err(Error::Conflict);
        }
        return Ok(());
    }
    let (status, current): (String, Option<Uuid>) =
        sqlx::query_as("SELECT status,authority_id FROM chat_turns WHERE id=$1 FOR UPDATE")
            .bind(turn)
            .fetch_one(&mut **tx)
            .await?;
    sqlx::query("UPDATE chat_authorities SET outcome=$2 WHERE id=$1")
        .bind(authority)
        .bind(value)
        .execute(&mut **tx)
        .await?;
    // Retain stale outcomes as evidence; never replace an accepted newer authority or reply.
    if current != Some(authority) || matches!(status.as_str(), "complete" | "cancelled") {
        return Ok(());
    }
    if !terminal_result_current(tx, turn, authority, attempt, was_active).await? {
        sqlx::query(
            "UPDATE chat_turns SET status='waiting',error='execution_interrupted' WHERE id=$1",
        )
        .bind(turn)
        .execute(&mut **tx)
        .await?;
        return Ok(());
    }
    apply(tx, turn, outcome).await
}

async fn apply(
    tx: &mut Transaction<'_, Postgres>,
    turn: Uuid,
    outcome: &Outcome,
) -> Result<(), Error> {
    match outcome {
        Outcome::Complete { response } => {
            if response.is_empty() || response.len() > 32768 || response.contains('\0') {
                return Err(Error::InvalidConfiguration);
            }
            sqlx::query("UPDATE chat_turns SET status='complete',response=$2,error=NULL,completed_at=clock_timestamp() WHERE id=$1")
                .bind(turn).bind(response).execute(&mut **tx).await?;
        }
        Outcome::Waiting { error } => {
            if !matches!(
                error.as_str(),
                "provider_unavailable"
                    | "provider_rejected"
                    | "provider_invalid_response"
                    | "authority_expired"
            ) {
                return Err(Error::InvalidConfiguration);
            }
            sqlx::query("UPDATE chat_turns SET status='waiting',error=$2 WHERE id=$1")
                .bind(turn)
                .bind(error)
                .execute(&mut **tx)
                .await?;
        }
    }
    Ok(())
}

async fn terminal_result_current(
    tx: &mut Transaction<'_, Postgres>,
    turn: Uuid,
    authority: Uuid,
    attempt: crate::agents::kubernetes::AttemptId,
    was_active: bool,
) -> Result<bool, Error> {
    let terminal: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM terminal_invocations WHERE authority_id=$1)",
    )
    .bind(authority)
    .fetch_one(&mut **tx)
    .await?;
    if !terminal {
        return Ok(true);
    }
    if !was_active || !super::runtime::sender_allowed(tx, turn).await? {
        return Ok(false);
    }
    // This record holds new-work admission until reconciliation; it cannot
    // disqualify itself. The sampled admission still binds the issuing attempt.
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM chat_authorities g JOIN provider_connection p ON p.revision=g.provider_revision WHERE g.id=$1 AND g.attempt_id=$2 AND g.expires_at>clock_timestamp())").bind(authority).bind(attempt).fetch_one(&mut **tx).await?)
}

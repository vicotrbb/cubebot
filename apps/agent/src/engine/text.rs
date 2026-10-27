use crate::{
    control::{Error, RuntimeClient},
    journal::Journal,
};
use cubebot_protocol::chat::{ChatEvidence, Outcome};
use std::time::{Duration, Instant};

pub(super) async fn execute(client: &RuntimeClient, journal: &mut Journal) -> Result<(), Error> {
    let start = Instant::now();
    let Some(grant) = client.claim_text().await? else {
        return Ok(());
    };
    let deadline = start + Duration::from_secs(grant.expires_in_seconds);
    append(
        journal,
        &ChatEvidence::TextIntent {
            turn: grant.turn,
            authority: grant.authority,
        },
    )?;
    let outcome = infer(&grant, deadline, client, journal).await?;
    let evidence = ChatEvidence::TextOutcome {
        turn: grant.turn,
        authority: grant.authority,
        outcome,
    };
    append(journal, &evidence)
}

async fn infer(
    grant: &cubebot_protocol::chat::InferenceGrant,
    deadline: Instant,
    client: &RuntimeClient,
    journal: &mut Journal,
) -> Result<Outcome, Error> {
    let provider = match configured_provider() {
        Ok(provider) => provider,
        Err(error) => {
            return Ok(Outcome::Waiting {
                error: error.code().into(),
            });
        }
    };
    let mut result = model_call(&provider, grant, deadline).await;
    if result.as_ref().is_err_and(|error| error.retryable()) && grant.max_attempts >= 2 {
        append(
            journal,
            &ChatEvidence::TextRetry {
                turn: grant.turn,
                authority: grant.authority,
            },
        )?;
        if !publish_progress(client, journal).await? {
            return Ok(Outcome::Waiting {
                error: "authority_expired".into(),
            });
        }
        // Each random authority spreads a single bounded retry across 0.5–1.5 seconds.
        let bytes = grant.authority.as_bytes();
        let backoff =
            Duration::from_millis(500 + u64::from(u16::from_be_bytes([bytes[0], bytes[1]])) % 1001);
        if tokio::time::timeout_at(deadline.into(), tokio::time::sleep(backoff))
            .await
            .is_err()
        {
            return Ok(Outcome::Waiting {
                error: "authority_expired".into(),
            });
        }
        result = model_call(&provider, grant, deadline).await;
    }
    Ok(match result {
        Ok(response) => bounded_response(response),
        Err(_) if Instant::now() >= deadline => Outcome::Waiting {
            error: "authority_expired".into(),
        },
        Err(error) => Outcome::Waiting {
            error: error.code().into(),
        },
    })
}

fn configured_provider() -> Result<cubebot_providers::OpenAi, cubebot_providers::Error> {
    match std::env::var("CUBEBOT_TEST_PROVIDER_ORIGIN") {
        Ok(origin) => cubebot_providers::OpenAi::for_loopback_tests(&origin),
        Err(std::env::VarError::NotPresent) => cubebot_providers::OpenAi::new(),
        Err(_) => Err(cubebot_providers::Error::Configuration),
    }
}
async fn model_call(
    provider: &cubebot_providers::OpenAi,
    grant: &cubebot_protocol::chat::InferenceGrant,
    deadline: Instant,
) -> Result<String, cubebot_providers::Error> {
    if Instant::now() >= deadline {
        return Err(cubebot_providers::Error::Unavailable);
    }
    tokio::time::timeout_at(
        deadline.into(),
        provider.complete(&grant.model, &grant.api_key, &grant.messages),
    )
    .await
    .unwrap_or(Err(cubebot_providers::Error::Unavailable))
}
async fn publish_progress(client: &RuntimeClient, journal: &mut Journal) -> Result<bool, Error> {
    for _ in 0..2 {
        let Some(record) = journal.pending().first() else {
            break;
        };
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            client.synchronize_record(record.sequence, &record.payload),
        )
        .await;
        match result {
            Ok(Ok(())) => {
                journal
                    .acknowledge(record.sequence)
                    .map_err(|_| Error::Journal)?;
            }
            Ok(Err(Error::Rejected | Error::Expired | Error::Protocol)) => return Ok(false),
            _ => return Ok(true),
        }
    }
    Ok(true)
}

fn bounded_response(response: String) -> Outcome {
    // JSON escaping can expand a short text result beyond the private journal record limit.
    if serde_json::to_vec(&response).is_ok_and(|encoded| encoded.len() <= 60 * 1024) {
        Outcome::Complete { response }
    } else {
        Outcome::Waiting {
            error: "provider_invalid_response".into(),
        }
    }
}

fn append(journal: &mut Journal, evidence: &ChatEvidence) -> Result<(), Error> {
    let payload = serde_json::to_vec(evidence).map_err(|_| Error::Journal)?;
    journal.append(&payload).map_err(|_| Error::Journal)?;
    Ok(())
}

pub(super) fn can_start(
    disposition: cubebot_protocol::runtime::RuntimeDisposition,
    journal: &Journal,
) -> bool {
    matches!(
        disposition,
        cubebot_protocol::runtime::RuntimeDisposition::Available
    ) && journal.pending().is_empty()
}

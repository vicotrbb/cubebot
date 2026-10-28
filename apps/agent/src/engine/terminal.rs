//! Bounded native tool loop. No invocation is recovered by executing it again.
use crate::{
    control::{Error, RuntimeClient},
    journal::Journal,
};
use cubebot_protocol::{
    chat::{ChatEvidence, InferenceGrant, Outcome},
    terminal::{Authorization, Invocation},
};
use serde_json::json;
use std::{
    path::Path,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub(super) async fn infer(
    grant: &InferenceGrant,
    client: &mut RuntimeClient,
    journal: &mut Journal,
) -> Result<Outcome, Error> {
    let provider = super::text::configured_provider().map_err(|_| Error::Configuration)?;
    let mut input = grant
        .messages
        .iter()
        .map(|message| serde_json::to_value(message).map_err(|_| Error::Protocol))
        .collect::<Result<Vec<_>, _>>()?;
    loop {
        let step = match model_step(&provider, grant, &input, client).await {
            Ok(step) => step,
            Err(outcome) => return Ok(outcome),
        };
        input.extend(step.output);
        let Some(call) = step.call else {
            return Ok(super::text::bounded_response(step.text));
        };
        let arguments = match crate::tools::terminal_arguments(&call.name, &call.arguments) {
            Ok(arguments) => arguments,
            Err(_) => {
                return Ok(Outcome::Waiting {
                    error: "provider_invalid_response".into(),
                });
            }
        };
        let authorization = Authorization {
            turn: grant.turn,
            invocation: Invocation {
                id: Uuid::new_v4(),
                authority: grant.authority,
                command: arguments.command,
            },
        };
        let (result, continuing) = match invoke(client, &authorization).await {
            Ok(result) => result,
            Err(_) => return Ok(waiting()),
        };
        super::text::append(
            journal,
            &ChatEvidence::TerminalOutcome {
                turn: grant.turn,
                authority: grant.authority,
                invocation: authorization.invocation.id,
                outcome: result.clone(),
            },
        )?;
        if !super::text::publish_progress(client, journal).await? {
            return Ok(waiting());
        }
        input.push(json!({"type":"function_call_output","call_id":call.call_id,"output":serde_json::to_string(&result).map_err(|_| Error::Protocol)?}));
        if !continuing {
            return Ok(waiting());
        }
    }
}
fn waiting() -> Outcome {
    Outcome::Waiting {
        error: "authority_expired".into(),
    }
}

async fn invoke(
    client: &mut RuntimeClient,
    input: &Authorization,
) -> Result<(cubebot_protocol::terminal::Outcome, bool), Error> {
    // The orchestrator records the attempt before anything runs locally.
    let deadline = execution_permit(client, input).await?;
    let mut attempts = attempts()?;
    if let Some(outcome) = attempts
        .begin(&input.invocation)
        .map_err(|_| Error::Journal)?
    {
        return Ok((outcome, false));
    }
    let outcome = execute(client, input, deadline).await?;
    attempts
        .finish(input.invocation.id, &outcome)
        .map_err(|_| Error::Journal)?;
    let continuing = matches!(outcome, cubebot_protocol::terminal::Outcome::Exited { .. });
    Ok((outcome, continuing))
}
async fn execute(
    client: &mut RuntimeClient,
    input: &Authorization,
    mut deadline: Instant,
) -> Result<cubebot_protocol::terminal::Outcome, Error> {
    let workspace =
        std::env::var("CUBEBOT_WORKSPACE_DIRECTORY").map_err(|_| Error::Configuration)?;
    let (mut child, path) =
        super::process::spawn(&input.invocation, &workspace).map_err(|_| Error::Unavailable)?;
    let status = loop {
        tokio::select! {
            status = child.wait() => break status.ok().map(|status| status.code()),
            _ = tokio::time::sleep_until(deadline.into()) => break None,
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                match execution_permit(client, input).await {
                    Ok(until) => deadline = until,
                    Err(Error::Unavailable) => (),
                    Err(_) => break None,
                }
            }
        }
    };
    Ok(super::process::finish(child, &path, status).await)
}
fn attempts() -> Result<super::attempts::Store, Error> {
    let root = std::env::var_os("CUBEBOT_JOURNAL_DIRECTORY").ok_or(Error::Configuration)?;
    super::attempts::Store::open(&Path::new(&root).join("attempts")).map_err(|_| Error::Journal)
}
async fn execution_permit(
    client: &mut RuntimeClient,
    input: &Authorization,
) -> Result<Instant, Error> {
    if client.requires_bootstrap() {
        client.bootstrap().await?;
    }
    let start = Instant::now();
    let permit = tokio::time::timeout(Duration::from_secs(2), client.authorize_terminal(input))
        .await
        .map_err(|_| Error::Unavailable)??;
    Ok(start + Duration::from_millis(permit.milliseconds))
}

pub(super) async fn recover(client: &RuntimeClient, journal: &mut Journal) -> Result<(), Error> {
    let pending = client.pending_terminal().await?;
    if pending.is_empty() {
        return Ok(());
    }
    let attempts = attempts()?;
    for input in pending {
        // Unknown or unfinished attempts stay uncertain; recovery never runs them again.
        let Ok(outcome) = attempts.lookup(input.invocation.id, input.invocation.authority) else {
            return Ok(());
        };
        super::text::append(
            journal,
            &ChatEvidence::TerminalOutcome {
                turn: input.turn,
                authority: input.invocation.authority,
                invocation: input.invocation.id,
                outcome,
            },
        )?;
    }
    Ok(())
}

async fn refresh(client: &mut RuntimeClient, grant: &InferenceGrant) -> Result<Instant, Error> {
    if client.requires_bootstrap() {
        client.bootstrap().await?;
    }
    let start = Instant::now();
    let permit = tokio::time::timeout(
        Duration::from_secs(2),
        client.renew_terminal(&cubebot_protocol::terminal::Renewal {
            turn: grant.turn,
            authority: grant.authority,
        }),
    )
    .await
    .map_err(|_| Error::Unavailable)??;
    Ok(start + Duration::from_millis(permit.milliseconds))
}
async fn model_step(
    provider: &cubebot_providers::OpenAi,
    grant: &InferenceGrant,
    input: &[serde_json::Value],
    client: &mut RuntimeClient,
) -> Result<cubebot_providers::tools::Step, Outcome> {
    let mut deadline = refresh(client, grant).await.map_err(|_| waiting())?;
    let tools = [crate::tools::terminal_definition()];
    let request = provider.tool_step(&grant.model, &grant.api_key, input, &tools);
    tokio::pin!(request);
    loop {
        tokio::select! {
            step = &mut request => return step.map_err(|error| Outcome::Waiting { error: error.code().into() }),
            _ = tokio::time::sleep_until(deadline.into()) => return Err(waiting()),
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                match refresh(client, grant).await {
                    Ok(until) => deadline = until,
                    Err(Error::Unavailable) => (),
                    Err(_) => return Err(waiting()),
                }
            }
        }
    }
}

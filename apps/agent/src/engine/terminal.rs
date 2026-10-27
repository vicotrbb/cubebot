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
use std::time::{Duration, Instant};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
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
    let (outcome, continuing) = admitted(client, input).await?;
    if !matches!(outcome, cubebot_protocol::terminal::Outcome::Uncertain) {
        return Ok((outcome, continuing));
    }
    // A distinct read-only request reconciles cancellation without replaying effects.
    Ok((
        tokio::time::timeout(Duration::from_secs(4), lookup(input))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(cubebot_protocol::terminal::Outcome::Uncertain),
        false,
    ))
}
async fn lookup(input: &Authorization) -> Result<cubebot_protocol::terminal::Outcome, Error> {
    let mut socket = executor_connection().await?;
    let request = serde_json::to_vec(&cubebot_protocol::terminal::Request::Lookup {
        id: input.invocation.id,
        authority: input.invocation.authority,
    })
    .map_err(|_| Error::Protocol)?;
    socket
        .write_u32(request.len() as u32)
        .await
        .map_err(|_| Error::Unavailable)?;
    socket
        .write_all(&request)
        .await
        .map_err(|_| Error::Unavailable)?;
    let mut response = Vec::new();
    socket
        .take(60 * 1024)
        .read_to_end(&mut response)
        .await
        .map_err(|_| Error::Unavailable)?;
    serde_json::from_slice(&response).map_err(|_| Error::Protocol)
}

async fn admitted(
    client: &mut RuntimeClient,
    input: &Authorization,
) -> Result<(cubebot_protocol::terminal::Outcome, bool), Error> {
    let (mut deadline, not_after_millis) = execution_permit(client, input).await?;
    let mut socket = executor_connection().await?;
    let request = serde_json::to_vec(&cubebot_protocol::terminal::Request::Execute {
        invocation: input.invocation.clone(),
        not_after_millis,
    })
    .map_err(|_| Error::Protocol)?;
    socket
        .write_u32(request.len() as u32)
        .await
        .map_err(|_| Error::Unavailable)?;
    socket
        .write_all(&request)
        .await
        .map_err(|_| Error::Unavailable)?;
    let (mut read, mut write) = socket.into_split();
    let response = async {
        let mut bytes = Vec::new();
        (&mut read)
            .take(60 * 1024)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| Error::Unavailable)?;
        serde_json::from_slice(&bytes).map_err(|_| Error::Protocol)
    };
    tokio::pin!(response);
    loop {
        tokio::select! {
            outcome = &mut response => return outcome.map(|outcome| (outcome, true)),
            _ = tokio::time::sleep_until(deadline.into()) => return Ok((cubebot_protocol::terminal::Outcome::Uncertain, false)),
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                match execution_permit(client, input).await {
                    Ok((until, renewal)) => {
                        deadline = until;
                        write.write_u64(renewal).await.map_err(|_| Error::Unavailable)?;
                    }
                    Err(Error::Unavailable) => (),
                    Err(_) => return Ok((cubebot_protocol::terminal::Outcome::Uncertain, false)),
                }
            }
        }
    }
}
async fn execution_permit(
    client: &mut RuntimeClient,
    input: &Authorization,
) -> Result<(Instant, u64), Error> {
    if client.requires_bootstrap() {
        client.bootstrap().await?;
    }
    let start = Instant::now();
    let clock = crate::execution::terminal::monotonic_millis().map_err(|_| Error::Unavailable)?;
    let permit = tokio::time::timeout(Duration::from_secs(2), client.authorize_terminal(input))
        .await
        .map_err(|_| Error::Unavailable)??;
    Ok((
        start + Duration::from_millis(permit.milliseconds),
        clock + permit.milliseconds,
    ))
}

pub(super) async fn recover(client: &RuntimeClient, journal: &mut Journal) -> Result<(), Error> {
    for input in client.pending_terminal().await? {
        let outcome = match tokio::time::timeout(Duration::from_secs(4), lookup(&input)).await {
            Ok(Ok(outcome)) => outcome,
            _ => return Ok(()),
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
async fn executor_connection() -> Result<UnixStream, Error> {
    let socket = UnixStream::connect("/run/cubebot-executor/terminal.sock")
        .await
        .map_err(|_| Error::Unavailable)?;
    if socket.peer_cred().map_err(|_| Error::Unavailable)?.uid() != 0 {
        return Err(Error::Rejected);
    }
    Ok(socket)
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

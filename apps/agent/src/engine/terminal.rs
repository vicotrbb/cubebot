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
    deadline: Instant,
    client: &RuntimeClient,
    journal: &mut Journal,
) -> Result<Outcome, Error> {
    let provider = super::text::configured_provider().map_err(|_| Error::Configuration)?;
    let mut input = grant
        .messages
        .iter()
        .map(|message| serde_json::to_value(message).map_err(|_| Error::Protocol))
        .collect::<Result<Vec<_>, _>>()?;
    for _ in 0..=grant.max_tool_calls {
        let step = match model_step(&provider, grant, &input, deadline).await {
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
                seconds: arguments.seconds,
            },
        };
        let (result, continuing) = match invoke(client, &authorization, deadline).await {
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
    Ok(waiting())
}
fn waiting() -> Outcome {
    Outcome::Waiting {
        error: "authority_expired".into(),
    }
}

async fn invoke(
    client: &RuntimeClient,
    input: &Authorization,
    deadline: Instant,
) -> Result<(cubebot_protocol::terminal::Outcome, bool), Error> {
    let (outcome, continuing) = admitted(client, input, deadline).await?;
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
    client: &RuntimeClient,
    input: &Authorization,
    turn_deadline: Instant,
) -> Result<(cubebot_protocol::terminal::Outcome, bool), Error> {
    let start = Instant::now();
    let monotonic_start =
        crate::execution::terminal::monotonic_millis().map_err(|_| Error::Unavailable)?;
    let permit = tokio::time::timeout(Duration::from_secs(2), client.authorize_terminal(input))
        .await
        .map_err(|_| Error::Unavailable)??;
    let deadline = (start + Duration::from_millis(permit.milliseconds)).min(turn_deadline);
    let mut socket = executor_connection().await?;
    let request = serde_json::to_vec(&cubebot_protocol::terminal::Request::Execute {
        invocation: input.invocation.clone(),
        not_after_millis: monotonic_start + permit.milliseconds,
    })
    .map_err(|_| Error::Protocol)?;
    let execution = async {
        socket
            .write_u32(request.len() as u32)
            .await
            .map_err(|_| Error::Unavailable)?;
        socket
            .write_all(&request)
            .await
            .map_err(|_| Error::Unavailable)?;
        let mut response = Vec::new();
        (&mut socket)
            .take(60 * 1024)
            .read_to_end(&mut response)
            .await
            .map_err(|_| Error::Unavailable)?;
        serde_json::from_slice(&response).map_err(|_| Error::Protocol)
    };
    tokio::pin!(execution);
    tokio::select! {
        response = &mut execution => response.map(|outcome| (outcome, true)),
        _ = tokio::time::sleep_until(deadline.into()) => Ok((cubebot_protocol::terminal::Outcome::Uncertain, false)),
        _ = revoked(client, input) => Ok((cubebot_protocol::terminal::Outcome::Uncertain, false)),
    }
    // Dropping the socket requests process-tree cancellation. Effects remain uncertain
    // until the executor's durable outcome is reconciled; never manufacture rollback.
}
async fn revoked(client: &RuntimeClient, input: &Authorization) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if let Ok(Err(Error::Rejected | Error::Expired | Error::Protocol)) =
            tokio::time::timeout(Duration::from_secs(1), client.authorize_terminal(input)).await
        {
            return;
        }
    }
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

async fn model_step(
    provider: &cubebot_providers::OpenAi,
    grant: &InferenceGrant,
    input: &[serde_json::Value],
    deadline: Instant,
) -> Result<cubebot_providers::tools::Step, Outcome> {
    if Instant::now() >= deadline {
        return Err(waiting());
    }
    match tokio::time::timeout_at(
        deadline.into(),
        provider.tool_step(
            &grant.model,
            &grant.api_key,
            input,
            &[crate::tools::terminal_definition()],
        ),
    )
    .await
    {
        Ok(Ok(step)) => Ok(step),
        Ok(Err(error)) => Err(Outcome::Waiting {
            error: error.code().into(),
        }),
        Err(_) => Err(waiting()),
    }
}

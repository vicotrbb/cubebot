//! One turn: model steps and serial terminal calls, reported as ordered events. Tool calls are
//! journaled before they run and are never recovered by running them again.
use super::attempts::{Entry, Store};
use cubebot_protocol::{
    runtime::{Turn, TurnEvent},
    terminal::Outcome,
};
use serde_json::{Value, json};
use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc::UnboundedSender, watch};
use uuid::Uuid;

/// An event and, for tool events, the journal sequence its acknowledgement releases.
pub(super) type Event = (Entry, Option<u64>);

struct Run {
    turn: Uuid,
    next: u64,
    events: UnboundedSender<Event>,
}
impl Run {
    fn emit(&mut self, event: TurnEvent, journal: Option<u64>) -> Entry {
        let entry = Entry {
            turn: self.turn,
            sequence: self.next,
            event,
        };
        self.next += 1;
        // The receiver lives as long as the process; a closed channel means shutdown.
        let _ = self.events.send((entry.clone(), journal));
        entry
    }
    fn journaled(&mut self, store: &Mutex<Store>, event: TurnEvent) -> io::Result<()> {
        let entry = Entry {
            turn: self.turn,
            sequence: self.next,
            event,
        };
        let journal = store
            .lock()
            .map_err(|_| io::Error::other("journal lock poisoned"))?
            .record(&entry)?;
        self.emit(entry.event, Some(journal));
        Ok(())
    }
}

pub(super) async fn run(
    turn: Turn,
    events: UnboundedSender<Event>,
    store: Arc<Mutex<Store>>,
    mut cancel: watch::Receiver<bool>,
) {
    let mut run = Run {
        turn: turn.turn,
        next: turn.sequence,
        events,
    };
    let end = converse(&turn, &mut run, &store, &mut cancel).await;
    run.emit(end, None);
}

async fn converse(
    turn: &Turn,
    run: &mut Run,
    store: &Mutex<Store>,
    cancel: &mut watch::Receiver<bool>,
) -> TurnEvent {
    let provider = match configured_provider() {
        Ok(provider) => provider,
        Err(error) => return failed(error.code()),
    };
    let tools: Vec<Value> = if std::env::var("CUBEBOT_TERMINAL_ENABLED").as_deref() == Ok("1") {
        vec![crate::tools::terminal_definition()]
    } else {
        Vec::new()
    };
    let mut input = crate::context::assemble(turn);
    loop {
        let step = tokio::select! {
            step = model_step(&provider, turn, &input, &tools) => step,
            _ = cancel.wait_for(|cancelled| *cancelled) => return failed("cancelled"),
        };
        let step = match step {
            Ok(step) => step,
            Err(error) => return failed(error.code()),
        };
        run.emit(
            TurnEvent::Usage {
                input_tokens: step.usage.0,
                output_tokens: step.usage.1,
            },
            None,
        );
        // JSON escaping can push a 32 KiB reply past the ledger's 64 KiB event bound.
        if serde_json::to_vec(&step.text).map_or(true, |encoded| encoded.len() > 60 * 1024) {
            return failed("provider_invalid_response");
        }
        if !step.text.is_empty() {
            // shortcut: one delta per model step; stream provider tokens once providers do.
            run.emit(
                TurnEvent::TextDelta {
                    text: step.text.clone(),
                },
                None,
            );
        }
        let Some(call) = step.call else {
            return TurnEvent::Done {
                response: step.text,
            };
        };
        input.extend(step.output);
        let Some(command) = (!tools.is_empty())
            .then(|| crate::tools::terminal_arguments(&call.name, &call.arguments).ok())
            .flatten()
        else {
            return failed("provider_invalid_response");
        };
        let Ok(outcome) = execute(run, store, command.command, cancel).await else {
            return failed("journal_unavailable");
        };
        if *cancel.borrow() {
            return failed("cancelled");
        }
        input.push(json!({"type":"function_call_output","call_id":call.call_id,"output":serde_json::to_string(&outcome).unwrap_or_default()}));
    }
}

async fn execute(
    run: &mut Run,
    store: &Mutex<Store>,
    command: String,
    cancel: &mut watch::Receiver<bool>,
) -> io::Result<Outcome> {
    let id = Uuid::new_v4();
    let input = json!({ "command": command });
    run.journaled(
        store,
        TurnEvent::ToolCall {
            id,
            name: "terminal".into(),
            input,
        },
    )?;
    let workspace = std::env::var("CUBEBOT_WORKSPACE_DIRECTORY").unwrap_or_default();
    let outcome = match super::process::spawn(id, &command, &workspace) {
        Ok((mut child, path)) => {
            let status = tokio::select! {
                status = child.wait() => status.ok().map(|status| status.code()),
                _ = cancel.wait_for(|cancelled| *cancelled) => None,
            };
            super::process::finish(child, &path, status).await
        }
        Err(error) => Outcome::Exited {
            code: None,
            output: format!("The command did not start: {error}"),
        },
    };
    run.journaled(
        store,
        TurnEvent::ToolResult {
            id,
            outcome: outcome.clone(),
        },
    )?;
    Ok(outcome)
}

async fn model_step(
    provider: &cubebot_providers::OpenAi,
    turn: &Turn,
    input: &[Value],
    tools: &[Value],
) -> Result<cubebot_providers::tools::Step, cubebot_providers::Error> {
    match provider
        .tool_step(&turn.model, &turn.api_key, input, tools)
        .await
    {
        Err(error) if error.retryable() => {
            tokio::time::sleep(Duration::from_secs(1)).await;
            provider
                .tool_step(&turn.model, &turn.api_key, input, tools)
                .await
        }
        result => result,
    }
}

fn configured_provider() -> Result<cubebot_providers::OpenAi, cubebot_providers::Error> {
    match std::env::var("CUBEBOT_TEST_PROVIDER_ORIGIN") {
        Ok(origin) => cubebot_providers::OpenAi::for_loopback_tests(&origin),
        Err(std::env::VarError::NotPresent) => cubebot_providers::OpenAi::new(),
        Err(_) => Err(cubebot_providers::Error::Configuration),
    }
}

fn failed(code: &str) -> TurnEvent {
    TurnEvent::Error { code: code.into() }
}

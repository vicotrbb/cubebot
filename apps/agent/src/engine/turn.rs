//! One turn: streamed model steps and parallel tool calls, reported as ordered events. Tool
//! calls are journaled before they run and are never recovered by running them again.
use super::attempts::{Entry, Store};
use crate::{context::Context, memory::Memories, tools::Registry};
use cubebot_protocol::{
    runtime::{EVENT_BYTES, ProviderKind, Turn, TurnEvent},
    terminal::Outcome,
};
use cubebot_providers::{
    Anthropic, Error, Event as ProviderEvent, FunctionCall, Limits, Message, OpenAi, Provider,
    Reply, Request, Usage,
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc::UnboundedSender, watch};
use uuid::Uuid;

/// Model requests per step, counting the first.
const ATTEMPTS: u32 = 4;
/// A provider asking for a longer wait leaves the turn waiting rather than holding it.
const LONGEST_WAIT: Duration = Duration::from_secs(60);

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
    tools: Option<Arc<Registry>>,
    memories: Memories,
    cancel: watch::Receiver<bool>,
) {
    let tools = tools.as_deref();
    let memories = &memories;
    let mut run = Run {
        turn: turn.turn,
        next: turn.sequence,
        events,
    };
    let end = match turn.provider {
        ProviderKind::Openai => {
            let provider = configured(OpenAi::new, OpenAi::for_loopback_tests);
            start(provider, &turn, &mut run, &store, (tools, memories), cancel).await
        }
        ProviderKind::Anthropic => {
            let provider = configured(Anthropic::new, Anthropic::for_loopback_tests);
            start(provider, &turn, &mut run, &store, (tools, memories), cancel).await
        }
    };
    run.emit(end, None);
}

pub(super) fn configured<P>(
    new: fn() -> Result<P, Error>,
    loopback: fn(&str) -> Result<P, Error>,
) -> Result<P, Error> {
    match std::env::var("CUBEBOT_TEST_PROVIDER_ORIGIN") {
        Ok(origin) => loopback(&origin),
        Err(std::env::VarError::NotPresent) => new(),
        Err(_) => Err(Error::Configuration),
    }
}

async fn start<P: Provider>(
    provider: Result<P, Error>,
    turn: &Turn,
    run: &mut Run,
    store: &Mutex<Store>,
    (registry, memories): (Option<&Registry>, &Memories),
    mut cancel: watch::Receiver<bool>,
) -> TurnEvent {
    let provider = match provider {
        Ok(provider) => provider,
        Err(error) => return failed(error.code()),
    };
    let capabilities = provider.capabilities();
    if !capabilities.tool_calls {
        return failed("provider_rejected");
    }
    let mut tools = crate::memory::definitions();
    tools.extend(
        registry
            .into_iter()
            .flat_map(|registry| registry.definitions().to_vec()),
    );
    let limits = match capabilities.model_limits {
        true => tokio::select! {
            limits = provider.limits(&turn.model, &turn.api_key) => limits,
            _ = cancel.wait_for(|cancelled| *cancelled) => return failed("cancelled"),
        },
        false => Ok(Limits::default()),
    };
    let limits = match limits {
        Ok(limits) => limits,
        Err(error) => return failed(error.code()),
    };
    let Some(window) = turn.context_window.or(limits.context_window) else {
        return failed("provider_invalid_response");
    };
    let mut context = Context::assemble(turn, tools, window);
    Session {
        provider: &provider,
        turn,
        max_output_tokens: limits.max_output_tokens,
        run,
        store,
        tools: registry,
        memories,
        cancel,
    }
    .converse(&mut context)
    .await
}

struct Session<'a, P> {
    provider: &'a P,
    turn: &'a Turn,
    max_output_tokens: Option<u64>,
    run: &'a mut Run,
    store: &'a Mutex<Store>,
    tools: Option<&'a Registry>,
    memories: &'a Memories,
    cancel: watch::Receiver<bool>,
}

impl<P: Provider> Session<'_, P> {
    async fn converse(&mut self, context: &mut Context) -> TurnEvent {
        loop {
            let request = context.request(self.turn, self.max_output_tokens);
            let (reply, usage) = match self.step(&request, true).await {
                Ok(step) => step,
                Err(code) => return failed(code),
            };
            self.usage(usage);
            // Compaction waits for a safe point: after the tool result, or before the reply.
            let full = context.full(usage);
            let calls = reply.calls.clone();
            let response = reply.text.clone();
            context.messages.push(Message::Assistant(reply));
            if calls.is_empty() {
                if let Err(code) = self.extract(context).await {
                    return failed(code);
                }
                if full {
                    self.compact(context).await;
                }
                return done(response, self.memories.take());
            }
            let outputs = match self.call(&calls).await {
                Ok(outputs) => outputs,
                Err(code) => return failed(code),
            };
            for (call, output) in calls.into_iter().zip(outputs) {
                context.messages.push(Message::ToolResult {
                    call_id: call.call_id,
                    output,
                });
            }
            if full {
                self.compact(context).await;
            }
        }
    }

    async fn step(
        &mut self,
        request: &Request<'_>,
        stream_text: bool,
    ) -> Result<(Reply, Option<Usage>), &'static str> {
        let mut cancel = self.cancel.clone();
        tokio::select! {
            step = self.retrying(request, stream_text) => step.map_err(|error| error.code()),
            _ = cancel.wait_for(|cancelled| *cancelled) => Err("cancelled"),
        }
    }

    /// Retries transient failures with bounded backoff, only before any text streamed so
    /// nothing is emitted twice. The selected model and provider never change.
    async fn retrying(
        &mut self,
        request: &Request<'_>,
        stream_text: bool,
    ) -> Result<(Reply, Option<Usage>), Error> {
        let provider = self.provider;
        let mut attempt = 1;
        loop {
            let mut streamed = false;
            let mut events = std::pin::pin!(provider.stream(request));
            let error = loop {
                match events.next().await {
                    Some(Ok(ProviderEvent::Text(text))) => {
                        streamed = true;
                        if stream_text {
                            self.run.emit(TurnEvent::TextDelta { text }, None);
                        }
                    }
                    Some(Ok(ProviderEvent::Done { reply, usage })) => return Ok((reply, usage)),
                    Some(Err(error)) => break error,
                    None => break Error::Unavailable,
                }
            };
            match backoff(&error, attempt) {
                Some(delay) if !streamed => tokio::time::sleep(delay).await,
                _ => return Err(error),
            }
            attempt += 1;
        }
    }

    fn usage(&mut self, usage: Option<Usage>) {
        if let Some(usage) = usage {
            self.run.emit(
                TurnEvent::Usage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                },
                None,
            );
        }
    }

    /// Journals every call before any starts, runs them in parallel and journals each result as
    /// it arrives. Outputs come back in call order.
    async fn call(&mut self, calls: &[FunctionCall]) -> Result<Vec<String>, &'static str> {
        let prepared = prepare(self.tools, self.memories, calls);
        let mut running = FuturesUnordered::new();
        let mut unfinished = Vec::new();
        for (index, (id, name, input, call)) in prepared.into_iter().enumerate() {
            self.journal(TurnEvent::ToolCall { id, name, input })?;
            unfinished.push(id);
            running.push(async move { (index, id, call.await) });
        }
        let mut outputs = vec![String::new(); calls.len()];
        let mut cancel = self.cancel.clone();
        while !running.is_empty() {
            tokio::select! {
                Some((index, id, outcome)) = running.next() => {
                    unfinished.retain(|other| *other != id);
                    outputs[index] = serde_json::to_string(&outcome).unwrap_or_default();
                    self.journal(TurnEvent::ToolResult { id, outcome })?;
                }
                _ = cancel.wait_for(|cancelled| *cancelled) => break,
            }
        }
        // Dropping the unfinished calls kills their process groups before their output is read.
        drop(running);
        for id in unfinished {
            let output = self
                .tools
                .map(|tools| tools.preview(id))
                .unwrap_or_default();
            let outcome = Outcome::Stopped {
                reason: "cancelled".into(),
                output,
            };
            self.journal(TurnEvent::ToolResult { id, outcome })?;
        }
        if *cancel.borrow() {
            return Err("cancelled");
        }
        Ok(outputs)
    }

    fn journal(&mut self, event: TurnEvent) -> Result<(), &'static str> {
        self.run
            .journaled(self.store, event)
            .map_err(|_| "journal_unavailable")
    }

    /// Stages facts the turn did not save. Only cancellation fails the turn; otherwise it
    /// completes without them.
    async fn extract(&mut self, context: &Context) -> Result<(), &'static str> {
        let Some(prompt) = self.memories.extraction() else {
            return Ok(());
        };
        let mut messages = context.messages.clone();
        messages.push(Message::User(prompt));
        let request = Request {
            messages: &messages,
            tools_disabled: true,
            ..context.request(self.turn, self.max_output_tokens)
        };
        match self.step(&request, false).await {
            Ok((reply, usage)) => {
                self.usage(usage);
                self.memories.extracted(&reply.text);
                Ok(())
            }
            Err("cancelled") => Err("cancelled"),
            Err(_) => {
                eprintln!("Memory extraction failed; the turn completes without it");
                Ok(())
            }
        }
    }

    /// Summarizes with the same model and replaces the context only with a valid summary.
    async fn compact(&mut self, context: &mut Context) {
        let messages = context.summarizing();
        let request = Request {
            messages: &messages,
            tools_disabled: true,
            ..context.request(self.turn, self.max_output_tokens)
        };
        let Ok((reply, usage)) = self.step(&request, false).await else {
            return keep_context();
        };
        self.usage(usage);
        let summary = TurnEvent::Summary {
            text: reply.text.clone(),
        };
        if !reply.calls.is_empty() || !fits(&summary) {
            return keep_context();
        }
        self.run.emit(summary, None);
        context.compacted(&reply.text, &self.turn.message);
    }
}

/// The previous context stays valid; the next safe point tries again.
fn keep_context() {
    eprintln!("Context compaction failed; keeping the previous context");
}

/// The delay before another attempt; `None` once the failure is not transient, attempts are
/// spent, or the provider asks for a longer wait than the turn holds on for.
fn backoff(error: &Error, attempt: u32) -> Option<Duration> {
    if !error.retryable() || attempt >= ATTEMPTS {
        return None;
    }
    // Jitter keeps agents sharing a connection from retrying in lockstep.
    let jitter = Duration::from_millis(u64::from(Uuid::new_v4().as_bytes()[0]) * 2);
    let delay = error
        .retry_after()
        .unwrap_or(Duration::from_secs(1 << (attempt - 1)))
        + jitter;
    (delay <= LONGEST_WAIT).then_some(delay)
}

fn fits(event: &TurnEvent) -> bool {
    serde_json::to_vec(event).is_ok_and(|encoded| encoded.len() <= EVENT_BYTES)
}

fn done(response: String, memories: Vec<cubebot_protocol::runtime::MemoryChange>) -> TurnEvent {
    let done = TurnEvent::Done { response, memories };
    if fits(&done) {
        done
    } else {
        failed("provider_invalid_response")
    }
}

/// Every call with its arguments and unstarted run. A call that cannot run is still recorded,
/// with unparsable arguments kept as text, and answers the model with why, so it can correct
/// the call while the rest of the response runs.
#[allow(clippy::type_complexity)]
fn prepare<'a>(
    tools: Option<&'a Registry>,
    memories: &'a Memories,
    calls: &[FunctionCall],
) -> Vec<(Uuid, String, serde_json::Value, crate::tools::Running<'a>)> {
    calls
        .iter()
        .map(|call| {
            let id = Uuid::new_v4();
            let parsed = serde_json::from_str(&call.arguments);
            let running = match &parsed {
                Ok(input) => memories.call(&call.name, input).unwrap_or_else(|| {
                    tools
                        .ok_or("no tool has this name")
                        .and_then(|tools| tools.call(&call.name, id, input))
                }),
                Err(_) => Err("arguments are not valid JSON"),
            };
            let running = running.unwrap_or_else(|reason| {
                Box::pin(async move {
                    Outcome::Exited {
                        code: None,
                        output: format!("The call was not run: {reason}."),
                    }
                })
            });
            let input = parsed.unwrap_or_else(|_| call.arguments.clone().into());
            (id, call.name.clone(), input, running)
        })
        .collect()
}

fn failed(code: &str) -> TurnEvent {
    TurnEvent::Error { code: code.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_honours_retry_after_and_stays_bounded() {
        let transient = Error::Transient(None);
        let first = backoff(&transient, 1).unwrap();
        assert!(first >= Duration::from_secs(1) && first < Duration::from_millis(1600));
        assert!(backoff(&transient, 3).unwrap() >= Duration::from_secs(4));
        assert!(backoff(&transient, ATTEMPTS).is_none());
        let hinted = backoff(&Error::Transient(Some(Duration::from_secs(7))), 1).unwrap();
        assert!(hinted >= Duration::from_secs(7) && hinted < Duration::from_secs(8));
        assert!(backoff(&Error::Transient(Some(Duration::from_secs(600))), 1).is_none());
        for error in [Error::Rejected, Error::Unavailable, Error::InvalidResponse] {
            assert!(backoff(&error, 1).is_none());
        }
    }

    #[test]
    fn a_response_past_the_ledger_bound_is_refused() {
        assert_eq!(
            done("ok".into(), Vec::new()),
            TurnEvent::Done {
                response: "ok".into(),
                memories: Vec::new(),
            }
        );
        assert_eq!(
            done("x".repeat(EVENT_BYTES), Vec::new()),
            failed("provider_invalid_response")
        );
    }
}

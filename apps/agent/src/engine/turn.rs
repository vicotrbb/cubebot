use super::attempts::{Entry, Store};
use crate::{
    collaboration::Collaboration, context::Context, memory::Memories, skills::Skills,
    tools::Registry,
};
use cubebot_protocol::{
    runtime::{EVENT_BYTES, ProviderKind, Turn, TurnEvent},
    terminal::Outcome,
};
use cubebot_providers::{
    Anthropic, Error, Event as ProviderEvent, FunctionCall, Gemini, Grok, Limits, Message, OpenAi,
    OpenRouter, Provider, Reply, Request, Usage,
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc::UnboundedSender, watch};
use tracing::Instrument;
use uuid::Uuid;

const ATTEMPTS: u32 = 4;
const LONGEST_WAIT: Duration = Duration::from_secs(60);
/// A subscription's token is renewed this close to expiry; the orchestrator may renew it from then.
const RENEW_WITHIN: i64 = 5 * 60;

pub(super) type Event = (Entry, Option<u64>);

struct Run {
    turn: Uuid,
    next: u64,
    events: UnboundedSender<Event>,
    open: u64,
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
        let recorded = store
            .lock()
            .map_err(|_| io::Error::other("journal lock poisoned"))
            .and_then(|mut store| store.record(&entry));
        let journal = match recorded {
            Ok(journal) => journal,
            Err(error) => {
                // Recovery reports each open call as uncertain right after the last journaled entry; later events must not take those sequences.
                self.next += self.open;
                return Err(error);
            }
        };
        match entry.event {
            TurnEvent::ToolCall { .. } => self.open += 1,
            _ => self.open = self.open.saturating_sub(1),
        }
        self.emit(entry.event, Some(journal));
        Ok(())
    }
}

#[tracing::instrument(skip_all, fields(turn = %turn.turn))]
pub(super) async fn run(
    mut turn: Turn,
    events: UnboundedSender<Event>,
    store: Arc<Mutex<Store>>,
    tools: Option<Arc<Registry>>,
    (memories, skills, collaboration): (Memories, Skills, Collaboration),
    cancel: watch::Receiver<bool>,
) {
    let tools = tools.as_deref();
    let knowledge = (&memories, &skills, &collaboration);
    let mut run = Run {
        turn: turn.turn,
        next: turn.sequence,
        events,
        open: 0,
    };
    if turn.command.is_some() {
        let ended = match command(&mut turn, &mut run, &store, tools, cancel.clone()).await {
            Ok(Some(answer)) => Some(done(answer, Vec::new())),
            Ok(None) => None,
            Err(code) => Some(failed(code)),
        };
        if let Some(end) = ended {
            run.emit(end, None);
            return;
        }
    }
    let end = match turn.provider {
        ProviderKind::Openai => {
            let provider = configured(OpenAi::new, OpenAi::for_loopback_tests);
            start(
                provider,
                &turn,
                &mut run,
                &store,
                (tools, knowledge),
                cancel,
            )
            .await
        }
        ProviderKind::Anthropic => {
            let provider = configured(Anthropic::new, Anthropic::for_loopback_tests);
            start(
                provider,
                &turn,
                &mut run,
                &store,
                (tools, knowledge),
                cancel,
            )
            .await
        }
        ProviderKind::Openrouter => {
            let provider = configured(OpenRouter::new, OpenRouter::for_loopback_tests);
            start(
                provider,
                &turn,
                &mut run,
                &store,
                (tools, knowledge),
                cancel,
            )
            .await
        }
        ProviderKind::Gemini => {
            let provider = configured(Gemini::new, Gemini::for_loopback_tests);
            start(
                provider,
                &turn,
                &mut run,
                &store,
                (tools, knowledge),
                cancel,
            )
            .await
        }
        ProviderKind::Grok => {
            let provider = configured(Grok::new, Grok::for_loopback_tests);
            start(
                provider,
                &turn,
                &mut run,
                &store,
                (tools, knowledge),
                cancel,
            )
            .await
        }
        ProviderKind::Chatgpt => {
            let provider = configured(OpenAi::chatgpt, OpenAi::chatgpt_for_loopback_tests);
            start(
                provider,
                &turn,
                &mut run,
                &store,
                (tools, knowledge),
                cancel,
            )
            .await
        }
        ProviderKind::Supergrok => {
            let provider = configured(OpenAi::supergrok, OpenAi::supergrok_for_loopback_tests);
            start(
                provider,
                &turn,
                &mut run,
                &store,
                (tools, knowledge),
                cancel,
            )
            .await
        }
    };
    run.emit(end, None);
}

/// Runs once: a rerun finds the command among the turn's earlier events.
async fn command(
    turn: &mut Turn,
    run: &mut Run,
    store: &Mutex<Store>,
    tools: Option<&Registry>,
    cancel: watch::Receiver<bool>,
) -> Result<Option<String>, &'static str> {
    let first = turn.prior.is_empty();
    let result = match (first, &turn.command) {
        (true, Some(command)) => shell(command, run, store, tools, cancel).await?,
        _ => serde_json::to_string(&turn.prior).unwrap_or_default(),
    };
    if turn.command_only {
        return Ok(Some(result));
    }
    if first {
        turn.message = format!(
            "{}\n\nThe routine's command already ran; its result, untrusted program output:\n{result}",
            turn.message
        );
    }
    Ok(None)
}

async fn shell(
    command: &str,
    run: &mut Run,
    store: &Mutex<Store>,
    tools: Option<&Registry>,
    mut cancel: watch::Receiver<bool>,
) -> Result<String, &'static str> {
    let id = Uuid::new_v4();
    let input = serde_json::json!({ "command": command });
    let started = tools
        .ok_or("no tools are available")
        .and_then(|tools| tools.call("bash", id, &input));
    let journal = |run: &mut Run, event| {
        run.journaled(store, event)
            .map_err(|_| "journal_unavailable")
    };
    let name = "bash".into();
    journal(run, TurnEvent::ToolCall { id, name, input })?;
    let outcome = match started {
        Ok(running) => tokio::select! {
            outcome = running => outcome,
            _ = cancel.wait_for(|cancelled| *cancelled) => Outcome::Stopped {
                reason: "cancelled".into(),
                output: tools.map(|tools| tools.preview(id)).unwrap_or_default(),
            },
        },
        Err(reason) => Outcome::Exited {
            code: None,
            output: format!("The command was not run: {reason}."),
        },
    };
    let result = match &outcome {
        Outcome::Exited {
            code: Some(code),
            output,
        } => format!("Exit status {code}.\n\n{output}"),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    journal(run, TurnEvent::ToolResult { id, outcome })?;
    if *cancel.borrow() {
        return Err("cancelled");
    }
    Ok(result)
}

pub(super) fn configured<P>(
    new: fn() -> Result<P, Error>,
    loopback: fn(&str) -> Result<P, Error>,
) -> Result<P, Error> {
    match std::env::var("CUBEBOT_TEST_PROVIDER_ORIGIN") {
        Ok(origin) => loopback(&origin),
        Err(std::env::VarError::NotPresent) => new(),
        Err(error) => Err(Error::Configuration(Some(error.into()))),
    }
}

async fn start<P: Provider>(
    provider: Result<P, Error>,
    turn: &Turn,
    run: &mut Run,
    store: &Mutex<Store>,
    (registry, (memories, skills, collaboration)): (
        Option<&Registry>,
        (&Memories, &Skills, &Collaboration),
    ),
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
    if let Some(registry) = registry {
        registry.connect_mcp(&turn.mcp_servers).await;
    }
    let mut tools = crate::memory::definitions();
    tools.extend(crate::skills::definitions());
    tools.extend(crate::files::definitions(registry.is_some()));
    if turn.knowledge_base {
        tools.extend(crate::knowledge_base::definitions(registry.is_some()));
    }
    let desktop = !turn.task && registry.is_some_and(|registry| registry.computer("computer"));
    tools.extend(crate::collaboration::definitions(
        desktop,
        turn.broadcast,
        turn.in_task,
    ));
    // A task lane never drives the shared desktop; the main lane owns it.
    tools.extend(
        registry
            .into_iter()
            .flat_map(Registry::definitions)
            .filter(|tool| !(turn.task && tool.computer)),
    );
    let limits = if capabilities.model_limits {
        tokio::select! {
            limits = provider.limits(&turn.model, &turn.api_key) => limits,
            _ = cancel.wait_for(|cancelled| *cancelled) => return failed("cancelled"),
        }
    } else {
        Ok(Limits::default())
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
        key: (turn.api_key.clone(), turn.api_key_expires_at),
        max_output_tokens: limits.max_output_tokens,
        run,
        store,
        tools: registry,
        memories,
        skills,
        collaboration,
        cancel,
        desktop: None,
    }
    .converse(&mut context)
    .await
}

struct Session<'a, P> {
    provider: &'a P,
    turn: &'a Turn,
    /// The provider key and, for a subscription, when it expires.
    key: (String, Option<i64>),
    max_output_tokens: Option<u64>,
    run: &'a mut Run,
    store: &'a Mutex<Store>,
    tools: Option<&'a Registry>,
    memories: &'a Memories,
    skills: &'a Skills,
    collaboration: &'a Collaboration,
    cancel: watch::Receiver<bool>,
    /// Held from this turn's first desktop call to its end, so another lane cannot move the mouse between its steps.
    desktop: Option<tokio::sync::OwnedMutexGuard<()>>,
}

impl<P: Provider> Session<'_, P> {
    async fn converse(&mut self, context: &mut Context) -> TurnEvent {
        loop {
            self.steer(context);
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
            for (call, (output, image)) in calls.into_iter().zip(outputs) {
                context.messages.push(Message::ToolResult {
                    call_id: call.call_id,
                    output,
                    image,
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
        self.renew().await;
        let key = self.key.0.clone();
        let request = &Request {
            key: &key,
            ..*request
        };
        let mut cancel = self.cancel.clone();
        tokio::select! {
            step = self.retrying(request, stream_text) => step.map_err(|error| error.code()),
            _ = cancel.wait_for(|cancelled| *cancelled) => Err("cancelled"),
        }
    }

    /// A long turn outlives a subscription's token, so it takes the renewed one before it lapses.
    async fn renew(&mut self) {
        let Some(expires) = self.key.1 else {
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs() as i64);
        if expires - now > RENEW_WITHIN {
            return;
        }
        if let Some(renewed) = self.collaboration.renewed_key(self.turn.provider).await {
            self.key = renewed;
        }
    }

    /// Retries only before any text streamed, so nothing is emitted twice.
    async fn retrying(
        &mut self,
        request: &Request<'_>,
        stream_text: bool,
    ) -> Result<(Reply, Option<Usage>), Error> {
        let mut attempt = 1;
        loop {
            let mut streamed = false;
            let error = match self.request(request, stream_text, &mut streamed).await {
                Ok(done) => return Ok(done),
                Err(error) => error,
            };
            match backoff(&error, attempt) {
                Some(delay) if !streamed => tokio::time::sleep(delay).await,
                _ => return Err(error),
            }
            attempt += 1;
        }
    }

    #[tracing::instrument(skip_all, fields(provider = ?self.turn.provider, model = request.model))]
    async fn request(
        &mut self,
        request: &Request<'_>,
        stream_text: bool,
        streamed: &mut bool,
    ) -> Result<(Reply, Option<Usage>), Error> {
        let provider = self.provider;
        let mut events = std::pin::pin!(provider.stream(request));
        loop {
            match events.next().await {
                Some(Ok(ProviderEvent::Text(text))) => {
                    *streamed = true;
                    if stream_text {
                        self.run.emit(TurnEvent::TextDelta { text }, None);
                    }
                }
                Some(Ok(ProviderEvent::Done { reply, usage })) => return Ok((reply, usage)),
                Some(Err(error)) => return Err(error),
                None => return Err(Error::Unavailable),
            }
        }
    }

    fn steer(&mut self, context: &mut Context) {
        for (id, message) in self.collaboration.take_steering() {
            context.messages.push(Message::User(format!(
                "Steering from a person about this request; apply it to everything you do next: {message}"
            )));
            self.run.emit(TurnEvent::Steered { id, message }, None);
        }
    }

    fn usage(&mut self, usage: Option<Usage>) {
        if let Some(usage) = usage {
            self.run.emit(
                TurnEvent::Usage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    cached_input_tokens: usage.cached_input_tokens,
                    cost_usd: usage.cost_usd,
                },
                None,
            );
        }
    }

    async fn call(
        &mut self,
        calls: &[FunctionCall],
    ) -> Result<Vec<(String, Option<String>)>, &'static str> {
        self.hold_desktop(calls).await?;
        let prepared = prepare(
            self.tools,
            (self.memories, self.skills, self.collaboration),
            calls,
            self.turn.task,
        );
        let mut running = FuturesUnordered::new();
        let mut unfinished = Vec::new();
        for (index, (id, name, input, call)) in prepared.into_iter().enumerate() {
            let span = tracing::info_span!("tool_call", tool = %name, call = %id);
            self.journal(TurnEvent::ToolCall { id, name, input })?;
            unfinished.push(id);
            running.push(async move { (index, id, call.await) }.instrument(span));
        }
        let mut outputs = vec![(String::new(), None); calls.len()];
        let mut cancel = self.cancel.clone();
        while !running.is_empty() {
            tokio::select! {
                Some((index, id, outcome)) = running.next() => {
                    unfinished.retain(|other| *other != id);
                    outputs[index] = model_output(&outcome);
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

    async fn hold_desktop(&mut self, calls: &[FunctionCall]) -> Result<(), &'static str> {
        let Some(tools) = self.tools else {
            return Ok(());
        };
        // A delegated task is refused these calls, so it has nothing to wait for.
        if self.desktop.is_some()
            || self.turn.task
            || !calls.iter().any(|call| on_desktop(tools, call))
        {
            return Ok(());
        }
        let mut cancel = self.cancel.clone();
        tokio::select! {
            held = tools.desktop() => self.desktop = Some(held),
            _ = cancel.wait_for(|cancelled| *cancelled) => return Err("cancelled"),
        }
        Ok(())
    }

    fn journal(&mut self, event: TurnEvent) -> Result<(), &'static str> {
        self.run
            .journaled(self.store, event)
            .map_err(|_| "journal_unavailable")
    }

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

fn keep_context() {
    eprintln!("Context compaction failed; keeping the previous context");
}

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

/// A call that cannot run is still recorded and answers the model with why, so it can correct the call while the rest of the response runs.
#[allow(clippy::type_complexity)]
fn prepare<'a>(
    tools: Option<&'a Registry>,
    (memories, skills, collaboration): (&'a Memories, &'a Skills, &'a Collaboration),
    calls: &[FunctionCall],
    task: bool,
) -> Vec<(Uuid, String, serde_json::Value, crate::tools::Running<'a>)> {
    calls
        .iter()
        .map(|call| {
            let id = Uuid::new_v4();
            let parsed = serde_json::from_str(&call.arguments);
            let running = match &parsed {
                Ok(input) => lane(task, tools, &call.name, input).and_then(|()| {
                    memories
                        .call(&call.name, input)
                        .or_else(|| skills.call(&call.name, id, input, tools))
                        .or_else(|| collaboration.call(&call.name, id, input))
                        .unwrap_or_else(|| {
                            tools
                                .ok_or("no tool has this name")
                                .and_then(|tools| tools.call(&call.name, id, input))
                        })
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
            let input = crate::collaboration::redacted(&call.name, input);
            (id, call.name.clone(), input, running)
        })
        .collect()
}

fn on_desktop(tools: &Registry, call: &FunctionCall) -> bool {
    crate::collaboration::drives_desktop(&call.name)
        || tools.computer(&call.name)
        || (call.name.starts_with("browser_")
            && serde_json::from_str::<serde_json::Value>(&call.arguments)
                .is_ok_and(|input| input["session"] == "main"))
}

fn lane(
    task: bool,
    tools: Option<&Registry>,
    name: &str,
    input: &serde_json::Value,
) -> Result<(), &'static str> {
    if !task {
        return Ok(());
    }
    if crate::collaboration::drives_desktop(name) || tools.is_some_and(|tools| tools.computer(name))
    {
        return Err("a delegated task cannot control the desktop");
    }
    if name.starts_with("browser_") && input["session"] == "main" {
        return Err(
            "a delegated task cannot use the main browser session; name a session for the task",
        );
    }
    Ok(())
}

fn model_output(outcome: &Outcome) -> (String, Option<String>) {
    match outcome {
        Outcome::Screenshot { output, image } => (output.clone(), Some(image.clone())),
        _ => (serde_json::to_string(outcome).unwrap_or_default(), None),
    }
}

fn failed(code: &str) -> TurnEvent {
    TurnEvent::Error { code: code.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_that_use_what_lanes_share_hold_the_desktop() {
        let tools = Registry::with_desktop();
        let call = |name: &str, arguments: &str| FunctionCall {
            call_id: "call".into(),
            name: name.into(),
            arguments: arguments.into(),
        };
        for shared in [
            call("computer", "{}"),
            call("request_login", "{}"),
            call("vault_fill", "{}"),
            call("browser_read", r#"{"session":"main"}"#),
        ] {
            assert!(on_desktop(&tools, &shared), "{}", shared.name);
        }
        for own in [
            call("bash", "{}"),
            call("browser_read", r#"{"session":"research"}"#),
            call("browser_read", "not json"),
        ] {
            assert!(!on_desktop(&tools, &own), "{}", own.name);
        }
    }

    #[tokio::test]
    async fn a_task_lane_cannot_ask_for_a_login() {
        let knowledge = (
            Memories::new(None),
            Skills::new(None, Uuid::nil()),
            Collaboration::new(None, Uuid::nil(), Default::default()),
        );
        let call = FunctionCall {
            call_id: "call".into(),
            name: "request_login".into(),
            arguments: r#"{"site":"github.com","reason":"read"}"#.into(),
        };
        let prepared = prepare(
            None,
            (&knowledge.0, &knowledge.1, &knowledge.2),
            &[call],
            true,
        );
        let (_, _, _, running) = prepared.into_iter().next().unwrap();
        let Outcome::Exited { output, .. } = running.await else {
            panic!("a refusal is an exited outcome");
        };
        assert_eq!(
            output,
            "The call was not run: a delegated task cannot control the desktop."
        );
    }

    #[tokio::test]
    async fn a_task_lane_cannot_drive_the_main_browser_session() {
        let knowledge = (
            Memories::new(None),
            Skills::new(None, Uuid::nil()),
            Collaboration::new(None, Uuid::nil(), Default::default()),
        );
        let call = |session: &str| FunctionCall {
            call_id: "call".into(),
            name: "browser_navigate".into(),
            arguments: format!(r#"{{"session":"{session}","url":"https://example.com"}}"#),
        };
        let calls = [call("main"), call("research")];
        let knowledge = (&knowledge.0, &knowledge.1, &knowledge.2);
        let mut prepared = prepare(None, knowledge, &calls, true).into_iter();
        let outputs = [
            prepared.next().unwrap().3.await,
            prepared.next().unwrap().3.await,
        ];
        let [
            Outcome::Exited { output: main, .. },
            Outcome::Exited { output: own, .. },
        ] = outputs
        else {
            panic!("refusals are exited outcomes");
        };
        assert!(main.contains("main browser session"), "{main}");
        assert_eq!(own, "The call was not run: no tool has this name.");
    }

    #[test]
    fn a_failed_record_leaves_the_recovery_sequences_of_open_calls_free() {
        let root = std::env::temp_dir().join(format!("cubebot-run-{}", Uuid::new_v4()));
        let store = Mutex::new(Store::open(&root).unwrap());
        let (events, _received) = tokio::sync::mpsc::unbounded_channel();
        let mut run = Run {
            turn: Uuid::new_v4(),
            next: 1,
            events,
            open: 0,
        };
        let call = |name: String| TurnEvent::ToolCall {
            id: Uuid::new_v4(),
            name,
            input: serde_json::json!({}),
        };
        run.journaled(&store, call("bash".into())).unwrap();
        let oversized = call("x".repeat(crate::journal::MAX_PAYLOAD));
        assert!(run.journaled(&store, oversized).is_err());
        let error = run.emit(failed("journal_unavailable"), None);
        let pending = store.lock().unwrap().pending().unwrap();
        let uncertain = pending.last().unwrap().0.sequence;
        assert!(error.sequence > uncertain, "{} {uncertain}", error.sequence);
        std::fs::remove_dir_all(&root).unwrap();
    }

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
        for error in [
            Error::Rejected,
            Error::Unavailable,
            Error::InvalidResponse(None),
        ] {
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

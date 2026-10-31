//! Collaboration tools (ADR 0016) and steering (ADR 0071). `delegate` gives work to an agent
//! sharing a room and makes this turn yield when it ends, resuming as each outcome arrives; a
//! `send_message` request is not waited for, and its reply can be relayed back. Steering from a person enters the context before
//! the next model request, never in the middle of a tool call.
use crate::{control::MemoryClient, tools::Running};
use cubebot_protocol::{runtime::Collaborate, terminal::Outcome};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Steering delivered for the active turn and not yet taken into its context.
pub type Steering = Arc<Mutex<Vec<(Uuid, String)>>>;

pub struct Collaboration {
    client: Option<MemoryClient>,
    turn: Uuid,
    steering: Steering,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Send {
    agent: String,
    message: String,
    #[serde(default)]
    expects_reply: bool,
}

pub fn definitions() -> Vec<cubebot_providers::Tool> {
    let tool = |name: &str, description: &str, reply: bool| {
        let mut properties = json!({"agent":{"type":"string","description":"The other agent's exact name."},"message":{"type":"string","description":"Self-contained instructions or information; the recipient sees nothing else of this conversation."}});
        if reply {
            properties["expects_reply"] = json!({"type":"boolean","description":"True to receive the recipient's answer back here as a new message once it is done; false for information only."});
        }
        let required: Vec<&String> = properties.as_object().unwrap().keys().collect();
        cubebot_providers::Tool {
            name: name.into(),
            description: description.into(),
            parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
            strict: true,
            computer: false,
        }
    };
    vec![
        tool(
            "delegate",
            "Give part of this request to another agent that shares a room with you, with the result you expect back. After delegating, end your turn with a short note: you resume each time one delegated task settles, with every outcome so far, failures and their evidence included, and remain responsible for the final answer. Outcomes come back on their own: never delegate a request for the status or output of work already delegated. Mentioning an agent in text does nothing; only this tool assigns work.",
            false,
        ),
        tool(
            "send_message",
            "Send a request or information to another agent that shares a room with you, without waiting for it. With expects_reply its answer arrives later as a new message to you; otherwise it stays in its own conversation.",
            true,
        ),
    ]
}

impl Collaboration {
    pub fn new(client: Option<MemoryClient>, turn: Uuid, steering: Steering) -> Self {
        Self {
            client,
            turn,
            steering,
        }
    }

    /// `None` when `name` is not a collaboration tool. `id` is the journaled call's, so a
    /// resent request creates nothing new.
    pub fn call(
        &self,
        name: &str,
        id: Uuid,
        input: &Value,
    ) -> Option<Result<Running<'_>, &'static str>> {
        let wait = match name {
            "delegate" => true,
            "send_message" => false,
            _ => return None,
        };
        let parsed = Send::deserialize(input)
            .ok()
            .filter(|send| !send.agent.is_empty() && !send.message.trim().is_empty())
            .filter(|send| send.message.len() <= 16384 && send.agent.len() <= 255)
            .ok_or("agent and message are required; a message is at most 16 KiB");
        Some(parsed.map(|send| -> Running<'_> {
            let request = Collaborate {
                call: id,
                agent: send.agent,
                message: send.message,
                wait,
                expects_reply: send.expects_reply,
            };
            Box::pin(self.send(request))
        }))
    }

    async fn send(&self, request: Collaborate) -> Outcome {
        let sent = match &self.client {
            Some(client) => client.collaborate(self.turn, &request).await,
            None => Err(crate::control::Error::Unavailable),
        };
        let (code, output) = match sent {
            Ok(Ok(child)) if request.wait => (Some(0), format!("Delegated to {} as task {child}. End this turn now with a short note; you resume as each delegated task settles.", request.agent)),
            Ok(Ok(child)) => (Some(0), format!("Sent to {} as task {child}.", request.agent)),
            Ok(Err(reason)) => (Some(1), format!("Not sent: {reason}.")),
            Err(_) => (None, "The orchestrator did not answer; the work may or may not have been sent. Do not send it again under a different wording.".into()),
        };
        Outcome::Exited { code, output }
    }

    /// Steering waiting for the next safe point, oldest first.
    pub fn take_steering(&self) -> Vec<(Uuid, String)> {
        std::mem::take(&mut *self.steering.lock().unwrap())
    }
}

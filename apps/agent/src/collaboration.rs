mod routines;
mod tasks;
mod teams;
mod vault;
pub use teams::definitions as team_definitions;
pub use vault::redacted;

use crate::{control::MemoryClient, tools::Running};
use cubebot_protocol::{
    runtime::{Broadcast, Collaborate, RequestLogin},
    terminal::Outcome,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Login {
    site: String,
    reason: String,
}

pub fn drives_desktop(name: &str) -> bool {
    matches!(name, "request_login" | "vault_fill")
}

/// A task's turn omits `delegate` and `send_message`: their work would land in the other agent's own conversation.
pub fn definitions(desktop: bool, broadcast: bool, in_task: bool) -> Vec<cubebot_providers::Tool> {
    let tool = |name: &str, description: &str, reply: bool| {
        let mut properties = json!({"agent":{"type":"string","description":"The other agent's exact name."},"message":{"type":"string","description":"Self-contained instructions or information; the recipient sees nothing else of this conversation."}});
        if reply {
            properties["expects_reply"] = json!({"type":"boolean","description":"True to receive the recipient's answer back here as a new message once it is done; false for information only."});
        }
        cubebot_providers::Tool::strict(name, description, properties)
    };
    let login = cubebot_providers::Tool::strict(
        "request_login",
        "Ask a person to log in to a website for you on your desktop, when a login page or challenge blocks you. First call vault_list: when a login item's site is the page's domain or a parent of it (amazon.com.br serves www.amazon.com.br), fill its username and password with vault_fill and submit, and ask a person only if the site still blocks you, such as for a one-time code. Leave the login page open in the browser, then end your turn with a short note: the person takes control of the desktop, and a new message tells you when they finish or decline. Never ask for passwords in text.",
        json!({"site":{"type":"string","description":"The website's domain, such as github.com."},"reason":{"type":"string","description":"What you need the login for, shown to the person."}}),
    );
    let mut tools = vec![
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
    ];
    if in_task {
        tools = tasks::definitions();
    }
    tools.extend(desktop.then_some(login));
    tools.extend(broadcast.then(|| {
        cubebot_providers::Tool::strict(
            "broadcast",
            "Send one message to every other agent on the platform at once, each in its own conversation; their replies stay there and show in the console's Broadcast channel. Use it only for news or requests that truly concern every agent, at most once per turn; for one agent use send_message.",
            json!({"message":{"type":"string","description":"Self-contained text every agent can act on without this conversation."}}),
        )
    }));
    tools.extend(vault::definitions(desktop));
    tools.extend(routines::definitions());
    tools
}

impl Collaboration {
    pub fn new(client: Option<MemoryClient>, turn: Uuid, steering: Steering) -> Self {
        Self {
            client,
            turn,
            steering,
        }
    }

    /// The orchestrator's current key and its expiry, while the agent still uses `provider`.
    pub async fn renewed_key(
        &self,
        provider: cubebot_protocol::runtime::ProviderKind,
    ) -> Option<(String, Option<i64>)> {
        let key = self.client.as_ref()?.provider().await.ok()?;
        (key.provider == provider).then_some((key.api_key, key.api_key_expires_at))
    }

    /// `id` is the journaled call's, so a resent request creates nothing new.
    pub fn call(
        &self,
        name: &str,
        id: Uuid,
        input: &Value,
    ) -> Option<Result<Running<'_>, &'static str>> {
        let wait = match name {
            "delegate" => true,
            "send_message" => false,
            "request_login" => return Some(self.login(id, input)),
            "broadcast" => return Some(self.broadcast(id, input)),
            _ => {
                return self
                    .vault(name, input)
                    .or_else(|| self.routine(name, id, input))
                    .or_else(|| self.task(name, id, input))
                    .or_else(|| self.team(name, id, input));
            }
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

    fn login(&self, id: Uuid, input: &Value) -> Result<Running<'_>, &'static str> {
        let login = Login::deserialize(input)
            .ok()
            .filter(|login| !login.site.trim().is_empty() && !login.reason.trim().is_empty())
            .filter(|login| login.site.len() <= 255 && login.reason.len() <= 2048)
            .ok_or("site and reason are required; a site is at most 255 bytes, a reason 2 KiB")?;
        let request = RequestLogin {
            call: id,
            site: login.site,
            reason: login.reason,
        };
        Ok(Box::pin(async move {
            let sent = match &self.client {
                Some(client) => client.request_login(self.turn, &request).await,
                None => Err(crate::control::Error::Unavailable(None)),
            };
            let (code, output) = match sent {
                Ok(Ok(())) => (Some(0), format!("A person was asked to log in on {}. End this turn now with a short note; a new message tells you when they finish or decline.", request.site)),
                Ok(Err(reason)) => (Some(1), format!("Not requested: {reason}.")),
                Err(_) => (None, "The orchestrator did not answer; the request may or may not have been recorded.".into()),
            };
            Outcome::Exited { code, output }
        }))
    }

    fn broadcast(&self, id: Uuid, input: &Value) -> Result<Running<'_>, &'static str> {
        let message = input["message"]
            .as_str()
            .filter(|message| !message.trim().is_empty() && message.len() <= 16384)
            .ok_or("a message of at most 16 KiB is required")?;
        let request = Broadcast {
            call: id,
            message: message.into(),
        };
        Ok(Box::pin(async move {
            let sent = match &self.client {
                Some(client) => client.broadcast(self.turn, &request).await,
                None => Err(crate::control::Error::Unavailable(None)),
            };
            let (code, output) = match sent {
                Ok(Ok(agents)) => (Some(0), format!("Broadcast to {agents} agents.")),
                Ok(Err(reason)) => (Some(1), format!("Not broadcast: {reason}.")),
                Err(_) => (None, "The orchestrator did not answer; the broadcast may or may not have been sent. Do not send it again.".into()),
            };
            Outcome::Exited { code, output }
        }))
    }

    async fn send(&self, request: Collaborate) -> Outcome {
        let sent = match &self.client {
            Some(client) => client.collaborate(self.turn, &request).await,
            None => Err(crate::control::Error::Unavailable(None)),
        };
        let (code, output) = match sent {
            Ok(Ok(child)) if request.wait => (Some(0), format!("Delegated to {} as task {child}. End this turn now with a short note; you resume as each delegated task settles.", request.agent)),
            Ok(Ok(child)) => (Some(0), format!("Sent to {} as task {child}.", request.agent)),
            Ok(Err(reason)) => (Some(1), format!("Not sent: {reason}.")),
            Err(_) => (None, "The orchestrator did not answer; the work may or may not have been sent. Do not send it again under a different wording.".into()),
        };
        Outcome::Exited { code, output }
    }

    pub fn take_steering(&self) -> Vec<(Uuid, String)> {
        std::mem::take(&mut *self.steering.lock().unwrap())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn broadcast_is_offered_only_to_a_turn_that_may_broadcast() {
        let names = |broadcast| -> Vec<String> {
            super::definitions(false, broadcast, false)
                .into_iter()
                .map(|tool| tool.name)
                .collect()
        };
        assert!(names(true).contains(&"broadcast".to_owned()));
        assert!(!names(false).contains(&"broadcast".to_owned()));
        let login = super::definitions(true, false, false)
            .into_iter()
            .find(|tool| tool.name == "request_login")
            .unwrap();
        assert!(login.description.contains("First call vault_list"));
    }

    #[test]
    fn a_task_turn_talks_in_its_room_instead_of_other_agents_conversations() {
        let names: Vec<String> = super::definitions(false, false, true)
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert!(names.contains(&"task_message".to_owned()));
        assert!(names.contains(&"task_item".to_owned()));
        assert!(!names.contains(&"delegate".to_owned()));
        assert!(!names.contains(&"send_message".to_owned()));
    }
}

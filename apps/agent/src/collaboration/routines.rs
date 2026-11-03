//! Routine tools (docs/adr/agents.md#routines). The orchestrator validates each call and answers with the text the
//! model reads; the call's id makes a resent create or run make nothing new.
use super::Collaboration;
use crate::tools::Running;
use cubebot_protocol::{runtime::RoutineCall, terminal::Outcome};
use serde_json::{Value, json};
use uuid::Uuid;

const NAMES: [&str; 5] = [
    "routine_create",
    "routine_list",
    "routine_update",
    "routine_delete",
    "routine_run",
];

pub fn definitions() -> Vec<cubebot_providers::Tool> {
    let tool = cubebot_providers::Tool::strict;
    let nullable = |description: &str| json!({"type":["string","null"],"description":description});
    let routine = json!({"type":"object","properties":{
        "name":{"type":"string","description":"A short name."},
        "prompt":nullable("What to do each run, as a message to yourself; null to only run the command."),
        "command":nullable("A bash command run before the prompt, whose result you then read; null for none. Without a prompt the result is the whole answer."),
        "schedule":nullable("Five-field cron in UTC, such as \"0 9 * * 1-5\"; null to run only on demand."),
        "coalesce":{"type":"boolean","description":"True to skip a run while another is still waiting to start."},
        "enabled":{"type":"boolean","description":"False pauses the schedule."},
    },"required":["name","prompt","command","schedule","coalesce","enabled"],"additionalProperties":false});
    let id = json!({"type":"string","description":"The routine's id from routine_list."});
    vec![
        tool(
            "routine_create",
            "Create a routine for yourself: a prompt and/or a command that runs on a schedule or on demand, each run a new message in this conversation.",
            json!({"routine":routine}),
        ),
        tool(
            "routine_list",
            "List your routines with their schedule, next run, who manages each and its revision.",
            json!({}),
        ),
        tool(
            "routine_update",
            "Replace a routine you manage. A person's routine is theirs to change.",
            json!({"id":id,"expected_revision":{"type":"integer","description":"The revision from routine_list."},"routine":routine}),
        ),
        tool(
            "routine_delete",
            "Delete a routine you manage; its pending runs are cancelled.",
            json!({"id":id}),
        ),
        tool(
            "routine_run",
            "Queue a run of one of your routines; it starts after this turn.",
            json!({"id":id}),
        ),
    ]
}

impl Collaboration {
    /// `None` when `name` is not a routine tool.
    pub(super) fn routine(
        &self,
        name: &str,
        id: Uuid,
        input: &Value,
    ) -> Option<Result<Running<'_>, &'static str>> {
        if !NAMES.contains(&name) {
            return None;
        }
        // `routine_create` takes the routine alone, the others its id beside it.
        let input = match name {
            "routine_create" => input.get("routine").cloned().unwrap_or(Value::Null),
            _ => input.clone(),
        };
        let call = RoutineCall {
            call: id,
            tool: name.into(),
            input,
        };
        Some(Ok(Box::pin(async move {
            let sent = match &self.client {
                Some(client) => client.routine(self.turn, &call).await,
                None => Err(crate::control::Error::Unavailable(None)),
            };
            let (code, output) = match sent {
                Ok(Ok(output)) => (Some(0), output),
                Ok(Err(reason)) => (Some(1), format!("Refused: {reason}.")),
                Err(_) => (
                    None,
                    "The orchestrator did not answer; it may or may not have happened.".into(),
                ),
            };
            Outcome::Exited { code, output }
        })))
    }
}

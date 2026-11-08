use super::Collaboration;
use crate::tools::Running;
use cubebot_protocol::runtime::RoutineCall;
use serde_json::{Value, json};
use uuid::Uuid;

const NAMES: [&str; 3] = ["team_member", "team_remove", "team_update"];

/// Offered only to an agent that manages a team; the orchestrator checks which teams it reaches.
pub fn definitions() -> Vec<cubebot_providers::Tool> {
    let tool = cubebot_providers::Tool::strict;
    let team = json!({"type":"string","description":"The team's exact name: one you manage or one under it."});
    let keep = |description: &str| json!({"type":["string","null"],"description":description});
    vec![
        tool(
            "team_member",
            "Add an agent to a team or change its place there. The room and every member's instructions follow at once.",
            json!({
                "team":team,
                "agent":{"type":"string","description":"The agent's exact name."},
                "title":keep("A short title such as Backend engineer; required for a new member, null keeps the current one."),
                "role":keep("What this agent does in the team, as instructions to it; null keeps the current text."),
                "reports_to":keep("The exact name of the member it reports to; null keeps its current line, and a new member reports to the manager."),
                "manager":{"type":"boolean","description":"True makes it the team's manager, and the old manager then reports to it; only for a team under one you manage. False otherwise."},
            }),
        ),
        tool(
            "team_remove",
            "Remove an agent from a team. Whoever reported to it reports to its own manager. A team's manager stays until another member is made manager.",
            json!({"team":team,"agent":{"type":"string","description":"The member's exact name."}}),
        ),
        tool(
            "team_update",
            "Change a team's name, description or shared instructions, which every member's instructions carry.",
            json!({
                "team":team,
                "name":keep("The new name; null keeps it."),
                "description":keep("What the team is for, in a sentence or two; null keeps it."),
                "instructions":keep("The whole new instructions every member follows; null keeps them."),
            }),
        ),
    ]
}

impl Collaboration {
    pub(super) fn team(
        &self,
        name: &str,
        id: Uuid,
        input: &Value,
    ) -> Option<Result<Running<'_>, &'static str>> {
        if !NAMES.contains(&name) {
            return None;
        }
        Some(Ok(self.forward(RoutineCall {
            call: id,
            tool: name.into(),
            input: input.clone(),
        })))
    }
}

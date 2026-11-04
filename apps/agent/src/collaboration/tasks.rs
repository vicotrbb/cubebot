//! A task's tools (docs/adr/rooms.md#tasks-and-the-task-orchestrator). The orchestrator validates each call and answers with
//! the text the model reads; the call's id makes a resent message post nothing new.
use super::Collaboration;
use crate::tools::Running;
use cubebot_protocol::runtime::RoutineCall;
use serde_json::{Value, json};
use uuid::Uuid;

pub fn definitions() -> Vec<cubebot_providers::Tool> {
    let tool = cubebot_providers::Tool::strict;
    vec![
        tool(
            "task_item",
            "Mark one of your items in this task, so people can follow along.",
            json!({
                "item":{"type":"integer","description":"The item's number, without its #."},
                "status":{"type":"string","enum":["doing","done","skipped"]},
                "note":{"type":["string","null"],"description":"A few words on the result, or why it was skipped; null for none."},
            }),
        ),
        tool(
            "task_message",
            "Post to this task's room, which every agent on it and the people following it read. Agents named in `to` get a turn to answer in the room; you do not wait for them.",
            json!({
                "message":{"type":"string","description":"What to say; Markdown renders."},
                "to":{"type":"array","items":{"type":"string"},"description":"Exact names of agents who should act on this; empty to just inform the room."},
            }),
        ),
    ]
}

impl Collaboration {
    /// `None` when `name` is not a task tool.
    pub(super) fn task(
        &self,
        name: &str,
        id: Uuid,
        input: &Value,
    ) -> Option<Result<Running<'_>, &'static str>> {
        if !matches!(name, "task_item" | "task_message") {
            return None;
        }
        Some(Ok(self.forward(RoutineCall {
            call: id,
            tool: name.into(),
            input: input.clone(),
        })))
    }
}

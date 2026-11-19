use super::Collaboration;
use crate::tools::Running;
use cubebot_protocol::runtime::RoutineCall;
use serde_json::{Value, json};
use uuid::Uuid;

pub fn definitions() -> Vec<cubebot_providers::Tool> {
    let tool = cubebot_providers::Tool::strict;
    vec![tool(
        "task_item",
        "Mark one of your items in this task, so people can follow along.",
        json!({
            "item":{"type":"integer","description":"The item's number, without its #."},
            "status":{"type":"string","enum":["doing","done","skipped"]},
            "note":{"type":["string","null"],"description":"A few words on the result, or why it was skipped; null for none."},
        }),
    )]
}

impl Collaboration {
    pub(super) fn task(
        &self,
        name: &str,
        id: Uuid,
        input: &Value,
    ) -> Option<Result<Running<'_>, &'static str>> {
        if name != "task_item" {
            return None;
        }
        Some(Ok(self.forward(RoutineCall {
            call: id,
            tool: name.into(),
            input: input.clone(),
        })))
    }
}

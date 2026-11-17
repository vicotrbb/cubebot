use super::Collaboration;
use crate::tools::Running;
use cubebot_protocol::runtime::RoutineCall;
use serde_json::{Value, json};
use uuid::Uuid;

const NAMES: [&str; 4] = ["wake_set", "wake_list", "wake_cancel", "request_attention"];

pub fn definitions() -> Vec<cubebot_providers::Tool> {
    let tool = cubebot_providers::Tool::strict;
    let nullable =
        |kind: &str, description: &str| json!({"type":[kind,"null"],"description":description});
    vec![
        tool(
            "wake_set",
            "Wake yourself later: at the time you set, a message with your instructions arrives in this conversation and you act on it. Use it whenever you must wait, such as for a deploy, a reply or a date, instead of waiting in this turn. Give exactly one of at, delay_minutes or schedule. Unlike a routine, a wake is a note to yourself, not a stored job.",
            json!({
                "instructions":{"type":"string","description":"What to do when you wake, self-contained: you see only this message and the conversation."},
                "at":nullable("string","RFC 3339 time with an offset, such as 2026-11-18T09:00:00-03:00; null to use delay_minutes or schedule."),
                "delay_minutes":nullable("integer","Minutes from now, 1 to 527040; null to use at or schedule."),
                "schedule":nullable("string","Five-field cron in time_zone to wake on a regular basis, such as \"0 9 * * 1-5\"; null for a single wake."),
                "time_zone":nullable("string","IANA zone the schedule is read in, such as \"America/Sao_Paulo\"; null is UTC."),
            }),
        ),
        tool(
            "wake_list",
            "List your pending wakes with their id, instructions, schedule, next time and who set each.",
            json!({}),
        ),
        tool(
            "wake_cancel",
            "Cancel a wake you set. A person's wake is theirs to change.",
            json!({"id":{"type":"string","description":"The wake's id from wake_list."}}),
        ),
        tool(
            "request_attention",
            "Flag this reply as needing a person: a decision, an answer, an approval outside the console or an action only they can take. It shows under Needs you in their Activity with your summary until they answer. Do not use it for updates or finished work that needs nothing from them.",
            json!({"summary":{"type":"string","description":"One short line, at most 280 bytes, of what you need from them, such as \"Pick the launch date: Nov 20 or Nov 27?\"."}}),
        ),
    ]
}

impl Collaboration {
    pub(super) fn wake(
        &self,
        name: &str,
        id: Uuid,
        input: &Value,
    ) -> Option<Result<Running<'_>, &'static str>> {
        NAMES.contains(&name).then(|| {
            Ok(self.forward(RoutineCall {
                call: id,
                tool: name.into(),
                input: input.clone(),
            }))
        })
    }
}

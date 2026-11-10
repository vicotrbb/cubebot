use super::Collaboration;
use crate::tools::Running;
use cubebot_protocol::runtime::RoutineCall;
use serde_json::{Value, json};
use uuid::Uuid;

const NAMES: [&str; 5] = [
    "platform_list",
    "agent_create",
    "agent_update",
    "room_create",
    "team_create",
];

const ASKED: &str = " Nothing is created until a person approves it in the console; it is created as that person, with their access. After asking, end your turn with a short note: approved requests are made without telling you, and once a person settles every request from that turn, one message tells you any that were declined or could not be made. Requests may name agents that another pending request creates.";

/// Offered while the agent's creation capability is on; the orchestrator checks it again.
pub fn definitions() -> Vec<cubebot_providers::Tool> {
    let tool = cubebot_providers::Tool::strict;
    let text = |description: &str| json!({"type":"string","description":description});
    let maybe = |description: &str| json!({"type":["string","null"],"description":description});
    let one_of = |values: &[&str], description: &str| json!({"type":"string","enum":values,"description":description});
    let avatar = |description: &str| {
        json!({"type":["object","null"],"description":description,"properties":{
        "paint":one_of(&["orange","sage","paper","wood","sky","rose","lemon","plum","coral","mint","ocean","charcoal"], "The cube's color."),
        "eyes":one_of(&["round","wide","sleepy","dot","happy","wink","star","heart"], "The cube's eyes."),
        "accessory":one_of(&["none","cap","glasses","bow","antenna","crown","headphones","sprout","party_hat","monocle"], "What the cube wears."),
        "color":maybe("A custom #rrggbb color that replaces paint; null to use paint."),
    },"required":["paint","eyes","accessory","color"],"additionalProperties":false})
    };
    let member = json!({"type":"object","properties":{
        "agent":text("The agent's exact name."),
        "title":text("A short title such as Backend engineer."),
        "role":text("What this agent does in the team, as instructions to it."),
        "reports_to":maybe("The exact name of the member it reports to; null for the team's one manager."),
    },"required":["agent","title","role","reports_to"],"additionalProperties":false});
    vec![
        tool(
            "platform_list",
            "List the agents, rooms and teams on this platform that the person you are working for can see, with what each agent does and the skills it has. Read it before asking to create anything, so you reuse or extend what exists instead of duplicating it.",
            json!({}),
        ),
        tool(
            "agent_create",
            &format!(
                "Ask to create a new permanent agent on the platform, with its own conversation, computer and memory. It starts with the platform's default skills and the same connection access an agent a person creates gets.{ASKED}"
            ),
            json!({
                "name":text("A short name people will address it by, unique on the platform."),
                "instructions":text("Its standing instructions in Markdown, written to it in the second person: its one job, how it works, what it never does, and how it reports."),
                "avatar":avatar("Its cube, chosen to fit its role and to tell it apart from the other agents; null for the default orange cube."),
            }),
        ),
        tool(
            "agent_update",
            &format!(
                "Ask to rename an agent that exists or change how its cube looks.{}",
                ASKED.replace("created", "changed")
            ),
            json!({
                "agent":text("The agent's exact current name."),
                "name":maybe("Its new name, unique on the platform; null to keep the name."),
                "avatar":avatar("Its new cube; null to keep the cube."),
            }),
        ),
        tool(
            "room_create",
            &format!(
                "Ask to create a room where people and the agents in it talk together.{ASKED}"
            ),
            json!({
                "name":text("The room's name."),
                "agents":{"type":"array","items":{"type":"string"},"description":"The exact names of the agents in it."},
            }),
        ),
        tool(
            "team_create",
            &format!(
                "Ask to create a team of agents with one manager, a reporting line for each member and a room of its own.{ASKED}"
            ),
            json!({
                "name":text("The team's name, unique on the platform."),
                "description":text("What the team is for, in a sentence or two."),
                "instructions":text("Instructions every member follows, such as how they hand work to each other; may be empty."),
                "parent":maybe("The exact name of the team this one sits under; null for a top-level team."),
                "reports_to":maybe("With a parent: the exact name of the parent team's member this team's manager reports to; null otherwise."),
                "members":{"type":"array","items":member,"description":"At least one member; exactly one has reports_to null and is the manager."},
            }),
        ),
    ]
}

impl Collaboration {
    pub(super) fn creation(
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

//! Machine identity and the runtime channel. The agent credential lasts for its sandbox's lifetime.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bootstrap {
    pub token: String,
    pub instance: Uuid,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSession {
    pub token: String,
    pub agent: Uuid,
    pub attempt: Uuid,
    pub epoch: i64,
    pub version: i64,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeActor {
    pub agent: Uuid,
    pub attempt: Uuid,
    pub epoch: i64,
    pub version: i64,
}

/// Orchestrator to agent over `/api/runtime/channel`.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Delivery {
    Turn(Turn),
    /// The event is durable in the ledger.
    Ack {
        turn: Uuid,
        sequence: u64,
    },
    /// Cancels the turn; the ledger has already cancelled its delegated descendants.
    Cancel {
        turn: Uuid,
    },
    /// A person's correction for the active turn, incorporated before its next model request
    /// and reported as `TurnEvent::Steered`. Redelivered after a reconnect until reported.
    Steer {
        turn: Uuid,
        id: Uuid,
        message: String,
    },
    Drain {
        request_id: Uuid,
    },
    /// The credential no longer authorizes this sandbox; stop work.
    Revoked,
    /// No work is waiting. An agent still idle and reconciled after the delay may ask to dream.
    Idle {
        dream_after_seconds: u64,
    },
    /// The agent's wallpaper was set or reset; fetch `/api/runtime/wallpaper` and redraw it.
    WallpaperChanged,
}

/// Largest encoded turn event the ledger accepts. Responses are not otherwise bounded.
pub const EVENT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Openai,
    Anthropic,
}
impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        [Self::Openai, Self::Anthropic]
            .into_iter()
            .find(|kind| kind.as_str() == value)
    }
}

/// The one active turn. Provider material is current at delivery; never log it.
#[derive(Serialize, Deserialize)]
pub struct Turn {
    pub turn: Uuid,
    /// First event sequence for this run; earlier sequences belong to previous runs.
    pub sequence: u64,
    pub message: String,
    pub instructions: String,
    pub provider: ProviderKind,
    pub model: String,
    pub api_key: String,
    /// Configured model window in tokens; providers that report their limits may omit it.
    pub context_window: Option<u64>,
    /// The latest compaction summary, covering the conversation before `history`.
    pub summary: Option<String>,
    /// Every completed exchange since the summary, oldest first. Context assembly is the
    /// agent's job.
    pub history: Vec<Exchange>,
    /// Tool events from earlier runs of this turn.
    pub prior: Vec<TurnEvent>,
    /// Work this turn delegated, oldest first: settled children with their outcome, and open
    /// ones with their status only.
    #[serde(default)]
    pub delegations: Vec<Delegation>,
    /// A delegated task in a task lane (ADR 0005): its own context, beside the main lane, and
    /// no control of the shared desktop.
    #[serde(default)]
    pub task: bool,
}

/// A child's settled outcome. `evidence` holds its last tool events, so a failed child still
/// shows what it did; it is untrusted program output.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delegation {
    pub turn: Uuid,
    pub agent: String,
    /// `complete`, `failed` or `cancelled` once settled; otherwise still open.
    pub status: String,
    pub response: Option<String>,
    pub error: Option<String>,
    pub evidence: Vec<TurnEvent>,
}

/// `POST /api/runtime/turns/{turn}/messages`: work for another agent sharing a room. `call`
/// is the tool call's id, so a resend creates nothing new. With `wait` the sending turn yields
/// when it ends and resumes once every awaited child has settled.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collaborate {
    pub call: Uuid,
    pub agent: String,
    pub message: String,
    pub wait: bool,
    /// Without `wait`: relay the recipient's response back as a new turn once it completes.
    #[serde(default)]
    pub expects_reply: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collaborated {
    pub turn: Uuid,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exchange {
    pub message: String,
    pub response: String,
}

/// Agent to orchestrator over `/api/runtime/channel`.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Report {
    /// Ready for the next turn.
    Pull,
    Event {
        turn: Uuid,
        sequence: u64,
        event: TurnEvent,
    },
    Drained {
        request_id: Uuid,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TurnEvent {
    TextDelta {
        text: String,
    },
    ToolCall {
        id: Uuid,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        id: Uuid,
        outcome: crate::terminal::Outcome,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    /// Replaces the earlier context for later model requests; the source history remains.
    Summary {
        text: String,
    },
    /// A steering message entered the context.
    Steered {
        id: Uuid,
        message: String,
    },
    /// Memory changes staged during the turn commit with it, or not at all.
    Done {
        response: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        memories: Vec<MemoryChange>,
    },
    Error {
        code: String,
    },
}

/// A memory write staged by the agent. A revision or removal names the revision it read; a
/// newer one, such as a human edit, wins and the change is dropped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum MemoryChange {
    Remember {
        content: String,
    },
    Revise {
        memory: Uuid,
        expected_revision: i64,
        content: String,
    },
    Forget {
        memory: Uuid,
        expected_revision: i64,
    },
}

/// An admitted dream run, from `POST /api/runtime/dreams/{run}`. Changes go one at a time to
/// `POST /api/runtime/dreams/{run}/changes` as `MemoryChange`s; `PUT /api/runtime/dreams/{run}`
/// ends it with a `DreamEnd`. Provider material is current at admission; never log it.
#[derive(Serialize, Deserialize)]
pub struct Dream {
    pub provider: ProviderKind,
    pub model: String,
    pub api_key: String,
    /// The agent's most recently changed memories.
    pub memories: Vec<Memory>,
    /// The agent's latest completed exchanges, oldest first, each side cut to 2000 characters.
    #[serde(default)]
    pub turns: Vec<Exchange>,
    /// The agent's installed skills at their installed revisions. A proposed skill commits to
    /// `POST /api/runtime/dreams/{run}/skills` as a `SkillChange`.
    #[serde(default)]
    pub skills: Vec<SkillPackage>,
}

/// Largest skill package: the sum of its path and content bytes.
pub const SKILL_BYTES: usize = 64 * 1024;

/// One revision of a skill package: relative path to text content, with `SKILL.md` at its root.
/// `GET /api/runtime/skills/{name}` returns the agent's installed revision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillPackage {
    pub name: String,
    pub revision: i64,
    pub files: std::collections::BTreeMap<String, String>,
}

/// A dream's proposed skill: new without `expected_revision`, otherwise a revision of the skill
/// named in its `SKILL.md`. A stale revision or a taken name is dropped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillChange {
    pub expected_revision: Option<i64>,
    pub files: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DreamOutcome {
    Completed,
    /// Stopped for a channel message; changes committed before it stay.
    Yielded,
    Failed,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DreamEnd {
    pub outcome: DreamOutcome,
    pub report: String,
}

/// Largest dream report in bytes; escaped, it still fits a runtime request body.
pub const REPORT_BYTES: usize = 8 * 1024;

/// Largest memory content in bytes.
pub const MEMORY_BYTES: usize = 4096;

/// A memory search result over `GET /api/runtime/memories?query=`.
#[derive(Serialize, Deserialize)]
pub struct Memory {
    pub id: Uuid,
    pub revision: i64,
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_messages_round_trip_with_their_tags() {
        let report = Report::Event {
            turn: Uuid::nil(),
            sequence: 3,
            event: TurnEvent::Done {
                response: "ok".into(),
                memories: Vec::new(),
            },
        };
        let text = serde_json::to_string(&report).unwrap();
        assert!(text.contains(r#""type":"event""#) && text.contains(r#""kind":"done""#));
        assert!(!text.contains("memories"), "{text}");
        let staged: TurnEvent = serde_json::from_str(
            r#"{"kind":"done","response":"ok","memories":[{"op":"forget","memory":"00000000-0000-0000-0000-000000000000","expected_revision":2}]}"#,
        )
        .unwrap();
        assert!(matches!(staged, TurnEvent::Done { memories, .. } if memories.len() == 1));
        assert!(matches!(
            serde_json::from_str::<Report>(&text).unwrap(),
            Report::Event { sequence: 3, .. }
        ));
        let turn = Delivery::Turn(Turn {
            turn: Uuid::nil(),
            sequence: 1,
            message: "hi".into(),
            instructions: String::new(),
            provider: ProviderKind::Anthropic,
            model: "m".into(),
            api_key: "k".into(),
            context_window: None,
            summary: None,
            history: Vec::new(),
            prior: Vec::new(),
            delegations: Vec::new(),
            task: false,
        });
        let text = serde_json::to_string(&turn).unwrap();
        assert!(text.contains(r#""provider":"anthropic""#));
        assert!(matches!(
            serde_json::from_str::<Delivery>(&text).unwrap(),
            Delivery::Turn(Turn { sequence: 1, .. })
        ));
        assert_eq!(ProviderKind::parse("openai"), Some(ProviderKind::Openai));
        assert_eq!(ProviderKind::parse("gemini"), None);
        assert!(serde_json::from_str::<Report>(r#"{"type":"claim"}"#).is_err());
    }
}

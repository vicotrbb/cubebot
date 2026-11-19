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
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Delivery {
    Turn(Box<Turn>),
    Ack {
        turn: Uuid,
        sequence: u64,
    },
    /// Cancels the turn; the ledger has already cancelled its delegated descendants.
    Cancel {
        turn: Uuid,
    },
    /// Redelivered after a reconnect until reported as `TurnEvent::Steered`.
    Steer {
        turn: Uuid,
        id: Uuid,
        message: String,
    },
    Drain {
        request_id: Uuid,
    },
    Revoked,
    /// No work is waiting. An agent still idle and reconciled after the delay may ask to dream.
    Idle {
        dream_after_seconds: u64,
    },
    WallpaperChanged,
    /// Send the workspace for this backup, at low priority, to `/api/runtime/backups/{backup}/workspace`.
    Backup {
        backup: String,
    },
}

/// How an agent's extraction of its restored workspace went; `error` is absent when it succeeded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRestored {
    pub error: Option<String>,
}

/// Largest encoded turn event the ledger accepts. Responses are not otherwise bounded.
pub const EVENT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Openai,
    Anthropic,
    Openrouter,
    Gemini,
    Grok,
    /// A ChatGPT plan signed in through Sign in with ChatGPT; the key is a short-lived access token.
    Chatgpt,
    /// A SuperGrok or X Premium plan through the Grok CLI's sign-in; the key is a short-lived access token.
    Supergrok,
    /// MiniMax pay-as-you-go through its Anthropic-compatible API; `_cn` is the mainland China platform.
    Minimax,
    MinimaxCn,
    /// A MiniMax M Plan key: personal like a signed-in plan, though the key itself does not expire.
    MinimaxPlan,
    MinimaxPlanCn,
}
impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Openrouter => "openrouter",
            Self::Gemini => "gemini",
            Self::Grok => "grok",
            Self::Chatgpt => "chatgpt",
            Self::Supergrok => "supergrok",
            Self::Minimax => "minimax",
            Self::MinimaxCn => "minimax_cn",
            Self::MinimaxPlan => "minimax_plan",
            Self::MinimaxPlanCn => "minimax_plan_cn",
        }
    }
    /// A person's plan: personal, held in `provider_subscriptions` rather than as an installation key.
    pub fn subscription(self) -> bool {
        matches!(
            self,
            Self::Chatgpt | Self::Supergrok | Self::MinimaxPlan | Self::MinimaxPlanCn
        )
    }
    pub fn parse(value: &str) -> Option<Self> {
        [
            Self::Openai,
            Self::Anthropic,
            Self::Openrouter,
            Self::Gemini,
            Self::Grok,
            Self::Chatgpt,
            Self::Supergrok,
            Self::Minimax,
            Self::MinimaxCn,
            Self::MinimaxPlan,
            Self::MinimaxPlanCn,
        ]
        .into_iter()
        .find(|kind| kind.as_str() == value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}
impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        use Effort::*;
        [Minimal, Low, Medium, High, Xhigh, Max]
            .into_iter()
            .find(|effort| effort.as_str() == value)
    }
}

impl ProviderKind {
    pub fn efforts(self) -> &'static [Effort] {
        use Effort::*;
        match self {
            Self::Openai | Self::Chatgpt | Self::Openrouter => {
                &[Minimal, Low, Medium, High, Xhigh, Max]
            }
            Self::Anthropic => &[Low, Medium, High, Xhigh, Max],
            Self::Gemini => &[Minimal, Low, Medium, High, Xhigh],
            Self::Grok => &[Low, Medium, High, Xhigh],
            Self::Supergrok
            | Self::Minimax
            | Self::MinimaxCn
            | Self::MinimaxPlan
            | Self::MinimaxPlanCn => &[],
        }
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
    /// Unix seconds when a subscription's `api_key` expires; ask `GET /api/runtime/provider` before then.
    #[serde(default)]
    pub api_key_expires_at: Option<i64>,
    pub context_window: Option<u64>,
    #[serde(default)]
    pub effort: Option<Effort>,
    pub summary: Option<String>,
    pub history: Vec<Exchange>,
    pub prior: Vec<TurnEvent>,
    /// The turn belongs to a task's room (`task` below means a delegated lane); its reply stays in that room.
    #[serde(default)]
    pub in_task: bool,
    #[serde(default)]
    pub delegations: Vec<Delegation>,
    #[serde(default)]
    pub task: bool,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub command_only: bool,
    /// Credentials included; never log them.
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
    /// False for a turn that is itself a broadcast, so broadcasts never answer each other.
    #[serde(default)]
    pub broadcast: bool,
    #[serde(default)]
    pub knowledge_base: bool,
    /// Native connections the agent may use; their credentials are fetched when a tool needs one.
    #[serde(default)]
    pub connections: Vec<Connection>,
    /// The agent manages a team, so it gets the team tools.
    #[serde(default)]
    pub team_manager: bool,
    /// The agent may ask to create agents, rooms and teams, which a person then approves.
    #[serde(default)]
    pub creates: bool,
    #[serde(default)]
    pub attachments: Vec<TurnFile>,
    #[serde(default)]
    pub talk_only: bool,
    #[serde(default)]
    pub meeting: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnFile {
    pub file: Uuid,
    pub name: String,
    pub content_type: Option<String>,
    pub size: u64,
}

/// A native connection (docs/adr/identity.md#native-connectors) as one turn sees it: no secret.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub id: Uuid,
    /// The compiled-in connector, such as `google`.
    pub connector: String,
    /// Prefixes the connection's tools as `<slug>__<tool>`.
    pub slug: String,
    pub account: Option<String>,
    /// Settings that are not secret, such as a server address or the products switched on.
    pub config: serde_json::Value,
    /// `read`, `write` or `full`: the agent is offered the tools at or below it.
    pub level: String,
    #[serde(default)]
    pub disabled_tools: Vec<String>,
}

/// `GET /api/runtime/connections/{id}/credential`: short-lived where the vendor allows. The agent
/// runtime holds it in its own process; never log it or show it to the model.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionCredential {
    pub credential: serde_json::Value,
    /// Unix seconds; ask again before then.
    pub expires_at: Option<i64>,
}

/// `GET /api/runtime/provider`: the agent's current provider material. Never log it.
#[derive(Serialize, Deserialize)]
pub struct ProviderKey {
    pub provider: ProviderKind,
    pub api_key: String,
    pub api_key_expires_at: Option<i64>,
}

/// Names are unique letters, digits or hyphens, and prefix the server's tools as `<name>__<tool>`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpServer {
    Stdio {
        name: String,
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: std::collections::BTreeMap<String, String>,
    },
    Http {
        name: String,
        url: String,
        #[serde(default)]
        headers: std::collections::BTreeMap<String, String>,
    },
}

impl McpServer {
    pub fn name(&self) -> &str {
        match self {
            Self::Stdio { name, .. } | Self::Http { name, .. } => name,
        }
    }
}

/// `evidence` is untrusted program output.
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

/// `POST /api/runtime/turns/{turn}/routines`. `call` is the tool call's id, so a resent create or run makes nothing new.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutineCall {
    pub call: Uuid,
    pub tool: String,
    pub input: serde_json::Value,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutineAnswer {
    pub output: String,
}

/// `POST /api/runtime/turns/{turn}/attachments`. A refusal is 409 with the reason the model reads.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachFile {
    pub file: Uuid,
}

/// `POST /api/runtime/turns/{turn}/messages`. `call` is the tool call's id, so a resend creates nothing new.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collaborate {
    pub call: Uuid,
    pub agent: String,
    pub message: String,
    pub wait: bool,
    #[serde(default)]
    pub expects_reply: bool,
}
/// `POST /api/runtime/turns/{turn}/logins`. `call` is the tool call's id, so a resend records nothing new; a refusal is 409.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestLogin {
    pub call: Uuid,
    pub site: String,
    pub reason: String,
}
/// `POST /api/runtime/turns/{turn}/broadcasts`. `call` is the tool call's id, so a resend sends nothing new; a refusal is 409.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Broadcast {
    pub call: Uuid,
    pub message: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Broadcasted {
    pub agents: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeetingScreen {
    pub share: bool,
}

/// A vault item (docs/adr/identity.md#a-vault-the-desktop-arbiter-fills) from `GET /api/runtime/vault`: never its secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct VaultItem {
    pub id: Uuid,
    pub kind: VaultKind,
    pub name: String,
    #[serde(default)]
    pub site: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last4: Option<String>,
    /// The agent's own item; otherwise the installation's.
    pub own: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum VaultKind {
    /// A site, a username and its password as the secret.
    Login,
    Secret,
    Card,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum VaultField {
    Username,
    /// A login's password or a secret's value.
    Secret,
    Holder,
    Number,
    Expiry,
    ExpiryMonth,
    ExpiryYear,
    Cvc,
}

impl VaultField {
    pub fn card(self) -> bool {
        !matches!(self, Self::Username | Self::Secret)
    }
}
/// `POST /api/runtime/vault/fills`. A refusal is 409 with the reason the model reads.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultFill {
    pub item: Uuid,
    pub field: VaultField,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<Uuid>,
}
/// `POST /api/runtime/vault`. A refusal is 409 with the reason the model reads.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultPut {
    pub kind: VaultKind,
    pub name: String,
    #[serde(default)]
    pub site: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    pub secret: String,
}
/// A shared file or folder (docs/adr/memory.md#orchestrator-managed-file-storage-on-a-persistent-volume and docs/adr/memory.md#native-cubebot-storage-api), as the console and `/api/runtime/files` list
/// it. Uploads stream the raw bytes in the request body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FileEntry {
    pub id: Uuid,
    /// The folder holding it; its owner's top level when absent.
    pub parent: Option<Uuid>,
    pub name: String,
    /// A folder has no size, digest or content type.
    pub folder: bool,
    pub size: Option<i64>,
    pub content_type: Option<String>,
    /// Lowercase hex of the stored bytes' SHA-256, computed by the orchestrator.
    pub sha256: Option<String>,
    /// The person who owns it, when a person does.
    pub owner_user: Option<Uuid>,
    /// The agent that owns it, when an agent does.
    pub owner_agent: Option<Uuid>,
    /// What the caller may do with it.
    pub access: FileAccess,
    pub updated_at: String,
}
/// `write` adds uploading, moving, renaming and deleting to `read`; `manage` adds sharing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum FileAccess {
    Read,
    Write,
    Manage,
}
/// A share of a file, and for a folder everything under it, with a person by login, a team or an
/// agent by name. `POST /api/runtime/files/{id}/shares` adds or changes one; a refusal is 400
/// with the reason the model reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct FileShare {
    pub kind: ShareKind,
    pub name: String,
    pub permission: SharePermission,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ShareKind {
    User,
    /// A group of people; agents built before 2026-11-08 say `team`.
    #[serde(alias = "team")]
    Group,
    Agent,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SharePermission {
    Read,
    Write,
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

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Report {
    Pull,
    Event {
        turn: Uuid,
        sequence: u64,
        event: TurnEvent,
    },
    Drained {
        request_id: Uuid,
    },
    Vitals(Vitals),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct Vitals {
    pub cpu_percent: Option<f64>,
    pub cpus: u32,
    pub memory: Option<Usage>,
    pub disk: Option<Usage>,
    pub network: Option<Throughput>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub used: u64,
    pub total: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct Throughput {
    pub received: u64,
    pub sent: u64,
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
        #[serde(default)]
        cached_input_tokens: u64,
        #[serde(default)]
        cost_usd: Option<f64>,
        #[serde(default)]
        stage: ModelStage,
    },
    Review {
        outcome: ReviewOutcome,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ModelStage {
    #[default]
    Reply,
    Review,
    Memory,
    Summary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReviewOutcome {
    Asked,
    Complete,
    Revised,
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

/// `POST /api/runtime/dreams/{run}`; changes go to `.../changes`, `PUT` ends it. Provider material is current at admission; never log it.
#[derive(Serialize, Deserialize)]
pub struct Dream {
    pub provider: ProviderKind,
    pub model: String,
    pub api_key: String,
    pub memories: Vec<Memory>,
    #[serde(default)]
    pub turns: Vec<Exchange>,
    /// A proposed skill commits to `POST /api/runtime/dreams/{run}/skills` as a `SkillChange`.
    #[serde(default)]
    pub skills: Vec<SkillPackage>,
    #[serde(default)]
    pub library: Vec<LibrarySkill>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub personality: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibrarySkill {
    pub name: String,
    pub description: String,
}

/// Largest skill package: the sum of its path and content bytes.
pub const SKILL_BYTES: usize = 64 * 1024;

/// Relative path to text content, with `SKILL.md` at its root.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillPackage {
    pub name: String,
    pub revision: i64,
    pub files: std::collections::BTreeMap<String, String>,
}

/// New without `expected_revision`; a stale revision or a taken name is dropped.
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
    Yielded,
    Failed,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DreamEnd {
    pub outcome: DreamOutcome,
    pub report: String,
    #[serde(default)]
    pub usage: Option<TokenUsage>,
    #[serde(default)]
    pub personality: Option<String>,
    #[serde(default)]
    pub outreach: Option<Outreach>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outreach {
    pub after_minutes: u32,
    pub intent: String,
}

/// Provider-reported tokens for work outside a turn; a turn reports them as `TurnEvent::Usage`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub cost_usd: Option<f64>,
}

/// Largest dream report in bytes; escaped, it still fits a runtime request body.
pub const REPORT_BYTES: usize = 8 * 1024;

pub const MEMORY_BYTES: usize = 4096;

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
    fn a_vitals_report_round_trips_with_its_tag() {
        let vitals = Report::Vitals(Vitals {
            cpu_percent: Some(12.5),
            cpus: 2,
            memory: Some(Usage { used: 1, total: 2 }),
            disk: None,
            network: Some(Throughput {
                received: 3,
                sent: 4,
            }),
        });
        let text = serde_json::to_string(&vitals).unwrap();
        assert!(
            text.starts_with(r#"{"type":"vitals","cpu_percent":12.5"#),
            "{text}"
        );
        assert!(matches!(
            serde_json::from_str::<Report>(&text).unwrap(),
            Report::Vitals(Vitals { cpus: 2, .. })
        ));
    }

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
        let turn = Delivery::Turn(Box::new(Turn {
            turn: Uuid::nil(),
            sequence: 1,
            message: "hi".into(),
            instructions: String::new(),
            provider: ProviderKind::Anthropic,
            model: "m".into(),
            api_key: "k".into(),
            api_key_expires_at: None,
            context_window: None,
            effort: None,
            summary: None,
            history: Vec::new(),
            prior: Vec::new(),
            delegations: Vec::new(),
            task: false,
            command: None,
            command_only: false,
            mcp_servers: vec![McpServer::Http {
                name: "docs".into(),
                url: "https://mcp.example".into(),
                headers: [("Authorization".into(), "Bearer k".into())].into(),
            }],
            broadcast: false,
            knowledge_base: false,
            team_manager: false,
            creates: false,
            attachments: Vec::new(),
            connections: Vec::new(),
            in_task: false,
            talk_only: false,
            meeting: None,
        }));
        let text = serde_json::to_string(&turn).unwrap();
        assert!(text.contains(r#""provider":"anthropic""#));
        assert!(matches!(
            serde_json::from_str::<Delivery>(&text).unwrap(),
            Delivery::Turn(turn) if turn.sequence == 1 && turn.mcp_servers[0].name() == "docs"
        ));
        assert_eq!(ProviderKind::parse("openai"), Some(ProviderKind::Openai));
        assert!(serde_json::from_str::<Report>(r#"{"type":"claim"}"#).is_err());
    }

    #[test]
    fn gemini_and_grok_are_provider_kinds_and_others_are_not() {
        assert_eq!(ProviderKind::parse("gemini"), Some(ProviderKind::Gemini));
        assert_eq!(ProviderKind::parse("grok"), Some(ProviderKind::Grok));
        assert_eq!(ProviderKind::parse("chatgpt"), Some(ProviderKind::Chatgpt));
        assert!(ProviderKind::Supergrok.subscription() && !ProviderKind::Openai.subscription());
        assert_eq!(
            ProviderKind::parse("minimax_plan_cn"),
            Some(ProviderKind::MinimaxPlanCn)
        );
        assert!(ProviderKind::MinimaxPlan.subscription() && !ProviderKind::Minimax.subscription());
        assert_eq!(ProviderKind::parse("palm"), None);
    }

    #[test]
    fn openrouter_is_a_provider_kind() {
        assert_eq!(
            ProviderKind::parse("openrouter"),
            Some(ProviderKind::Openrouter)
        );
        assert_eq!(ProviderKind::Openrouter.as_str(), "openrouter");
    }
}

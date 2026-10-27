//! Bounded text-turn authority. Never log or persist an inference grant's credentials.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
pub struct InferenceGrant {
    pub turn: Uuid,
    pub authority: Uuid,
    pub expires_in_seconds: u64,
    pub max_attempts: u8,
    #[serde(default)]
    pub max_tool_calls: u8,
    pub model: String,
    pub provider_revision: i64,
    pub api_key: String,
    pub messages: Vec<Message>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatEvidence {
    TerminalOutcome {
        turn: Uuid,
        authority: Uuid,
        invocation: Uuid,
        outcome: crate::terminal::Outcome,
    },
    TextIntent {
        turn: Uuid,
        authority: Uuid,
    },
    TextRetry {
        turn: Uuid,
        authority: Uuid,
    },
    TextOutcome {
        turn: Uuid,
        authority: Uuid,
        outcome: Outcome,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Complete { response: String },
    Waiting { error: String },
}

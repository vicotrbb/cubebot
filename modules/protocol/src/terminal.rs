//! Finite terminal authority. Invocation identity is immutable across reconciliation.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const LEASE_MILLIS: u64 = 60_000;
pub const REQUEST_LIMIT: usize = 16384;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "InvocationWire")]
pub struct Invocation {
    pub id: Uuid,
    pub authority: Uuid,
    pub command: String,
}
// Retained intents from the first terminal implementation include `seconds`.
// Read that historical field without reintroducing a command-duration policy.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InvocationWire {
    id: Uuid,
    authority: Uuid,
    command: String,
    #[serde(default, rename = "seconds")]
    _legacy_seconds: Option<u64>,
}
impl From<InvocationWire> for Invocation {
    fn from(value: InvocationWire) -> Self {
        Self {
            id: value.id,
            authority: value.authority,
            command: value.command,
        }
    }
}
impl Invocation {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.is_nil() || self.authority.is_nil() {
            return Err("missing invocation identity");
        }
        if self.command.is_empty() || self.command.len() > 8192 || self.command.contains('\0') {
            return Err("invalid terminal command");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Exited { code: Option<i32>, output: String },
    Stopped { reason: String, output: String },
    Uncertain,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authorization {
    pub turn: Uuid,
    pub invocation: Invocation,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Renewal {
    pub turn: Uuid,
    pub authority: Uuid,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permit {
    pub milliseconds: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Execute {
        invocation: Invocation,
        not_after_millis: u64,
    },
    Lookup {
        id: Uuid,
        authority: Uuid,
    },
}

impl Outcome {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Stopped { reason, .. }
                if !matches!(
                    reason.as_str(),
                    "cancelled" | "authority_expired" | "duration_limit" | "output_limit"
                ) =>
            {
                Err("invalid terminal stop reason")
            }
            _ => Ok(()),
        }
    }
}

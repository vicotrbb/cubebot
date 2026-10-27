//! Finite terminal authority. Invocation identity is immutable across reconciliation.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const OUTPUT_LIMIT: usize = 8192;
pub const REQUEST_LIMIT: usize = 16384;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invocation {
    pub id: Uuid,
    pub authority: Uuid,
    pub command: String,
    pub seconds: u64,
}
impl Invocation {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.is_nil() || self.authority.is_nil() {
            return Err("missing invocation identity");
        }
        if !(1..=30).contains(&self.seconds) {
            return Err("terminal duration must be 1..30 seconds");
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
        let output = match self {
            Self::Exited { output, .. } => output,
            Self::Stopped { output, reason } => {
                if !matches!(
                    reason.as_str(),
                    "cancelled" | "duration_limit" | "output_limit"
                ) {
                    return Err("invalid terminal stop reason");
                }
                output
            }
            Self::Uncertain => return Ok(()),
        };
        if output.len() > OUTPUT_LIMIT {
            return Err("terminal output too large");
        }
        Ok(())
    }
}

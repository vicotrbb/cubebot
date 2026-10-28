//! Terminal command outcomes reported as tool results.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Exited { code: Option<i32>, output: String },
    Stopped { reason: String, output: String },
    Uncertain,
}

impl Outcome {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Stopped { reason, .. } if reason != "cancelled" => {
                Err("invalid terminal stop reason")
            }
            _ => Ok(()),
        }
    }
}

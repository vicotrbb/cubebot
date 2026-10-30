//! Terminal command outcomes reported as tool results.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Exited {
        code: Option<i32>,
        output: String,
    },
    Stopped {
        reason: String,
        output: String,
    },
    /// A computer-use call that returns the screen; `image` is a base64 PNG (ADR 0085).
    Screenshot {
        output: String,
        image: String,
    },
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

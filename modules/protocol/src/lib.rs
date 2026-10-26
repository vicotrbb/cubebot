//! Wire types shared across Cubebot process boundaries, not internal domain models.

use serde::{Deserialize, Serialize};

/// Process liveness only; this does not attest to dependency or agent readiness.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Liveness {
    pub status: LivenessStatus,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LivenessStatus {
    Alive,
}

pub mod runtime;

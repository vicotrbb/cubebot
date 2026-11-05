use serde::{Deserialize, Serialize};

/// Process liveness only; this does not attest to dependency or agent readiness.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Liveness {
    pub status: LivenessStatus,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LivenessStatus {
    Alive,
}

/// The exit status of a sandbox that refused a workspace volume laid out by the other runtime. Launchers report it.
pub const LAYOUT_MISMATCH: u8 = 78;

pub mod desktop;
pub mod guest;
pub mod network;
pub mod runtime;

pub mod terminal;

//! Machine identity and lifecycle control; these messages convey no per-turn authority.
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
    /// Maximum lifetime measured from the client's bootstrap request start.
    pub expires_in_seconds: u64,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeActor {
    pub agent: Uuid,
    pub attempt: Uuid,
    pub epoch: i64,
    pub version: i64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrainRequest {
    pub request_id: Uuid,
    pub acknowledged: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrainAcknowledgement {
    pub request_id: Uuid,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeDisposition {
    Available,
    Hold,
    Drain,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeControl {
    pub actor: RuntimeActor,
    pub disposition: RuntimeDisposition,
    pub drain: Option<DrainRequest>,
}

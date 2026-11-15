//! Cubebot Meetings, phase 1 ([Cubebot Meetings](../../../../docs/adr/meetings.md)): the meetings,
//! links, guests, tokens and transcript the orchestrator owns. There is no media here yet.
//!
//! Two kinds of caller meet in this module and never share an authority. A console person owns the
//! meetings they create and reaches them through [`host`]. A guest holds only a join token for one
//! meeting and reaches [`guests`]; guest tokens are not sessions and no console route accepts them.
mod guests;
mod host;
mod settings;

pub use guests::{
    GUEST_LIMIT, GuestAccess, GuestJoin, JoinClaims, LOBBY_TOKEN_SECONDS, MEDIA_TOKEN_SECONDS,
};
pub use host::{
    CreateMeeting, CreatedMeetingLink, InviteMeetingAgent, Meeting, MeetingDetail, MeetingLink,
    MeetingParticipant, NewMeetingLink, TranscriptSegment,
};
pub use settings::MeetingSettings;

use crate::identity::IdentityError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// How an agent takes part. Phase 1 stores the choice; the speaking policy that acts on it comes
/// with the agent bridge.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum MeetingAgentMode {
    ListenOnly,
    OnMention,
    Proactive,
}

impl MeetingAgentMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ListenOnly => "listen_only",
            Self::OnMention => "on_mention",
            Self::Proactive => "proactive",
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum MeetingParticipantKind {
    Agent,
    Guest,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum MeetingParticipantStatus {
    Invited,
    Waiting,
    Admitted,
    Denied,
}

impl MeetingParticipantStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Invited => "invited",
            Self::Waiting => "waiting",
            Self::Admitted => "admitted",
            Self::Denied => "denied",
        }
    }
}

/// `Lobby` may only ask whether the host has admitted the guest; `Media` may join the call.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum JoinTokenScope {
    Lobby,
    Media,
}

impl JoinTokenScope {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Lobby => "lobby",
            Self::Media => "media",
        }
    }
}

#[derive(Clone)]
pub struct Meetings {
    pool: PgPool,
}

impl Meetings {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// A trimmed, non-empty name or title of at most `max` characters without control characters.
fn clean(text: &str, max: usize) -> Result<&str, IdentityError> {
    let text = text.trim();
    let valid = (1..=max).contains(&text.chars().count()) && !text.chars().any(char::is_control);
    valid.then_some(text).ok_or(IdentityError::InvalidInput)
}

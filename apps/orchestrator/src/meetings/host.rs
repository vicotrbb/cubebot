//! What the person who created a meeting may do with it. Meetings belong to their creator alone;
//! administrators have no extra reach, as with calendars.
use super::{MeetingAgentMode, MeetingParticipantKind, MeetingParticipantStatus, Meetings, clean};
use crate::{
    agents::AgentId,
    identity::{Actor, IdentityError, digest, token},
    work::Work,
};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

const TITLE_CHARS: usize = 120;
const SPEAKER_CHARS: usize = 64;
const TEXT_BYTES: usize = 4096;
const SEGMENT_PAGE: i64 = 1000;
const MIN_LINK_SECONDS: i64 = 60;
const MAX_LINK_SECONDS: i64 = 365 * 24 * 3600;

#[derive(Serialize, utoipa::ToSchema)]
pub struct Meeting {
    pub id: Uuid,
    pub title: String,
    pub consent_notice: bool,
    pub agent_mode: MeetingAgentMode,
    pub created_at: String,
    /// `None` while the meeting is open.
    pub ended_at: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct MeetingParticipant {
    pub id: Uuid,
    pub kind: MeetingParticipantKind,
    /// The agent, for an agent participant.
    pub agent: Option<Uuid>,
    pub name: String,
    pub status: MeetingParticipantStatus,
    pub agent_mode: Option<MeetingAgentMode>,
    pub created_at: String,
}

/// A link's record. Its token is shown once, when it is created, and is never stored.
#[derive(Serialize, utoipa::ToSchema)]
pub struct MeetingLink {
    pub id: Uuid,
    pub lobby: bool,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
    /// Not revoked and not expired: a new guest can join with it.
    pub active: bool,
    pub created_at: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct MeetingDetail {
    pub meeting: Meeting,
    pub participants: Vec<MeetingParticipant>,
    pub links: Vec<MeetingLink>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct CreatedMeetingLink {
    pub link: MeetingLink,
    /// The secret half of the public link; anyone holding it can ask to join.
    pub token: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct TranscriptSegment {
    pub sequence: i64,
    pub participant: Option<Uuid>,
    pub speaker: String,
    pub spoken_at: String,
    pub text: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateMeeting {
    pub title: String,
    /// `None` takes the installation's setting.
    #[serde(default)]
    pub consent_notice: Option<bool>,
    /// `None` takes the installation's setting.
    #[serde(default)]
    pub agent_mode: Option<MeetingAgentMode>,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewMeetingLink {
    /// `None` takes the installation's setting.
    #[serde(default)]
    pub lobby: Option<bool>,
    /// How long new guests can join with this link, from 60 seconds to a year; `None` for no limit.
    #[serde(default)]
    pub expires_in_seconds: Option<i64>,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InviteMeetingAgent {
    pub agent: Uuid,
    /// `None` takes the meeting's setting.
    #[serde(default)]
    pub mode: Option<MeetingAgentMode>,
}

impl Meetings {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn create(
        &self,
        actor: &Actor,
        input: &CreateMeeting,
    ) -> Result<Meeting, IdentityError> {
        let title = clean(&input.title, TITLE_CHARS)?;
        sqlx::query_as!(
            Meeting,
            r#"
            INSERT INTO meetings (created_by, title, consent_notice, agent_mode)
            SELECT a.id, $2, COALESCE($3, s.consent_notice), COALESCE($4, s.agent_mode)
            FROM human_accounts a CROSS JOIN meeting_settings s
            WHERE a.id = $1 AND a.active
            RETURNING id, title, consent_notice, agent_mode AS "agent_mode: MeetingAgentMode",
                (to_jsonb(created_at) #>> '{}') AS "created_at!",
                (to_jsonb(ended_at) #>> '{}') AS ended_at
            "#,
            actor.id,
            title,
            input.consent_notice,
            input.agent_mode.map(MeetingAgentMode::as_str),
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::Unauthorized)
    }

    /// The caller's meetings, newest first.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn list(&self, actor: &Actor) -> Result<Vec<Meeting>, IdentityError> {
        Ok(sqlx::query_as!(
            Meeting,
            r#"
            SELECT m.id, m.title, m.consent_notice, m.agent_mode AS "agent_mode: MeetingAgentMode",
                (to_jsonb(m.created_at) #>> '{}') AS "created_at!",
                (to_jsonb(m.ended_at) #>> '{}') AS ended_at
            FROM meetings m
            JOIN human_accounts a ON a.id = m.created_by AND a.active
            WHERE m.created_by = $1
            ORDER BY m.created_at DESC, m.id
            LIMIT 200
            "#,
            actor.id,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, meeting = %meeting))]
    pub async fn get(&self, actor: &Actor, meeting: Uuid) -> Result<MeetingDetail, IdentityError> {
        let mut tx = self.pool.begin().await?;
        lock_owned(&mut tx, actor, meeting).await?;
        let record = sqlx::query_as!(
            Meeting,
            r#"
            SELECT id, title, consent_notice, agent_mode AS "agent_mode: MeetingAgentMode",
                (to_jsonb(created_at) #>> '{}') AS "created_at!",
                (to_jsonb(ended_at) #>> '{}') AS ended_at
            FROM meetings WHERE id = $1
            "#,
            meeting,
        )
        .fetch_one(&mut *tx)
        .await?;
        let participants = sqlx::query_as!(
            MeetingParticipant,
            r#"
            SELECT id, kind AS "kind: MeetingParticipantKind", agent_id AS agent, name,
                status AS "status: MeetingParticipantStatus", agent_mode AS "agent_mode: MeetingAgentMode",
                (to_jsonb(created_at) #>> '{}') AS "created_at!"
            FROM meeting_participants WHERE meeting_id = $1
            ORDER BY created_at, id
            "#,
            meeting,
        )
        .fetch_all(&mut *tx)
        .await?;
        let links = sqlx::query_as!(
            MeetingLink,
            r#"
            SELECT id, lobby, (to_jsonb(expires_at) #>> '{}') AS expires_at,
                (to_jsonb(revoked_at) #>> '{}') AS revoked_at,
                (revoked_at IS NULL AND (expires_at IS NULL OR expires_at > clock_timestamp()))
                    AS "active!",
                (to_jsonb(created_at) #>> '{}') AS "created_at!"
            FROM meeting_links WHERE meeting_id = $1
            ORDER BY created_at, id
            "#,
            meeting,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(MeetingDetail {
            meeting: record,
            participants,
            links,
        })
    }

    /// Ends the meeting for good: no one can join and nothing more is recorded.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, meeting = %meeting))]
    pub async fn end(&self, actor: &Actor, meeting: Uuid) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        require_open(&mut tx, actor, meeting).await?;
        sqlx::query!(
            "UPDATE meetings SET ended_at = clock_timestamp() WHERE id = $1",
            meeting,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Makes a public link. The token comes back once and only its digest is kept.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, meeting = %meeting))]
    pub async fn create_link(
        &self,
        actor: &Actor,
        meeting: Uuid,
        input: &NewMeetingLink,
    ) -> Result<CreatedMeetingLink, IdentityError> {
        if let Some(seconds) = input.expires_in_seconds
            && !(MIN_LINK_SECONDS..=MAX_LINK_SECONDS).contains(&seconds)
        {
            return Err(IdentityError::InvalidInput);
        }
        let mut tx = self.pool.begin().await?;
        require_open(&mut tx, actor, meeting).await?;
        let secret = token();
        let link = sqlx::query_as!(
            MeetingLink,
            r#"
            INSERT INTO meeting_links (meeting_id, token_digest, lobby, expires_at)
            SELECT $1, $2, COALESCE($3, s.lobby_default),
                clock_timestamp() + make_interval(secs => $4::double precision)
            FROM meeting_settings s
            RETURNING id, lobby, (to_jsonb(expires_at) #>> '{}') AS expires_at,
                (to_jsonb(revoked_at) #>> '{}') AS revoked_at,
                (revoked_at IS NULL AND (expires_at IS NULL OR expires_at > clock_timestamp()))
                    AS "active!",
                (to_jsonb(created_at) #>> '{}') AS "created_at!"
            "#,
            meeting,
            digest(&secret),
            input.lobby,
            input.expires_in_seconds.map(|seconds| seconds as f64),
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(CreatedMeetingLink {
            link,
            token: secret,
        })
    }

    /// Stops new guests and ends the access of every guest who came in through the link.
    /// Revoking twice is not an error.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, meeting = %meeting, link = %link))]
    pub async fn revoke_link(
        &self,
        actor: &Actor,
        meeting: Uuid,
        link: Uuid,
    ) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        lock_owned(&mut tx, actor, meeting).await?;
        sqlx::query_scalar!(
            r#"
            UPDATE meeting_links SET revoked_at = COALESCE(revoked_at, clock_timestamp())
            WHERE id = $1 AND meeting_id = $2
            RETURNING id
            "#,
            link,
            meeting,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        tx.commit().await?;
        Ok(())
    }

    /// The caller must be allowed to message the agent (`conversation.send`), as for a call.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, meeting = %meeting))]
    pub async fn invite_agent(
        &self,
        actor: &Actor,
        meeting: Uuid,
        input: &InviteMeetingAgent,
    ) -> Result<MeetingParticipant, IdentityError> {
        Work::new(self.pool.clone())
            .may_call(actor, AgentId(input.agent))
            .await?;
        let mut tx = self.pool.begin().await?;
        require_open(&mut tx, actor, meeting).await?;
        let participant = sqlx::query_as!(
            MeetingParticipant,
            r#"
            INSERT INTO meeting_participants (meeting_id, kind, agent_id, name, status, agent_mode)
            SELECT m.id, 'agent', a.id, a.name, 'invited', COALESCE($3, m.agent_mode)
            FROM meetings m CROSS JOIN agents a
            WHERE m.id = $1 AND a.id = $2
            RETURNING id, kind AS "kind: MeetingParticipantKind", agent_id AS agent, name,
                status AS "status: MeetingParticipantStatus", agent_mode AS "agent_mode: MeetingAgentMode",
                (to_jsonb(created_at) #>> '{}') AS "created_at!"
            "#,
            meeting,
            input.agent,
            input.mode.map(MeetingAgentMode::as_str),
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        tx.commit().await?;
        Ok(participant)
    }

    /// Lets a waiting guest in. Only a waiting guest can be admitted.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, meeting = %meeting))]
    pub async fn admit(
        &self,
        actor: &Actor,
        meeting: Uuid,
        participant: Uuid,
    ) -> Result<MeetingParticipant, IdentityError> {
        self.decide(
            actor,
            meeting,
            participant,
            MeetingParticipantStatus::Admitted,
        )
        .await
    }

    /// Turns a waiting guest away, or removes an admitted one: their tokens stop working at once.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, meeting = %meeting))]
    pub async fn deny(
        &self,
        actor: &Actor,
        meeting: Uuid,
        participant: Uuid,
    ) -> Result<MeetingParticipant, IdentityError> {
        self.decide(
            actor,
            meeting,
            participant,
            MeetingParticipantStatus::Denied,
        )
        .await
    }

    async fn decide(
        &self,
        actor: &Actor,
        meeting: Uuid,
        participant: Uuid,
        to: MeetingParticipantStatus,
    ) -> Result<MeetingParticipant, IdentityError> {
        // A denial may remove an admitted guest; an admission only moves a waiting one.
        let from: &[&str] = match to {
            MeetingParticipantStatus::Admitted => &["waiting"],
            _ => &["waiting", "admitted"],
        };
        let mut tx = self.pool.begin().await?;
        require_open(&mut tx, actor, meeting).await?;
        let changed = sqlx::query_as!(
            MeetingParticipant,
            r#"
            UPDATE meeting_participants SET status = $3
            WHERE id = $1 AND meeting_id = $2 AND kind = 'guest' AND status = ANY($4)
            RETURNING id, kind AS "kind: MeetingParticipantKind", agent_id AS agent, name,
                status AS "status: MeetingParticipantStatus", agent_mode AS "agent_mode: MeetingAgentMode",
                (to_jsonb(created_at) #>> '{}') AS "created_at!"
            "#,
            participant,
            meeting,
            to.as_str(),
            from as &[&str],
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(changed) = changed else {
            return Err(refused(&mut tx, meeting, participant).await?);
        };
        if to == MeetingParticipantStatus::Denied {
            sqlx::query!(
                "DELETE FROM meeting_join_tokens WHERE participant_id = $1",
                participant,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(changed)
    }

    /// The segments after `after`, oldest first, at most a page of them.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, meeting = %meeting))]
    pub async fn transcript(
        &self,
        actor: &Actor,
        meeting: Uuid,
        after: i64,
    ) -> Result<Vec<TranscriptSegment>, IdentityError> {
        let mut tx = self.pool.begin().await?;
        lock_owned(&mut tx, actor, meeting).await?;
        let segments = sqlx::query_as!(
            TranscriptSegment,
            r#"
            SELECT sequence, participant_id AS participant, speaker,
                (to_jsonb(spoken_at) #>> '{}') AS "spoken_at!", text
            FROM meeting_transcript_segments
            WHERE meeting_id = $1 AND sequence > $2
            ORDER BY sequence
            LIMIT $3
            "#,
            meeting,
            after,
            SEGMENT_PAGE,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(segments)
    }

    /// Records one thing said, for the agent bridge and speech pipeline to call. Refused for a
    /// meeting that has ended, and for a participant who is not in the meeting or not let in.
    #[tracing::instrument(skip_all, fields(meeting = %meeting))]
    pub async fn append_segment(
        &self,
        meeting: Uuid,
        participant: Option<Uuid>,
        speaker: &str,
        text: &str,
    ) -> Result<i64, IdentityError> {
        let speaker = clean(speaker, SPEAKER_CHARS)?;
        let text = text.trim();
        if text.is_empty() || text.len() > TEXT_BYTES {
            return Err(IdentityError::InvalidInput);
        }
        sqlx::query_scalar!(
            r#"
            INSERT INTO meeting_transcript_segments (meeting_id, participant_id, speaker, text)
            SELECT m.id, $2, $3, $4
            FROM meetings m
            WHERE m.id = $1 AND m.ended_at IS NULL
                AND ($2::uuid IS NULL OR EXISTS (
                    SELECT 1 FROM meeting_participants p
                    WHERE p.id = $2 AND p.meeting_id = m.id AND p.status IN ('invited', 'admitted')))
            RETURNING sequence
            "#,
            meeting,
            participant,
            speaker,
            text,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::NotFound)
    }
}

/// Locks the caller's own meeting and says whether it has ended. Someone else's meeting does not
/// exist as far as the caller can tell.
async fn lock_owned(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    meeting: Uuid,
) -> Result<bool, IdentityError> {
    let ended = sqlx::query_scalar!(
        r#"
        SELECT m.ended_at IS NOT NULL AS "ended!"
        FROM meetings m
        JOIN human_accounts a ON a.id = m.created_by AND a.active
        WHERE m.id = $1 AND m.created_by = $2
        FOR UPDATE OF m
        "#,
        meeting,
        actor.id,
    )
    .fetch_optional(&mut **tx)
    .await?;
    ended.ok_or(IdentityError::NotFound)
}

async fn require_open(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    meeting: Uuid,
) -> Result<(), IdentityError> {
    if lock_owned(tx, actor, meeting).await? {
        return Err(IdentityError::Conflict);
    }
    Ok(())
}

/// Why a participant could not be moved: absent from this meeting, or in the wrong state.
async fn refused(
    tx: &mut Transaction<'_, Postgres>,
    meeting: Uuid,
    participant: Uuid,
) -> Result<IdentityError, IdentityError> {
    let present = sqlx::query_scalar!(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM meeting_participants
            WHERE id = $1 AND meeting_id = $2 AND kind = 'guest'
        ) AS "present!"
        "#,
        participant,
        meeting,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(if present {
        IdentityError::Conflict
    } else {
        IdentityError::NotFound
    })
}

//! How someone with a public link gets in. A guest has no account: they show a link token, give a
//! name, and receive a short-lived join token for that one meeting. Nothing here creates a session,
//! and the console's authentication never reads these tokens.
use super::{JoinTokenScope, MeetingParticipantStatus, Meetings, clean};
use crate::identity::{IdentityError, digest, token};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// A guest waiting in the lobby keeps asking, so the token lives long enough to wait.
pub const LOBBY_TOKEN_SECONDS: i64 = 30 * 60;
/// Valid only to join the call, so it stays short; the guest refreshes it while they stay.
pub const MEDIA_TOKEN_SECONDS: i64 = 5 * 60;
/// Guests waiting or admitted at once in one meeting. A stop-gap against a link flooding a lobby,
/// until joins are rate-limited per address (docs/adr/meetings.md, open details).
pub const GUEST_LIMIT: i64 = 100;
const NAME_CHARS: usize = 64;

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GuestJoin {
    /// The link's secret token.
    pub link: String,
    /// What the others see.
    pub name: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct GuestAccess {
    pub meeting: Uuid,
    pub meeting_title: String,
    pub participant: Uuid,
    pub status: MeetingParticipantStatus,
    pub scope: JoinTokenScope,
    pub token: String,
    pub expires_in_seconds: i64,
    /// Show the guest that agents may listen and a transcript is kept.
    pub consent_notice: bool,
}

/// What a live join token stands for.
#[derive(Debug, PartialEq, Eq)]
pub struct JoinClaims {
    pub meeting: Uuid,
    pub participant: Uuid,
    pub scope: JoinTokenScope,
}

struct Claims {
    meeting: Uuid,
    title: String,
    consent_notice: bool,
    participant: Uuid,
    status: MeetingParticipantStatus,
}

impl Meetings {
    /// Asks to join through a link. An unknown, revoked or expired link, and a meeting that has
    /// ended, all answer alike: `NotFound`.
    #[tracing::instrument(skip_all)]
    pub async fn join(&self, input: &GuestJoin) -> Result<GuestAccess, IdentityError> {
        let name = clean(&input.name, NAME_CHARS)?;
        let mut tx = self.pool.begin().await?;
        // Locking the meeting makes the count below and the insert one step, so concurrent joins
        // cannot pass the limit together.
        let link = sqlx::query!(
            r#"
            SELECT l.id AS link, l.lobby, m.id AS meeting, m.title, m.consent_notice
            FROM meeting_links l
            JOIN meetings m ON m.id = l.meeting_id
            WHERE l.token_digest = $1 AND l.revoked_at IS NULL
                AND (l.expires_at IS NULL OR l.expires_at > clock_timestamp())
                AND m.ended_at IS NULL
            FOR UPDATE OF m
            "#,
            digest(&input.link),
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        let guests = sqlx::query_scalar!(
            r#"
            SELECT count(*) AS "guests!" FROM meeting_participants
            WHERE meeting_id = $1 AND kind = 'guest' AND status IN ('waiting', 'admitted')
            "#,
            link.meeting,
        )
        .fetch_one(&mut *tx)
        .await?;
        if guests >= GUEST_LIMIT {
            return Err(IdentityError::Busy);
        }
        let status = if link.lobby {
            MeetingParticipantStatus::Waiting
        } else {
            MeetingParticipantStatus::Admitted
        };
        let participant = sqlx::query_scalar!(
            r#"
            INSERT INTO meeting_participants (meeting_id, kind, link_id, name, status)
            VALUES ($1, 'guest', $2, $3, $4)
            RETURNING id
            "#,
            link.meeting,
            link.link,
            name,
            status.as_str(),
        )
        .fetch_one(&mut *tx)
        .await?;
        let access = issue(
            &mut tx,
            Claims {
                meeting: link.meeting,
                title: link.title,
                consent_notice: link.consent_notice,
                participant,
                status,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(access)
    }

    /// Trades a live join token for a fresh one that matches where the guest stands now: a lobby
    /// token while they wait, a media token once admitted. This is how a waiting guest finds out.
    #[tracing::instrument(skip_all)]
    pub async fn refresh(&self, join_token: &str) -> Result<GuestAccess, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let (claims, _) = live(&mut *tx, join_token).await?;
        let access = issue(&mut tx, claims).await?;
        tx.commit().await?;
        Ok(access)
    }

    /// What the media side calls before it lets a guest into a call. A token of any other scope,
    /// or one whose guest, link or meeting is no longer in good standing, is refused.
    #[tracing::instrument(skip_all)]
    pub async fn verify(
        &self,
        join_token: &str,
        scope: JoinTokenScope,
    ) -> Result<JoinClaims, IdentityError> {
        let (claims, held) = live(&self.pool, join_token).await?;
        if held != scope {
            return Err(IdentityError::Forbidden);
        }
        Ok(JoinClaims {
            meeting: claims.meeting,
            participant: claims.participant,
            scope: held,
        })
    }
}

/// Finds the token's guest only while all of this holds: the token has not expired, the meeting is
/// open, the link the guest came through is not revoked, and the guest is waiting or admitted. A
/// guest only ever moves from waiting to admitted to denied, and a denial deletes their tokens, so
/// a media token cannot outlive its admission.
async fn live(
    executor: impl sqlx::PgExecutor<'_>,
    join_token: &str,
) -> Result<(Claims, JoinTokenScope), IdentityError> {
    let row = sqlx::query!(
        r#"
        SELECT p.meeting_id AS meeting, m.title, m.consent_notice, p.id AS participant,
            p.status AS "status: MeetingParticipantStatus", t.scope AS "scope: JoinTokenScope"
        FROM meeting_join_tokens t
        JOIN meeting_participants p ON p.id = t.participant_id
        JOIN meetings m ON m.id = p.meeting_id
        JOIN meeting_links l ON l.id = p.link_id
        WHERE t.token_digest = $1 AND t.expires_at > clock_timestamp()
            AND m.ended_at IS NULL AND l.revoked_at IS NULL
            AND p.status IN ('waiting', 'admitted')
        "#,
        digest(join_token),
    )
    .fetch_optional(executor)
    .await?
    .ok_or(IdentityError::Unauthorized)?;
    let claims = Claims {
        meeting: row.meeting,
        title: row.title,
        consent_notice: row.consent_notice,
        participant: row.participant,
        status: row.status,
    };
    Ok((claims, row.scope))
}

/// Mints a token whose scope follows the guest's status, and drops the guest's expired ones.
async fn issue(
    tx: &mut Transaction<'_, Postgres>,
    claims: Claims,
) -> Result<GuestAccess, IdentityError> {
    let (scope, seconds) = match claims.status {
        MeetingParticipantStatus::Admitted => (JoinTokenScope::Media, MEDIA_TOKEN_SECONDS),
        _ => (JoinTokenScope::Lobby, LOBBY_TOKEN_SECONDS),
    };
    sqlx::query!(
        "DELETE FROM meeting_join_tokens WHERE participant_id = $1 AND expires_at <= clock_timestamp()",
        claims.participant,
    )
    .execute(&mut **tx)
    .await?;
    let secret = token();
    sqlx::query!(
        r#"
        INSERT INTO meeting_join_tokens (token_digest, participant_id, scope, expires_at)
        VALUES ($1, $2, $3, clock_timestamp() + make_interval(secs => $4::double precision))
        "#,
        digest(&secret),
        claims.participant,
        scope.as_str(),
        seconds as f64,
    )
    .execute(&mut **tx)
    .await?;
    Ok(GuestAccess {
        meeting: claims.meeting,
        meeting_title: claims.title,
        participant: claims.participant,
        status: claims.status,
        scope,
        token: secret,
        expires_in_seconds: seconds,
        consent_notice: claims.consent_notice,
    })
}

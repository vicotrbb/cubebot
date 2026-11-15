mod calendar;
mod invitations;
mod settings;

pub(crate) use calendar::items as calendar_items;
pub use invitations::{EmailConnection, Invited};
pub use settings::{MeetingSettings, SaveMeetingSettings};

use crate::{
    agents::Avatar,
    connections::Connections,
    identity::{Actor, IdentityError, oidc::Config},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::DateTime;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction, types::Json};
use uuid::Uuid;

const TITLE_CHARS: usize = 200;
const DESCRIPTION_CHARS: usize = 4000;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum AgentMode {
    Relevant,
    Named,
}

impl AgentMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Relevant => "relevant",
            Self::Named => "named",
        }
    }
}

#[derive(Clone)]
pub struct Meetings {
    pool: PgPool,
    connections: Option<Connections>,
    origin: Option<String>,
}

#[derive(Default, Deserialize, utoipa::ToSchema)]
#[serde(default, deny_unknown_fields)]
#[schema(as = MeetingInvite)]
pub struct Invite {
    pub accounts: Vec<Uuid>,
    pub agents: Vec<Uuid>,
    pub emails: Vec<String>,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewMeeting {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    #[serde(default)]
    pub open_join: bool,
    pub agent_mode: Option<AgentMode>,
    #[serde(default)]
    pub invite: Invite,
    pub email_connection: Option<Uuid>,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewInvitations {
    #[serde(default)]
    pub invite: Invite,
    pub email_connection: Option<Uuid>,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeMeeting {
    pub title: Option<String>,
    pub description: Option<String>,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub open_join: Option<bool>,
    pub locked: Option<bool>,
    pub agent_mode: Option<AgentMode>,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetLink {
    pub enabled: bool,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct MeetingLink {
    pub link: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct MeetingSummary {
    pub id: Uuid,
    pub title: String,
    pub owner_name: String,
    pub state: String,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub people: i64,
    pub agents: i64,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Meeting {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub owner: bool,
    pub owner_name: String,
    pub state: String,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub link: Option<String>,
    pub open_join: bool,
    pub locked: bool,
    pub agent_mode: AgentMode,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub summary: Option<String>,
    pub created_at: String,
    pub invitations: Vec<Invitation>,
    pub attendance: Vec<Attendance>,
    pub utterances: Vec<Utterance>,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = MeetingInvitation)]
pub struct Invitation {
    pub id: Uuid,
    pub kind: String,
    pub account: Option<Uuid>,
    pub agent: Option<Uuid>,
    pub email: Option<String>,
    pub name: String,
    #[schema(value_type = Option<Avatar>)]
    pub avatar: Option<Json<Avatar>>,
    pub emailed_at: Option<String>,
    pub email_error: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Attendance {
    pub kind: String,
    pub account: Option<Uuid>,
    pub agent: Option<Uuid>,
    pub name: String,
    pub joined_at: String,
    pub left_at: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Utterance {
    pub sequence: i64,
    pub kind: String,
    pub speaker: String,
    pub text: String,
    pub at: String,
}

struct Access {
    owner: bool,
    live: bool,
    ended: bool,
    title: String,
}

impl Access {
    fn owned(self) -> Result<Self, IdentityError> {
        self.owner.then_some(self).ok_or(IdentityError::Forbidden)
    }
}

impl Meetings {
    pub fn new(pool: PgPool, connections: Option<Connections>, origin: Option<String>) -> Self {
        Self {
            pool,
            connections,
            origin,
        }
    }

    pub fn environment(
        pool: PgPool,
        connections: Option<Connections>,
        console: Option<String>,
    ) -> Result<Self, IdentityError> {
        let origin = match std::env::var("CUBEBOT_MEETINGS_ORIGIN") {
            Ok(origin) => Some(Config::public_origin(&origin)?),
            Err(std::env::VarError::NotPresent) => console,
            Err(error) => return Err(IdentityError::Invalid(error.into())),
        };
        Ok(Self::new(pool, connections, origin))
    }

    fn link(&self, token: Option<String>) -> Option<String> {
        let origin = self.origin.as_deref().unwrap_or_default();
        token.map(|token| format!("{origin}/m/{token}"))
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn create(
        &self,
        actor: &Actor,
        input: &NewMeeting,
    ) -> Result<(Meeting, Invited), IdentityError> {
        let title = title(&input.title)?;
        description(&input.description)?;
        let schedule = schedule(input.starts_at.as_deref(), input.ends_at.as_deref())?;
        let emails = invitations::addresses(&input.invite.emails)?;
        let sender = self.sender(actor, input.email_connection).await?;
        let mut tx = self.pool.begin().await?;
        let id = sqlx::query_scalar!(
            "
            INSERT INTO meetings
                (title, description, owner_id, starts_at, ends_at, link_token, open_join, agent_mode)
            VALUES ($1, $2, $3, to_timestamp($4::bigint), to_timestamp($5::bigint), $6, $7,
                COALESCE($8, (SELECT agent_mode FROM meeting_settings)))
            RETURNING id
            ",
            title,
            input.description,
            actor.id,
            schedule.map(|(starts, _)| starts),
            schedule.map(|(_, ends)| ends),
            token(),
            input.open_join,
            input.agent_mode.map(AgentMode::as_str),
        )
        .fetch_one(&mut *tx)
        .await?;
        let added = self
            .add(&mut tx, actor, id, &input.invite, &emails, sender.as_ref())
            .await?;
        tx.commit().await?;
        self.deliver(actor, id, sender.as_ref(), &added).await?;
        Ok((self.meeting(actor, id).await?, added.counts(title)))
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, meeting = %id))]
    pub async fn invite(
        &self,
        actor: &Actor,
        id: Uuid,
        input: &NewInvitations,
    ) -> Result<Invited, IdentityError> {
        let emails = invitations::addresses(&input.invite.emails)?;
        let sender = self.sender(actor, input.email_connection).await?;
        let mut tx = self.pool.begin().await?;
        let access = access(&mut tx, actor, id).await?;
        if access.ended {
            return Err(IdentityError::Conflict);
        }
        let added = self
            .add(&mut tx, actor, id, &input.invite, &emails, sender.as_ref())
            .await?;
        tx.commit().await?;
        self.deliver(actor, id, sender.as_ref(), &added).await?;
        Ok(added.counts(access.title))
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, meeting = %id))]
    pub async fn uninvite(
        &self,
        actor: &Actor,
        id: Uuid,
        invitation: Uuid,
    ) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        access(&mut tx, actor, id).await?.owned()?;
        let removed = sqlx::query!(
            "DELETE FROM meeting_invitations WHERE id=$2 AND meeting_id=$1",
            id,
            invitation,
        )
        .execute(&mut *tx)
        .await?;
        if removed.rows_affected() == 0 {
            return Err(IdentityError::NotFound);
        }
        tx.commit().await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, meeting = %id))]
    pub async fn update(
        &self,
        actor: &Actor,
        id: Uuid,
        change: &ChangeMeeting,
    ) -> Result<String, IdentityError> {
        let new_title = change.title.as_deref().map(title).transpose()?;
        change.description.as_deref().map(description).transpose()?;
        let schedule = schedule(change.starts_at.as_deref(), change.ends_at.as_deref())?;
        let mut tx = self.pool.begin().await?;
        access(&mut tx, actor, id).await?.owned()?;
        let title = sqlx::query_scalar!(
            "
            UPDATE meetings
            SET title=COALESCE($2, title), description=COALESCE($3, description),
                starts_at=COALESCE(to_timestamp($4::bigint), starts_at),
                ends_at=COALESCE(to_timestamp($5::bigint), ends_at),
                open_join=COALESCE($6, open_join), locked=COALESCE($7, locked),
                agent_mode=COALESCE($8, agent_mode)
            WHERE id=$1
            RETURNING title
            ",
            id,
            new_title,
            change.description,
            schedule.map(|(starts, _)| starts),
            schedule.map(|(_, ends)| ends),
            change.open_join,
            change.locked,
            change.agent_mode.map(AgentMode::as_str),
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(title)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, meeting = %id))]
    pub async fn cancel(&self, actor: &Actor, id: Uuid) -> Result<String, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let access = access(&mut tx, actor, id).await?.owned()?;
        if access.live {
            return Err(IdentityError::Conflict);
        }
        sqlx::query!("DELETE FROM meetings WHERE id=$1", id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(access.title)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, meeting = %id))]
    pub async fn set_link(
        &self,
        actor: &Actor,
        id: Uuid,
        enabled: bool,
    ) -> Result<(MeetingLink, String), IdentityError> {
        let mut tx = self.pool.begin().await?;
        let access = access(&mut tx, actor, id).await?.owned()?;
        let token = enabled.then(token);
        sqlx::query!("UPDATE meetings SET link_token=$2 WHERE id=$1", id, token)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let link = MeetingLink {
            link: self.link(token),
        };
        Ok((link, access.title))
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn list(&self, actor: &Actor) -> Result<Vec<MeetingSummary>, IdentityError> {
        Ok(sqlx::query_as!(
            MeetingSummary,
            r#"
            WITH visible AS (
                SELECT m.*, row_number() OVER (
                    PARTITION BY m.ended_at IS NULL ORDER BY m.ended_at DESC
                ) AS recent
                FROM meetings m
                WHERE m.owner_id=$1 OR EXISTS(
                    SELECT 1 FROM meeting_invitations i WHERE i.meeting_id=m.id AND i.account_id=$1
                )
            )
            SELECT v.id, v.title, COALESCE(NULLIF(o.profile->>'name', ''), o.login) AS "owner_name!",
                CASE WHEN v.ended_at IS NOT NULL THEN 'ended'
                    WHEN v.started_at IS NOT NULL THEN 'live' ELSE 'scheduled' END AS "state!",
                (to_jsonb(v.starts_at) #>> '{}') AS starts_at, (to_jsonb(v.ends_at) #>> '{}') AS ends_at,
                (to_jsonb(v.started_at) #>> '{}') AS started_at, (to_jsonb(v.ended_at) #>> '{}') AS ended_at,
                (SELECT count(*) FROM meeting_invitations i
                    WHERE i.meeting_id=v.id AND i.agent_id IS NULL) AS "people!",
                (SELECT count(*) FROM meeting_invitations i
                    WHERE i.meeting_id=v.id AND i.agent_id IS NOT NULL) AS "agents!"
            FROM visible v
            JOIN human_accounts o ON o.id=v.owner_id
            WHERE (v.ended_at IS NOT NULL AND v.recent <= 50)
                OR (v.ended_at IS NULL AND (v.started_at IS NOT NULL OR v.starts_at IS NULL
                    OR v.starts_at >= now() - interval '1 day'))
            ORDER BY v.ended_at IS NOT NULL, v.started_at IS NULL, v.ended_at DESC,
                COALESCE(v.starts_at, v.created_at), v.id
            "#,
            actor.id,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, meeting = %id))]
    pub async fn meeting(&self, actor: &Actor, id: Uuid) -> Result<Meeting, IdentityError> {
        let row = sqlx::query!(
            r#"
            SELECT m.id, m.title, m.description, m.owner_id=$2 AS "owner!",
                COALESCE(NULLIF(o.profile->>'name', ''), o.login) AS "owner_name!",
                CASE WHEN m.ended_at IS NOT NULL THEN 'ended'
                    WHEN m.started_at IS NOT NULL THEN 'live' ELSE 'scheduled' END AS "state!",
                (to_jsonb(m.starts_at) #>> '{}') AS starts_at, (to_jsonb(m.ends_at) #>> '{}') AS ends_at,
                m.link_token, m.open_join, m.locked, m.agent_mode AS "agent_mode: AgentMode",
                (to_jsonb(m.started_at) #>> '{}') AS started_at, (to_jsonb(m.ended_at) #>> '{}') AS ended_at,
                m.summary, (to_jsonb(m.created_at) #>> '{}') AS "created_at!"
            FROM meetings m
            JOIN human_accounts o ON o.id=m.owner_id
            WHERE m.id=$1 AND (m.owner_id=$2 OR EXISTS(
                SELECT 1 FROM meeting_invitations i WHERE i.meeting_id=m.id AND i.account_id=$2
            ))
            "#,
            id,
            actor.id,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::NotFound)?;
        let (invitations, attendance, utterances) = tokio::try_join!(
            self.invitations(id),
            self.attendance(id),
            self.utterances(id)
        )?;
        Ok(Meeting {
            id: row.id,
            title: row.title,
            description: row.description,
            owner: row.owner,
            owner_name: row.owner_name,
            state: row.state,
            starts_at: row.starts_at,
            ends_at: row.ends_at,
            link: self.link(row.link_token),
            open_join: row.open_join,
            locked: row.locked,
            agent_mode: row.agent_mode,
            started_at: row.started_at,
            ended_at: row.ended_at,
            summary: row.summary,
            created_at: row.created_at,
            invitations,
            attendance,
            utterances,
        })
    }

    async fn invitations(&self, id: Uuid) -> Result<Vec<Invitation>, IdentityError> {
        Ok(sqlx::query_as!(
            Invitation,
            r#"
            SELECT i.id,
                CASE WHEN i.account_id IS NOT NULL THEN 'account'
                    WHEN i.agent_id IS NOT NULL THEN 'agent' ELSE 'email' END AS "kind!",
                i.account_id AS account, i.agent_id AS agent, i.email,
                COALESCE(NULLIF(h.profile->>'name', ''), h.login, g.name, i.email) AS "name!",
                g.avatar AS "avatar?: Json<Avatar>",
                (to_jsonb(i.emailed_at) #>> '{}') AS emailed_at, i.email_error
            FROM meeting_invitations i
            LEFT JOIN human_accounts h ON h.id=i.account_id
            LEFT JOIN agents g ON g.id=i.agent_id
            WHERE i.meeting_id=$1
            ORDER BY i.created_at, i.id
            "#,
            id,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn attendance(&self, id: Uuid) -> Result<Vec<Attendance>, IdentityError> {
        Ok(sqlx::query_as!(
            Attendance,
            r#"
            SELECT CASE WHEN a.account_id IS NOT NULL THEN 'account'
                    WHEN a.agent_id IS NOT NULL THEN 'agent' ELSE 'guest' END AS "kind!",
                a.account_id AS account, a.agent_id AS agent,
                COALESCE(NULLIF(h.profile->>'name', ''), h.login, g.name, a.guest_name || ' (guest)') AS "name!",
                (to_jsonb(a.joined_at) #>> '{}') AS "joined_at!", (to_jsonb(a.left_at) #>> '{}') AS left_at
            FROM meeting_attendance a
            LEFT JOIN human_accounts h ON h.id=a.account_id
            LEFT JOIN agents g ON g.id=a.agent_id
            WHERE a.meeting_id=$1
            ORDER BY a.joined_at, a.id
            "#,
            id,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn utterances(&self, id: Uuid) -> Result<Vec<Utterance>, IdentityError> {
        Ok(sqlx::query_as!(
            Utterance,
            r#"
            SELECT u.sequence,
                CASE WHEN u.account_id IS NOT NULL THEN 'account'
                    WHEN u.agent_id IS NOT NULL THEN 'agent' ELSE 'guest' END AS "kind!",
                COALESCE(NULLIF(h.profile->>'name', ''), h.login, g.name, u.guest_name || ' (guest)') AS "speaker!",
                u.text, (to_jsonb(u.at) #>> '{}') AS "at!"
            FROM meeting_utterances u
            LEFT JOIN human_accounts h ON h.id=u.account_id
            LEFT JOIN agents g ON g.id=u.agent_id
            WHERE u.meeting_id=$1
            ORDER BY u.sequence
            "#,
            id,
        )
        .fetch_all(&self.pool)
        .await?)
    }
}

async fn access(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    id: Uuid,
) -> Result<Access, IdentityError> {
    sqlx::query_as!(
        Access,
        r#"
        SELECT owner_id=$2 AS "owner!", (started_at IS NOT NULL AND ended_at IS NULL) AS "live!",
            ended_at IS NOT NULL AS "ended!", title
        FROM meetings m
        WHERE id=$1 AND (owner_id=$2 OR EXISTS(
            SELECT 1 FROM meeting_invitations i WHERE i.meeting_id=m.id AND i.account_id=$2
        ))
        FOR UPDATE
        "#,
        id,
        actor.id,
    )
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(IdentityError::NotFound)
}

fn token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn title(text: &str) -> Result<String, IdentityError> {
    let title = text.trim();
    let valid =
        (1..=TITLE_CHARS).contains(&title.chars().count()) && !title.chars().any(char::is_control);
    valid
        .then(|| title.to_owned())
        .ok_or(IdentityError::InvalidInput)
}

fn description(text: &str) -> Result<(), IdentityError> {
    (text.chars().count() <= DESCRIPTION_CHARS)
        .then_some(())
        .ok_or(IdentityError::InvalidInput)
}

fn schedule(starts: Option<&str>, ends: Option<&str>) -> Result<Option<(i64, i64)>, IdentityError> {
    let (starts, ends) = match (starts, ends) {
        (None, None) => return Ok(None),
        (Some(starts), Some(ends)) => (instant(starts)?, instant(ends)?),
        _ => return Err(IdentityError::InvalidInput),
    };
    (starts < ends)
        .then_some(Some((starts, ends)))
        .ok_or(IdentityError::InvalidInput)
}

fn instant(text: &str) -> Result<i64, IdentityError> {
    DateTime::parse_from_rfc3339(text)
        .map(|at| at.timestamp())
        .map_err(|error| IdentityError::Invalid(error.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_token_is_32_random_bytes_in_unpadded_base64url() {
        let (first, second) = (token(), token());
        assert_eq!(first.len(), 43);
        assert_ne!(first, second);
        let alphabet = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_';
        assert!(first.bytes().all(alphabet));
        assert_eq!(URL_SAFE_NO_PAD.decode(&first).unwrap().len(), 32);
    }

    #[test]
    fn a_schedule_has_both_ends_in_order_or_none() {
        assert_eq!(schedule(None, None).unwrap(), None);
        let start = "2026-11-16T10:00:00Z";
        let end = "2026-11-16T11:00:00+01:00";
        assert!(schedule(Some(start), Some(end)).is_err());
        assert_eq!(
            schedule(Some(start), Some("2026-11-16T11:00:00Z")).unwrap(),
            Some((1_794_823_200, 1_794_826_800))
        );
        assert!(schedule(Some(start), None).is_err());
        assert!(schedule(Some("tomorrow"), Some(start)).is_err());
    }

    #[test]
    fn a_title_is_trimmed_and_bounded() {
        assert_eq!(title("  Weekly sync ").unwrap(), "Weekly sync");
        assert!(title("   ").is_err());
        assert!(title(&"x".repeat(201)).is_err());
        assert!(title("a\nb").is_err());
    }
}

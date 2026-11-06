//! A person's profile: what they want agents (and the people they work with) to know about them.
//! Agents read it for the people who can reach them, and see the speaker's with every turn.
use super::{Actor, Identity, IdentityError};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Every field is optional; empty means not given.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    /// What to call them, such as "Victor".
    pub name: String,
    pub pronouns: String,
    /// What they do, such as "Platform engineer".
    pub role: String,
    /// An IANA zone such as `Europe/Lisbon`.
    pub time_zone: String,
    /// The language they want replies in.
    pub language: String,
    /// Free text: what agents should know about them.
    pub about: String,
}

/// Someone an agent works with, as the agent may read them.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Person {
    pub login: String,
    pub administrator: bool,
    pub profile: Profile,
}

const ABOUT_BYTES: usize = 4000;

impl Profile {
    fn valid(&self) -> bool {
        let short =
            |text: &str, most: usize| text.len() <= most && !text.chars().any(char::is_control);
        short(&self.name, 80)
            && short(&self.pronouns, 40)
            && short(&self.role, 120)
            && short(&self.language, 40)
            && short(&self.time_zone, 64)
            && self.about.len() <= ABOUT_BYTES
    }

    /// The profile as a few lines for an agent's instructions; `None` when nothing is given.
    fn introduction(&self, login: &str, local_time: Option<&str>) -> String {
        let name = if self.name.is_empty() {
            login
        } else {
            &self.name
        };
        let mut lines = vec![format!("This turn is for {name} (@{login}).")];
        let mut add = |label: &str, value: &str| {
            if !value.is_empty() {
                lines.push(format!("- {label}: {value}"));
            }
        };
        add("Pronouns", &self.pronouns);
        add("Role", &self.role);
        add("Prefers replies in", &self.language);
        add("Time zone", &self.time_zone);
        add("Their local time now", local_time.unwrap_or_default());
        if !self.about.trim().is_empty() {
            lines.push(format!(
                "What they want you to know, in their words:\n{}",
                self.about.trim()
            ));
        }
        lines.join("\n")
    }
}

impl Identity {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn profile(&self, actor: &Actor) -> Result<Profile, IdentityError> {
        let stored = sqlx::query_scalar!(
            r#"SELECT profile AS "profile: sqlx::types::Json<Profile>" FROM human_accounts WHERE id = $1 AND active"#,
            actor.id,
        )
        .fetch_optional(&self.pool)
        .await?;
        stored
            .map(|profile| profile.0)
            .ok_or(IdentityError::Unauthorized)
    }

    /// Replaces the caller's own profile. A time zone must be one PostgreSQL knows.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn set_profile(
        &self,
        actor: &Actor,
        profile: &Profile,
    ) -> Result<Profile, IdentityError> {
        if !profile.valid() {
            return Err(IdentityError::InvalidInput);
        }
        let zone = sqlx::query_scalar!(
            r#"SELECT $1 = '' OR EXISTS(SELECT 1 FROM pg_timezone_names WHERE name = $1) AS "known!""#,
            profile.time_zone,
        )
        .fetch_one(&self.pool)
        .await?;
        if !zone {
            return Err(IdentityError::InvalidInput);
        }
        let stored = sqlx::query!(
            "UPDATE human_accounts SET profile = $2 WHERE id = $1 AND active",
            actor.id,
            sqlx::types::Json(profile) as sqlx::types::Json<&Profile>,
        )
        .execute(&self.pool)
        .await?;
        if stored.rows_affected() == 0 {
            return Err(IdentityError::Unauthorized);
        }
        Ok(profile.clone())
    }
}

/// The active people who can reach an agent: administrators and anyone granted on it or its
/// conversation, directly or through a team.
#[tracing::instrument(skip_all, fields(agent = %agent))]
pub async fn people(pool: &sqlx::PgPool, agent: Uuid) -> Result<Vec<Person>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT a.login, a.role = 'administrator' AS "administrator!",
            a.profile AS "profile: sqlx::types::Json<Profile>"
        FROM human_accounts a
        WHERE a.active AND (
            a.role = 'administrator'
            OR EXISTS(
                SELECT 1 FROM effective_human_grants g
                WHERE g.actor_id = a.id AND g.scope_id = $1 AND g.scope_kind IN ('agent', 'conversation')
            )
        )
        ORDER BY a.login
        LIMIT 200
        "#,
        agent,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Person {
            login: row.login,
            administrator: row.administrator,
            profile: row.profile.0,
        })
        .collect())
}

/// Appends who the turn is for, so the agent knows who it is talking with.
pub async fn introduced(
    tx: &mut Transaction<'_, Postgres>,
    turn: Uuid,
    instructions: String,
) -> Result<String, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT a.login, a.profile AS "profile: sqlx::types::Json<Profile>",
            CASE WHEN EXISTS(SELECT 1 FROM pg_timezone_names z WHERE z.name = a.profile->>'time_zone')
                THEN to_char(clock_timestamp() AT TIME ZONE (a.profile->>'time_zone'), 'Dy YYYY-MM-DD HH24:MI')
            END AS local_time
        FROM chat_turns t
        JOIN human_accounts a ON a.id = t.actor_id
        WHERE t.id = $1
        "#,
        turn,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(match row {
        Some(row) => format!(
            "{instructions}\n\n## Who you are working with\n\n{}",
            row.profile
                .0
                .introduction(&row.login, row.local_time.as_deref())
        ),
        None => instructions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_profile_introduces_only_the_login() {
        let text = Profile::default().introduction("maya", None);
        assert_eq!(text, "This turn is for maya (@maya).");
    }

    #[test]
    fn a_full_profile_introduces_every_given_field() {
        let profile = Profile {
            name: "Maya".into(),
            pronouns: "she/her".into(),
            time_zone: "Europe/Lisbon".into(),
            about: "  Keep it short.  ".into(),
            ..Profile::default()
        };
        let text = profile.introduction("maya", Some("Fri 2026-11-06 21:04"));
        assert!(text.starts_with("This turn is for Maya (@maya)."));
        assert!(text.contains("- Pronouns: she/her"));
        assert!(text.contains("- Their local time now: Fri 2026-11-06 21:04"));
        assert!(text.ends_with("in their words:\nKeep it short."));
        assert!(!text.contains("Role"));
    }

    #[test]
    fn long_or_control_text_is_refused() {
        let too_long = Profile {
            about: "x".repeat(ABOUT_BYTES + 1),
            ..Profile::default()
        };
        assert!(!too_long.valid());
        let control = Profile {
            name: "a\u{7}".into(),
            ..Profile::default()
        };
        assert!(!control.valid());
        assert!(Profile::default().valid());
    }
}

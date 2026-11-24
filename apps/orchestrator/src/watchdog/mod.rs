//! The watchdog (docs/adr/agents.md#a-watchdog-that-notifies): rules over records the platform
//! already keeps open findings for administrators, and a few posture checks describe how the
//! installation is set up. It reads; it never blocks, pauses or stops an agent.
pub mod platform;
pub mod rules;
pub mod sandbox;
mod worker;

use crate::{
    api::Installation,
    identity::{Actor, IdentityError, audit},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
pub(crate) use worker::open;
pub use worker::{sweep, watch};

/// What the watchdog can read beyond PostgreSQL: each agent's traffic from its VMM, and the pods
/// of the installation's namespace. Either is absent where the installation has no such thing.
#[derive(Clone, Default)]
pub struct Reach {
    pub traffic: Option<sandbox::Probe>,
    pub pods: Option<platform::Pods>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Finding {
    pub id: Uuid,
    /// Such as `reverse_shell`, `secret_sent` or `sign_in_failures`.
    pub rule: String,
    /// `high`, `medium` (both notify administrators) or `low` (listed only).
    pub severity: String,
    pub agent: Option<Uuid>,
    pub agent_name: Option<String>,
    /// The latest turn that matched; its trace is the evidence.
    pub turn: Option<Uuid>,
    pub summary: String,
    /// What matched, with credential shapes masked.
    pub evidence: Option<String>,
    /// How many times it matched while open.
    pub count: i32,
    /// RFC 3339.
    pub opened_at: String,
    pub seen_at: String,
    pub dismissed_at: Option<String>,
    pub dismissed_by: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Mute {
    pub id: Uuid,
    pub rule: String,
    /// The agent it applies to; every agent when absent.
    pub agent: Option<Uuid>,
    pub agent_name: Option<String>,
    pub muted_by: String,
    pub muted_at: String,
}

/// One fact about how the installation is set up, true or worth a look.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Check {
    pub key: &'static str,
    pub ok: bool,
    pub title: String,
    pub detail: String,
    /// The console page that changes it.
    pub to: Option<&'static str>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Overview {
    /// Open findings, most severe first.
    pub findings: Vec<Finding>,
    /// Dismissed in the last 30 days, newest first, at most 50.
    pub dismissed: Vec<Finding>,
    pub mutes: Vec<Mute>,
    pub checks: Vec<Check>,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Dismiss {
    /// Also stop this rule opening findings for this agent, or for the installation when the
    /// finding has no agent.
    #[serde(default)]
    pub mute: bool,
}

#[derive(Clone)]
pub struct Watchdog {
    pool: PgPool,
}

impl Watchdog {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn overview(
        &self,
        actor: &Actor,
        installation: &Installation,
    ) -> Result<Overview, IdentityError> {
        crate::identity::administrator(&self.pool, actor.id).await?;
        Ok(Overview {
            findings: self.findings(true).await?,
            dismissed: self.findings(false).await?,
            mutes: self.mutes().await?,
            checks: checks(&self.facts().await?, installation),
        })
    }

    async fn findings(&self, open: bool) -> Result<Vec<Finding>, sqlx::Error> {
        sqlx::query_as!(
            Finding,
            r#"
            SELECT f.id, f.rule, f.severity, f.agent_id AS agent, g.name AS "agent_name?",
                   f.turn_id AS turn, f.summary, f.evidence, f.count,
                   (to_jsonb(f.opened_at) #>> '{}') AS "opened_at!", (to_jsonb(f.seen_at) #>> '{}') AS "seen_at!",
                   (to_jsonb(f.dismissed_at) #>> '{}') AS dismissed_at, h.login AS "dismissed_by?"
            FROM watchdog_findings f
            LEFT JOIN agents g ON g.id = f.agent_id
            LEFT JOIN human_accounts h ON h.id = f.dismissed_by
            WHERE CASE WHEN $1 THEN f.dismissed_at IS NULL
                       ELSE f.dismissed_at > clock_timestamp() - interval '30 days' END
            ORDER BY CASE WHEN $1 THEN array_position(ARRAY['high', 'medium', 'low'], f.severity) END,
                     COALESCE(f.dismissed_at, f.seen_at) DESC
            LIMIT CASE WHEN $1 THEN 200 ELSE 50 END
            "#,
            open,
        )
        .fetch_all(&self.pool)
        .await
    }

    async fn mutes(&self) -> Result<Vec<Mute>, sqlx::Error> {
        sqlx::query_as!(
            Mute,
            r#"
            SELECT m.id, m.rule, m.agent_id AS agent, g.name AS "agent_name?", h.login AS muted_by,
                   (to_jsonb(m.muted_at) #>> '{}') AS "muted_at!"
            FROM watchdog_mutes m
            JOIN human_accounts h ON h.id = m.muted_by
            LEFT JOIN agents g ON g.id = m.agent_id
            ORDER BY m.muted_at DESC
            "#
        )
        .fetch_all(&self.pool)
        .await
    }

    async fn facts(&self) -> Result<Facts, sqlx::Error> {
        sqlx::query_as!(
            Facts,
            r#"
            SELECT
                (SELECT count(*) FROM human_accounts WHERE role = 'administrator' AND active) AS "administrators!",
                (SELECT count(*) FROM connections WHERE status <> 'connected') AS "unhealthy_connections!",
                (SELECT count(*) FROM connections WHERE level = 'full' AND access->>'mode' = 'all_agents') AS "open_full_connections!",
                (SELECT extract(epoch FROM clock_timestamp() - max(finished_at))::bigint FROM backups
                 WHERE status IN ('complete', 'incomplete')) AS backup_age_seconds,
                (SELECT extract(epoch FROM clock_timestamp() - read_at)::bigint FROM watchdog_cursor) AS "read_seconds_ago!"
            "#
        )
        .fetch_one(&self.pool)
        .await
    }

    /// Settles a finding, and with `mute` stops its rule opening more for the same agent.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, finding = %finding))]
    pub async fn dismiss(
        &self,
        actor: &Actor,
        finding: Uuid,
        input: &Dismiss,
    ) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        crate::identity::administrator(&mut *tx, actor.id).await?;
        let settled = sqlx::query!(
            "
            UPDATE watchdog_findings SET dismissed_at = clock_timestamp(), dismissed_by = $2
            WHERE id = $1 AND dismissed_at IS NULL
            RETURNING rule, agent_id
            ",
            finding,
            actor.id,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        if input.mute {
            sqlx::query!(
                "INSERT INTO watchdog_mutes (rule, agent_id, muted_by) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                settled.rule,
                settled.agent_id,
                actor.id,
            )
            .execute(&mut *tx)
            .await?;
        }
        let mut event = audit::Event::by(actor.id, "watchdog.dismiss").detail(json!({
            "rule": settled.rule,
            "muted": input.mute,
            "scope": worker::scope(settled.agent_id),
        }));
        if let Some(agent) = settled.agent_id {
            event = event.on(agent);
        }
        audit::record(&mut *tx, &event).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Lets a muted rule open findings again.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, mute = %mute))]
    pub async fn unmute(&self, actor: &Actor, mute: Uuid) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        crate::identity::administrator(&mut *tx, actor.id).await?;
        let removed = sqlx::query!(
            "DELETE FROM watchdog_mutes WHERE id = $1 RETURNING rule, agent_id",
            mute,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        let mut event = audit::Event::by(actor.id, "watchdog.unmute").detail(json!({
            "rule": removed.rule,
            "scope": worker::scope(removed.agent_id),
        }));
        if let Some(agent) = removed.agent_id {
            event = event.on(agent);
        }
        audit::record(&mut *tx, &event).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// What the posture checks read, in one query.
#[derive(Debug, Default)]
pub struct Facts {
    pub administrators: i64,
    pub unhealthy_connections: i64,
    pub open_full_connections: i64,
    /// Since the newest verified backup finished; absent when there is none.
    pub backup_age_seconds: Option<i64>,
    pub read_seconds_ago: i64,
}

const DAY: i64 = 24 * 60 * 60;

pub fn checks(facts: &Facts, installation: &Installation) -> Vec<Check> {
    [
        Some(reading(facts)),
        sandbox(installation),
        Some(reach(facts, installation)),
        Some(administrators(facts)),
        Some(backups(facts)),
        Some(connections(facts)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn check(key: &'static str, ok: bool, title: &str, detail: String, to: &'static str) -> Check {
    Check {
        key,
        ok,
        title: title.to_owned(),
        detail,
        to: (!to.is_empty()).then_some(to),
    }
}

fn reading(facts: &Facts) -> Check {
    if facts.read_seconds_ago < 120 {
        return check(
            "watchdog",
            true,
            "The watchdog is reading",
            "It reads every tool call and audit entry within seconds of it being recorded.".into(),
            "",
        );
    }
    check(
        "watchdog",
        false,
        "The watchdog is behind",
        format!(
            "It last read the records {} ago. It runs only while this orchestrator owns the installation.",
            ago(facts.read_seconds_ago)
        ),
        "",
    )
}

fn sandbox(installation: &Installation) -> Option<Check> {
    use crate::agents::sandbox::Runtime;
    let (ok, detail) = match installation.runtime.as_ref()? {
        Runtime::Kvm {} => (
            true,
            "Each agent runs in its own KVM guest with its own kernel.".to_owned(),
        ),
        Runtime::Container { class: Some(class) } => (
            true,
            format!("Each agent runs in a container under the {class} runtime class."),
        ),
        Runtime::Container { class: None } => (
            false,
            "Agents run as plain containers and share the node's kernel. The kvm runtime, or a runtime class such as gVisor, contains a kernel exploit.".to_owned(),
        ),
    };
    Some(check(
        "sandbox",
        ok,
        "Sandbox isolation",
        detail,
        "/platform/health",
    ))
}

fn reach(facts: &Facts, installation: &Installation) -> Check {
    let open = facts.open_full_connections;
    let ok = open == 0 || !installation.terminal_enabled;
    let detail = if ok {
        "No agent can both run code and send as a connection open to every agent.".to_owned()
    } else {
        format!(
            "{open} {} at the Full level {} open to every agent, and agents can run code, so a misled agent can send as {}.",
            plural(open, "connection", "connections"),
            if open == 1 { "is" } else { "are" },
            if open == 1 { "it" } else { "them" },
        )
    };
    check(
        "reach",
        ok,
        "What one agent can reach",
        detail,
        "/platform/connections",
    )
}

fn administrators(facts: &Facts) -> Check {
    let ok = facts.administrators > 1;
    check(
        "administrators",
        ok,
        "Administrators",
        if ok {
            format!(
                "{} people can manage the installation.",
                facts.administrators
            )
        } else {
            "Only one person can manage the installation; if that account is lost, nobody can."
                .to_owned()
        },
        "/platform/people",
    )
}

fn backups(facts: &Facts) -> Check {
    let (ok, detail) = match facts.backup_age_seconds {
        None => (false, "No verified backup exists yet.".to_owned()),
        Some(age) if age > 2 * DAY => (
            false,
            format!("The newest verified backup is {} old.", ago(age)),
        ),
        Some(age) => (
            true,
            format!("The newest verified backup is {} old.", ago(age)),
        ),
    };
    check("backups", ok, "Backups", detail, "/platform/backup")
}

fn connections(facts: &Facts) -> Check {
    let unhealthy = facts.unhealthy_connections;
    check(
        "connections",
        unhealthy == 0,
        "Connections",
        if unhealthy == 0 {
            "Every connection is signed in and working.".to_owned()
        } else {
            format!(
                "{unhealthy} {} a new sign-in or {} failing.",
                plural(unhealthy, "connection needs", "connections need"),
                if unhealthy == 1 { "is" } else { "are" },
            )
        },
        "/platform/connections",
    )
}

fn plural<'a>(count: i64, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

fn ago(seconds: i64) -> String {
    if seconds < 120 {
        format!("{} seconds", seconds.max(0))
    } else if seconds < 2 * 3600 {
        format!("{} minutes", seconds / 60)
    } else if seconds < 2 * DAY {
        format!("{} hours", seconds / 3600)
    } else {
        format!("{} days", seconds / DAY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::sandbox::Runtime;

    fn installation(runtime: Option<Runtime>, terminal: bool) -> Installation {
        Installation {
            terminal_enabled: terminal,
            runtime,
            ..Installation::default()
        }
    }

    fn keyed(checks: &[Check], key: &str) -> bool {
        checks.iter().find(|check| check.key == key).unwrap().ok
    }

    #[test]
    fn posture_names_what_widens_an_agents_reach() {
        let facts = Facts {
            administrators: 1,
            open_full_connections: 2,
            unhealthy_connections: 1,
            backup_age_seconds: Some(3 * DAY),
            read_seconds_ago: 5,
        };
        let shared = installation(Some(Runtime::Container { class: None }), true);
        let all = checks(&facts, &shared);
        assert_eq!(
            all.iter()
                .map(|check| (check.key, check.ok))
                .collect::<Vec<_>>(),
            [
                ("watchdog", true),
                ("sandbox", false),
                ("reach", false),
                ("administrators", false),
                ("backups", false),
                ("connections", false)
            ]
        );
        let reach = all.iter().find(|check| check.key == "reach").unwrap();
        assert!(
            reach
                .detail
                .starts_with("2 connections at the Full level are"),
            "{}",
            reach.detail
        );
        assert!(all[4].detail.contains("3 days"), "{}", all[4].detail);

        let sound = Facts {
            administrators: 2,
            backup_age_seconds: Some(DAY / 2),
            ..facts
        };
        let guest = installation(Some(Runtime::Kvm {}), false);
        let all = checks(&sound, &guest);
        assert!(keyed(&all, "sandbox") && keyed(&all, "reach"));
        assert!(keyed(&all, "administrators") && keyed(&all, "backups"));
        assert!(
            checks(&sound, &installation(None, false))
                .iter()
                .all(|check| check.key != "sandbox")
        );
    }

    #[test]
    fn a_stalled_watchdog_says_so() {
        let stalled = Facts {
            read_seconds_ago: 600,
            ..Facts::default()
        };
        let check = reading(&stalled);
        assert!(
            !check.ok && check.detail.contains("10 minutes"),
            "{}",
            check.detail
        );
        assert_eq!(backups(&stalled).detail, "No verified backup exists yet.");
    }
}

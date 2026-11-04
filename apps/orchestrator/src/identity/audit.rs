//! The installation audit log (migrations/0044_audit_log.sql): who did what to which resource.
//! Command journals record their events in the command's own transaction; other changes record
//! theirs once they commit, through [`Identity::audit`]. Details never carry a secret.
use super::{Actor, Identity, IdentityError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

/// One thing that happened, before it is recorded.
pub struct Event {
    kind: &'static str,
    actor: Option<Uuid>,
    agent: Option<Uuid>,
    resource: Option<Uuid>,
    detail: Value,
}

impl Event {
    /// `kind` is `area.action`, such as `vault.create`.
    pub fn by(actor: Uuid, kind: &'static str) -> Self {
        Self {
            kind,
            actor: Some(actor),
            agent: None,
            resource: None,
            detail: json!({}),
        }
    }

    /// An agent acting on its own, outside any person's request.
    pub fn by_agent(agent: Uuid, kind: &'static str) -> Self {
        Self {
            agent: Some(agent),
            actor: None,
            ..Self::by(Uuid::nil(), kind)
        }
    }

    /// A change nobody signed in made, such as a refused sign-in.
    pub fn anonymous(kind: &'static str) -> Self {
        Self {
            actor: None,
            ..Self::by(Uuid::nil(), kind)
        }
    }

    pub fn on(mut self, resource: Uuid) -> Self {
        self.resource = Some(resource);
        self
    }

    /// Non-secret context: names, sites, modes; an object.
    pub fn detail(mut self, detail: Value) -> Self {
        self.detail = detail;
        self
    }
}

pub(crate) async fn record<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    event: &Event,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "
        INSERT INTO audit_log (actor_id, agent_id, kind, resource_id, detail)
        VALUES ($1, $2, $3, $4, $5)
        ",
        event.actor,
        event.agent,
        event.kind,
        event.resource,
        &event.detail,
    )
    .execute(executor)
    .await?;
    Ok(())
}

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct AuditQuery {
    /// Entries older than this sequence; the newest when absent.
    pub before: Option<i64>,
    /// Areas such as `vault` or `session`, comma-separated; every area when absent.
    pub area: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct AuditEntry {
    pub sequence: i64,
    pub at: String,
    pub kind: String,
    pub actor: Option<Uuid>,
    pub actor_name: Option<String>,
    pub agent: Option<Uuid>,
    pub agent_name: Option<String>,
    pub resource: Option<Uuid>,
    /// The resource's current name when it is an agent, account, team, room, vault item or MCP
    /// server that still exists.
    pub resource_name: Option<String>,
    #[schema(value_type = Object)]
    pub detail: Value,
}

/// Rows per page.
pub const PAGE: i64 = 100;

impl Identity {
    /// Records a change that has already committed. A failure is logged rather than returned:
    /// the change stands either way.
    // shortcut: recorded after commit, so a crash in between loses the entry; move the write
    // into the change's transaction where the log must be complete.
    pub async fn audit(&self, event: Event) {
        if let Err(error) = record(&self.pool, &event).await {
            tracing::error!(kind = event.kind, %error, "audit entry not recorded");
        }
    }

    /// Newest first, a page at a time; administrators only.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn audit_log(
        &self,
        actor: &Actor,
        query: &AuditQuery,
    ) -> Result<Vec<AuditEntry>, IdentityError> {
        self.require_administrator(actor).await?;
        let areas = areas(query.area.as_deref()).ok_or(IdentityError::InvalidInput)?;
        Ok(sqlx::query_as!(
            AuditEntry,
            r#"
            SELECT l.sequence, (to_jsonb(l.at) #>> '{}') AS "at!", l.kind,
                l.actor_id AS actor, h.login AS "actor_name?", l.agent_id AS agent,
                a.name AS "agent_name?", l.resource_id AS resource,
                COALESCE(ra.name, rh.login, rt.name, rr.name, rv.name, rm.name) AS resource_name,
                l.detail
            FROM audit_log l
            LEFT JOIN human_accounts h ON h.id = l.actor_id
            LEFT JOIN agents a ON a.id = l.agent_id
            LEFT JOIN agents ra ON ra.id = l.resource_id
            LEFT JOIN human_accounts rh ON rh.id = l.resource_id
            LEFT JOIN human_teams rt ON rt.id = l.resource_id
            LEFT JOIN rooms rr ON rr.id = l.resource_id
            LEFT JOIN vault_items rv ON rv.id = l.resource_id
            LEFT JOIN mcp_servers rm ON rm.id = l.resource_id
            WHERE l.sequence < COALESCE($1, 9223372036854775807)
                AND (cardinality($2::text[]) = 0 OR split_part(l.kind, '.', 1) = ANY($2))
            ORDER BY l.sequence DESC
            LIMIT $3
            "#,
            query.before,
            &areas,
            PAGE,
        )
        .fetch_all(&self.pool)
        .await?)
    }
}

/// The areas a filter names, at most 16 of lowercase letters and underscores; `None` when it
/// names anything else.
fn areas(filter: Option<&str>) -> Option<Vec<String>> {
    let Some(filter) = filter.filter(|filter| !filter.is_empty()) else {
        return Some(Vec::new());
    };
    let areas: Vec<String> = filter.split(',').map(str::to_owned).collect();
    let valid = |area: &String| {
        (1..=32).contains(&area.len())
            && area
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    };
    (areas.len() <= 16 && areas.iter().all(valid)).then_some(areas)
}

#[cfg(test)]
mod tests {
    use super::areas;

    #[test]
    fn a_filter_names_whole_areas_only() {
        assert_eq!(areas(None), Some(vec![]));
        assert_eq!(areas(Some("")), Some(vec![]));
        assert_eq!(
            areas(Some("account,team,oidc")),
            Some(vec!["account".into(), "team".into(), "oidc".into()])
        );
        for invalid in ["vault.%", "vault,", "Vault", "a b", "x%"] {
            assert_eq!(areas(Some(invalid)), None, "{invalid}");
        }
    }
}

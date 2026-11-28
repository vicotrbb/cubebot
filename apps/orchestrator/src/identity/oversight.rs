//! What administrators and auditors see across everyone: who owns what, and who spends what.

use serde::Serialize;
use uuid::Uuid;

use super::{Actor, Identity, IdentityError};

/// One agent, room, team or connection and its owner.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct OwnedResource {
    /// `agent`, `room`, `team` or `connection`.
    pub kind: String,
    pub id: Uuid,
    pub name: String,
    pub owner: Uuid,
    pub owner_login: String,
    /// False when the owner is suspended: the resource needs a new owner.
    pub owner_active: bool,
    /// Grants that share it with people or groups.
    pub shares: i64,
    /// A personal connection; installation connections are shared by administrators.
    pub personal: bool,
}

/// One person's agents' model spend over the period asked for.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct OwnerSpend {
    pub account: Uuid,
    pub login: String,
    pub agents: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// `None` when no price is known for any of the models used.
    pub cost_usd: Option<f64>,
}

impl Identity {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn inventory(&self, actor: &Actor) -> Result<Vec<OwnedResource>, IdentityError> {
        self.require_overseer(actor).await?;
        Ok(sqlx::query_as!(
            OwnedResource,
            r#"
            WITH owned AS (
                SELECT 'agent' AS kind, id, name, created_by AS owner, false AS personal
                FROM agents WHERE desired_state <> 'retired'
                UNION ALL
                SELECT 'room', r.id, r.name, r.created_by, false
                FROM rooms r
                WHERE NOT r.task AND NOT r.archived AND NOT EXISTS(SELECT 1 FROM teams t WHERE t.room_id = r.id)
                UNION ALL
                SELECT 'team', id, name, created_by, false FROM teams WHERE deleted_at IS NULL
                UNION ALL
                SELECT 'connection', id, slug, created_by, personal FROM connections
            )
            SELECT o.kind AS "kind!", o.id AS "id!", o.name AS "name!", o.owner AS "owner!",
                   h.login AS "owner_login!", h.active AS "owner_active!", o.personal AS "personal!",
                   (SELECT count(*) FROM access_grants g WHERE g.scope_id = o.id AND g.revoked_at IS NULL) AS "shares!"
            FROM owned o JOIN human_accounts h ON h.id = o.owner
            ORDER BY h.active, o.kind, lower(o.name)
            "#,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, days))]
    pub async fn spend_by_owner(
        &self,
        actor: &Actor,
        days: i32,
    ) -> Result<Vec<OwnerSpend>, IdentityError> {
        self.require_overseer(actor).await?;
        if !(1..=366).contains(&days) {
            return Err(IdentityError::InvalidInput);
        }
        Ok(sqlx::query_as!(
            OwnerSpend,
            r#"
            SELECT h.id AS "account!", h.login AS "login!",
                   count(DISTINCT g.id) AS "agents!",
                   COALESCE(sum(u.input_tokens), 0)::bigint AS "input_tokens!",
                   COALESCE(sum(u.output_tokens), 0)::bigint AS "output_tokens!",
                   sum(u.cost_usd) AS cost_usd
            FROM human_accounts h
            JOIN agents g ON g.created_by = h.id
            LEFT JOIN usage_events u ON u.agent_id = g.id AND u.created_at > clock_timestamp() - make_interval(days => $1)
            GROUP BY h.id, h.login
            ORDER BY sum(u.cost_usd) DESC NULLS LAST, h.login
            "#,
            days,
        )
        .fetch_all(&self.pool)
        .await?)
    }
}

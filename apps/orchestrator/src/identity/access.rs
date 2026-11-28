//! Exact scoped human permissions; agent execution grants are a different authority.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Actor, Identity, IdentityError, Receipt, commands::Command, groups::GroupId};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Scope {
    Agent { id: Uuid },
    Room { id: Uuid },
    Conversation { id: Uuid },
}

impl Scope {
    fn parts(self) -> (&'static str, Uuid) {
        match self {
            Self::Agent { id } => ("agent", id),
            Self::Room { id } => ("room", id),
            Self::Conversation { id } => ("conversation", id),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    AgentManage,
    ConversationRead,
    ConversationSend,
    MemoryRead,
    MemoryEdit,
    WorkspaceRead,
    WorkspaceWrite,
    BrowserView,
    BrowserControl,
    RoomManage,
}

impl Capability {
    fn name(self) -> &'static str {
        match self {
            Self::AgentManage => "agent.manage",
            Self::ConversationRead => "conversation.read",
            Self::ConversationSend => "conversation.send",
            Self::MemoryRead => "memory.read",
            Self::MemoryEdit => "memory.edit",
            Self::WorkspaceRead => "workspace.read",
            Self::WorkspaceWrite => "workspace.write",
            Self::BrowserView => "browser.view",
            Self::BrowserControl => "browser.control",
            Self::RoomManage => "room.manage",
        }
    }

    fn validate(self, scope: Scope) -> Result<(), IdentityError> {
        let valid = match self {
            Self::ConversationRead | Self::ConversationSend => {
                matches!(scope, Scope::Room { .. } | Scope::Conversation { .. })
            }
            Self::RoomManage => matches!(scope, Scope::Room { .. }),
            _ => matches!(scope, Scope::Agent { .. }),
        };
        if !valid || scope.parts().1.is_nil() {
            return Err(IdentityError::InvalidInput);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Recipient {
    User { id: Uuid },
    Group { id: GroupId },
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GrantRequest {
    pub request_id: Uuid,
    pub recipient: Recipient,
    pub scope: Scope,
    pub capability: Capability,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(transparent)]
#[sqlx(transparent)]
pub struct GrantId(pub Uuid);

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeGrant {
    pub request_id: Uuid,
    pub id: GrantId,
    pub expected_revision: i64,
}

/// A grant in force.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct GrantSummary {
    pub id: Uuid,
    pub revision: i64,
    pub account_id: Option<Uuid>,
    pub group_id: Option<Uuid>,
    /// `agent`, `room` or `conversation`; an agent's conversation has the agent's id.
    pub scope_kind: String,
    pub scope_id: Uuid,
    /// As in `Capability`, such as `conversation_send`.
    pub capability: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AccessSource {
    pub id: GrantId,
    pub group_id: Option<GroupId>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AccessExplanation {
    pub allowed: bool,
    pub administrator: bool,
    /// They own the resource.
    pub owner: bool,
    /// They are an auditor, who reads everything.
    pub auditor: bool,
    pub grants: Vec<AccessSource>,
}

/// Who owns a resource and the grants that share it.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ResourceAccess {
    pub owner: Option<Uuid>,
    pub grants: Vec<GrantSummary>,
}

impl Identity {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn grants(&self, actor: &Actor) -> Result<Vec<GrantSummary>, IdentityError> {
        self.require_overseer(actor).await?;
        Ok(sqlx::query_as!(
            GrantSummary,
            r#"
            SELECT id, revision, account_id, team_id AS group_id, scope_kind, scope_id,
                replace(capability,'.','_') AS "capability!"
            FROM access_grants
            WHERE revoked_at IS NULL
            "#,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id))]
    pub async fn grant(
        &self,
        actor: &Actor,
        input: &GrantRequest,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(actor, "access.grant", input.request_id, input)?;
        let (mut transaction, receipt) = command.begin_account(&self.pool).await?;
        if let Some(receipt) = receipt {
            return Ok(receipt);
        }
        input.capability.validate(input.scope)?;
        let (kind, scope_id) = input.scope.parts();
        may_share(
            &mut transaction,
            actor.id,
            kind,
            scope_id,
            Some(input.capability.name()),
        )
        .await?;
        let (account_id, team_id) = validate_recipient(&mut transaction, input.recipient).await?;
        let id = sqlx::query_scalar!(
            r#"
            INSERT INTO access_grants (account_id, team_id, scope_kind, scope_id, capability)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id AS "id!"
            "#,
            account_id,
            team_id.map(|team| team.0),
            kind,
            scope_id,
            input.capability.name(),
        )
        .fetch_one(&mut *transaction)
        .await?;
        command
            .commit(transaction, Receipt { id, revision: 1 })
            .await
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id, grant = %input.id.0))]
    pub async fn revoke_grant(
        &self,
        actor: &Actor,
        input: &RevokeGrant,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(actor, "access.revoke", input.request_id, input)?;
        let (mut transaction, receipt) = command.begin_account(&self.pool).await?;
        if let Some(receipt) = receipt {
            return Ok(receipt);
        }
        let scope = sqlx::query!(
            "SELECT scope_kind, scope_id FROM access_grants WHERE id = $1 AND revoked_at IS NULL",
            input.id.0,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IdentityError::Conflict)?;
        may_share(
            &mut transaction,
            actor.id,
            &scope.scope_kind,
            scope.scope_id,
            None,
        )
        .await?;
        let revision = sqlx::query_scalar!(
            r#"
            UPDATE access_grants
            SET revoked_at = clock_timestamp(), revision = revision + 1
            WHERE id = $1 AND revoked_at IS NULL AND revision = $2
            RETURNING revision AS "revision!"
            "#,
            input.id.0,
            input.expected_revision,
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let revision = revision.ok_or(IdentityError::Conflict)?;
        command
            .commit(
                transaction,
                Receipt {
                    id: input.id.0,
                    revision,
                },
            )
            .await
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn explain_access(
        &self,
        actor: &Actor,
        scope: Scope,
        capability: Capability,
    ) -> Result<AccessExplanation, IdentityError> {
        capability.validate(scope)?;
        let (kind, scope_id) = scope.parts();
        let rows = sqlx::query!(
            r#"
            SELECT
                a.role = 'administrator' AS "administrator!",
                g.id AS "id?: GrantId",
                g.team_id AS "group_id?: GroupId",
                g.source AS "source?"
            FROM human_accounts a
            LEFT JOIN effective_human_grants g
                ON g.actor_id = a.id
                AND g.scope_kind = $2
                AND g.scope_id = $3
                AND g.capability = $4
            WHERE a.id = $1 AND a.active
            ORDER BY g.id
            "#,
            actor.id,
            kind,
            scope_id,
            capability.name(),
        )
        .fetch_all(&self.pool)
        .await?;
        let administrator = rows
            .first()
            .ok_or(IdentityError::Unauthorized)?
            .administrator;
        let derived = |source: &str| rows.iter().any(|row| row.source.as_deref() == Some(source));
        let (owner, auditor) = (derived("owner"), derived("auditor"));
        let grants = rows
            .into_iter()
            .filter_map(|row| {
                row.id.map(|id| AccessSource {
                    id,
                    group_id: row.group_id,
                })
            })
            .collect::<Vec<_>>();
        Ok(AccessExplanation {
            allowed: administrator || owner || auditor || !grants.is_empty(),
            administrator,
            owner,
            auditor,
            grants,
        })
    }

    pub async fn authorize(
        &self,
        actor: &Actor,
        scope: Scope,
        capability: Capability,
    ) -> Result<(), IdentityError> {
        if !self.explain_access(actor, scope, capability).await?.allowed {
            return Err(IdentityError::Forbidden);
        }
        Ok(())
    }
}

impl Identity {
    /// The owner and the grants on one agent (with its conversation) or room, for the people
    /// who manage it and for overseers.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, resource = %resource))]
    pub async fn resource_access(
        &self,
        actor: &Actor,
        resource: Uuid,
    ) -> Result<ResourceAccess, IdentityError> {
        let mut transaction = self.pool.begin().await?;
        let kind = sqlx::query_scalar!(
            r#"SELECT CASE WHEN EXISTS(SELECT 1 FROM agents WHERE id = $1) THEN 'agent' ELSE 'room' END AS "kind!""#,
            resource,
        )
        .fetch_one(&mut *transaction)
        .await?;
        let oversees = sqlx::query_scalar!(
            r#"SELECT EXISTS(SELECT 1 FROM human_accounts WHERE id = $1 AND active AND role IN ('administrator', 'auditor')) AS "yes!""#,
            actor.id,
        )
        .fetch_one(&mut *transaction)
        .await?;
        if !oversees {
            may_share(&mut transaction, actor.id, &kind, resource, None).await?;
        }
        let owner = sqlx::query_scalar!(
            "SELECT created_by FROM agents WHERE id = $1 UNION ALL SELECT created_by FROM rooms WHERE id = $1 AND NOT task",
            resource,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .flatten();
        let grants = sqlx::query_as!(
            GrantSummary,
            r#"
            SELECT id, revision, account_id, team_id AS group_id, scope_kind, scope_id,
                replace(capability,'.','_') AS "capability!"
            FROM access_grants
            WHERE revoked_at IS NULL AND scope_id = $1
            "#,
            resource,
        )
        .fetch_all(&mut *transaction)
        .await?;
        Ok(ResourceAccess { owner, grants })
    }
}

/// Administrators share anything. Otherwise the actor must manage the resource (agent.manage
/// on the agent behind it, or room.manage on the room) and, when granting, hold the capability
/// they hand out, so nobody passes on more than they have.
async fn may_share(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Uuid,
    kind: &str,
    scope: Uuid,
    capability: Option<&str>,
) -> Result<(), IdentityError> {
    let allowed = sqlx::query_scalar!(
        r#"
        WITH target AS (
            SELECT CASE $2
                WHEN 'room' THEN $3
                ELSE COALESCE((SELECT id FROM agents WHERE id = $3), (SELECT agent_id FROM conversation_origins WHERE id = $3))
            END AS id,
            CASE $2 WHEN 'room' THEN 'room' ELSE 'agent' END AS kind,
            CASE $2 WHEN 'room' THEN 'room.manage' ELSE 'agent.manage' END AS capability
        )
        SELECT EXISTS(
            SELECT 1 FROM human_accounts a, target t
            WHERE a.id = $1 AND a.active AND (
                a.role = 'administrator'
                OR (a.role = 'member'
                    AND EXISTS(SELECT 1 FROM effective_human_grants p
                        WHERE p.actor_id = a.id AND p.scope_kind = t.kind AND p.scope_id = t.id AND p.capability = t.capability)
                    AND ($4::text IS NULL OR EXISTS(SELECT 1 FROM effective_human_grants p
                        WHERE p.actor_id = a.id AND p.scope_kind = $2 AND p.scope_id = $3 AND p.capability = $4))))
        ) AS "allowed!"
        "#,
        actor,
        kind,
        scope,
        capability,
    )
    .fetch_one(&mut **transaction)
    .await?;
    if !allowed {
        return Err(IdentityError::Forbidden);
    }
    Ok(())
}

async fn validate_recipient(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    recipient: Recipient,
) -> Result<(Option<Uuid>, Option<GroupId>), IdentityError> {
    let (exists, account, team) = match recipient {
        Recipient::User { id } => {
            let exists = sqlx::query_scalar!(
                r#"
                SELECT id AS "id!"
                FROM human_accounts
                WHERE id = $1 AND active
                FOR SHARE
                "#,
                id,
            )
            .fetch_optional(&mut **transaction)
            .await?;
            (exists.is_some(), Some(id), None)
        }
        Recipient::Group { id } => {
            let exists = sqlx::query_scalar!(
                r#"
                SELECT id AS "id!"
                FROM human_teams
                WHERE id = $1 AND active
                FOR SHARE
                "#,
                id.0,
            )
            .fetch_optional(&mut **transaction)
            .await?;
            (exists.is_some(), None, Some(id))
        }
    };
    if !exists {
        return Err(IdentityError::InvalidInput);
    }
    Ok((account, team))
}

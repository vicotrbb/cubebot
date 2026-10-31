//! Exact scoped human permissions; agent execution grants are a different authority.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Actor, Identity, IdentityError, Receipt, commands::Command, teams::TeamId};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Recipient {
    User { id: Uuid },
    Team { id: TeamId },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantRequest {
    pub request_id: Uuid,
    pub recipient: Recipient,
    pub scope: Scope,
    pub capability: Capability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(transparent)]
#[sqlx(transparent)]
pub struct GrantId(pub Uuid);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeGrant {
    pub request_id: Uuid,
    pub id: GrantId,
    pub expected_revision: i64,
}

#[derive(Debug, Serialize)]
pub struct AccessSource {
    pub id: GrantId,
    pub team_id: Option<TeamId>,
}

#[derive(Debug, Serialize)]
pub struct AccessExplanation {
    pub allowed: bool,
    pub administrator: bool,
    pub grants: Vec<AccessSource>,
}

impl Identity {
    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id))]
    pub async fn grant(
        &self,
        actor: &Actor,
        input: &GrantRequest,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(actor, "access.grant", input.request_id, input)?;
        let (mut transaction, receipt) = command.begin(&self.pool).await?;
        if let Some(receipt) = receipt {
            return Ok(receipt);
        }
        input.capability.validate(input.scope)?;
        let (account_id, team_id) = validate_recipient(&mut transaction, input.recipient).await?;
        let (kind, scope_id) = input.scope.parts();
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
        let (mut transaction, receipt) = command.begin(&self.pool).await?;
        if let Some(receipt) = receipt {
            return Ok(receipt);
        }
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
                g.team_id AS "team_id?: TeamId"
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
        let grants = rows
            .into_iter()
            .filter_map(|row| {
                row.id.map(|id| AccessSource {
                    id,
                    team_id: row.team_id,
                })
            })
            .collect::<Vec<_>>();
        Ok(AccessExplanation {
            allowed: administrator || !grants.is_empty(),
            administrator,
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

async fn validate_recipient(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    recipient: Recipient,
) -> Result<(Option<Uuid>, Option<TeamId>), IdentityError> {
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
        Recipient::Team { id } => {
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

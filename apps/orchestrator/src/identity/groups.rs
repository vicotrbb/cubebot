use serde::{Deserialize, Serialize};
use uuid::Uuid;

use sqlx::{Postgres, Transaction};

use super::{Actor, Identity, IdentityError, Receipt, Role, commands::Command, members};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(transparent)]
#[sqlx(transparent)]
pub struct GroupId(pub Uuid);

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateGroup {
    pub request_id: Uuid,
    pub name: String,
    /// Where its members come from; `local` when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<GroupSource>,
    /// A Google group's address, or the value of the single sign-on groups claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GroupSource {
    /// Members are added in Cubebot.
    Local,
    /// Mirrored from a Google Workspace group.
    Google,
    /// Filled at each single sign-on from the groups claim.
    Oidc,
}

impl GroupSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Google => "google",
            Self::Oidc => "oidc",
        }
    }
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupRole {
    pub request_id: Uuid,
    pub group: GroupId,
    /// The role its members get; `None` leaves their roles alone.
    pub role: Option<Role>,
    pub expected_revision: i64,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MembershipChange {
    pub request_id: Uuid,
    pub group: GroupId,
    pub account_id: Uuid,
    pub expected_revision: i64,
    pub present: bool,
    /// Makes them a manager of the group's members, or not; administrators only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manager: Option<bool>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    pub revision: i64,
    pub active: bool,
    /// The accounts in the group.
    pub members: Vec<Uuid>,
    /// Members who may change who is in it.
    pub managers: Vec<Uuid>,
    pub source: String,
    pub external_id: Option<String>,
    pub role: Option<String>,
    pub synced_at_unix: Option<i64>,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteGroup {
    pub request_id: Uuid,
    pub group: GroupId,
    pub expected_revision: i64,
}

impl Identity {
    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id))]
    pub async fn create_group(
        &self,
        actor: &Actor,
        input: &CreateGroup,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(actor, "team.create", input.request_id, input)?;
        let (mut transaction, receipt) = command.begin(&self.pool).await?;
        if let Some(receipt) = receipt {
            return Ok(receipt);
        }
        let name = input.name.trim();
        if name.is_empty() || name.len() > 100 || name.chars().any(char::is_control) {
            return Err(IdentityError::InvalidInput);
        }
        let source = input.source.unwrap_or(GroupSource::Local);
        let external = input
            .external_id
            .as_deref()
            .map(str::trim)
            .filter(|external| !external.is_empty());
        let valid_external = external.is_none_or(|external| {
            external.len() <= 320 && !external.chars().any(char::is_control)
        });
        if (source == GroupSource::Local) != external.is_none() || !valid_external {
            return Err(IdentityError::InvalidInput);
        }
        let external = match source {
            GroupSource::Google => external.map(str::to_ascii_lowercase),
            _ => external.map(str::to_owned),
        };
        let id = sqlx::query_scalar!(
            r#"
            INSERT INTO human_teams (name, source, external_id)
            VALUES ($1, $2, $3)
            RETURNING id AS "id!"
            "#,
            name,
            source.as_str(),
            external,
        )
        .fetch_one(&mut *transaction)
        .await?;
        command
            .commit(transaction, Receipt { id, revision: 1 })
            .await
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id, group = %input.group.0))]
    pub async fn change_membership(
        &self,
        actor: &Actor,
        input: &MembershipChange,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(actor, "team.membership", input.request_id, input)?;
        let (mut transaction, receipt) = command.begin_account(&self.pool).await?;
        if let Some(receipt) = receipt {
            return Ok(receipt);
        }
        may_change_members(&mut transaction, actor, input).await?;
        let revision = revise_group(&mut transaction, input.group, input.expected_revision).await?;
        let exists = sqlx::query_scalar!(
            r#"
            SELECT id AS "id!"
            FROM human_accounts
            WHERE id = $1 AND (active OR NOT $2)
            FOR SHARE
            "#,
            input.account_id,
            input.present,
        )
        .fetch_optional(&mut *transaction)
        .await?;
        exists.ok_or(IdentityError::InvalidInput)?;
        if input.present {
            sqlx::query!(
                "
                INSERT INTO human_team_members (team_id, account_id, manager)
                VALUES ($1, $2, COALESCE($3, false))
                ON CONFLICT (team_id, account_id) DO UPDATE SET manager = COALESCE($3, human_team_members.manager)
                ",
                input.group.0,
                input.account_id,
                input.manager,
            )
            .execute(&mut *transaction)
            .await?;
        } else {
            sqlx::query!(
                "
                DELETE FROM human_team_members
                WHERE team_id = $1 AND account_id = $2
                ",
                input.group.0,
                input.account_id,
            )
            .execute(&mut *transaction)
            .await?;
        }
        apply_roles(&mut transaction, &[input.account_id]).await?;
        command
            .commit(
                transaction,
                Receipt {
                    id: input.group.0,
                    revision,
                },
            )
            .await
    }

    /// Gives a group's members a role, or stops doing so. Only the installation owner maps a
    /// group to administrators.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id, group = %input.group.0))]
    pub async fn set_group_role(
        &self,
        actor: &Actor,
        input: &GroupRole,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(actor, "team.role", input.request_id, input)?;
        let (mut transaction, receipt) = command.begin(&self.pool).await?;
        if let Some(receipt) = receipt {
            return Ok(receipt);
        }
        let current = sqlx::query_scalar!(
            "SELECT role FROM human_teams WHERE id = $1 AND active FOR UPDATE",
            input.group.0,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IdentityError::NotFound)?;
        let touches_admins =
            input.role == Some(Role::Administrator) || current.as_deref() == Some("administrator");
        if touches_admins && !members::is_owner(&mut transaction, actor.id).await? {
            return Err(IdentityError::Forbidden);
        }
        let revision = revise_group(&mut transaction, input.group, input.expected_revision).await?;
        sqlx::query!(
            "UPDATE human_teams SET role = $2 WHERE id = $1",
            input.group.0,
            input.role.map(Role::as_str),
        )
        .execute(&mut *transaction)
        .await?;
        let accounts = sqlx::query_scalar!(
            "SELECT account_id FROM human_team_members WHERE team_id = $1",
            input.group.0,
        )
        .fetch_all(&mut *transaction)
        .await?;
        apply_roles(&mut transaction, &accounts).await?;
        command
            .commit(
                transaction,
                Receipt {
                    id: input.group.0,
                    revision,
                },
            )
            .await
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id, group = %input.group.0))]
    pub async fn delete_group(
        &self,
        actor: &Actor,
        input: &DeleteGroup,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(actor, "team.delete", input.request_id, input)?;
        let (mut transaction, receipt) = command.begin(&self.pool).await?;
        if let Some(receipt) = receipt {
            return Ok(receipt);
        }
        let revision = sqlx::query_scalar!(
            r#"
            UPDATE human_teams
            SET active = false, revision = revision + 1
            WHERE id = $1 AND active AND revision = $2
            RETURNING revision AS "revision!"
            "#,
            input.group.0,
            input.expected_revision,
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let revision = revision.ok_or(IdentityError::Conflict)?;
        let accounts = sqlx::query_scalar!(
            "SELECT account_id FROM human_team_members WHERE team_id = $1",
            input.group.0,
        )
        .fetch_all(&mut *transaction)
        .await?;
        apply_roles(&mut transaction, &accounts).await?;
        command
            .commit(
                transaction,
                Receipt {
                    id: input.group.0,
                    revision,
                },
            )
            .await
    }

    /// Every group for administrators and auditors; members see the groups they manage.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn groups(
        &self,
        actor: &Actor,
        after: Option<GroupId>,
    ) -> Result<Vec<Group>, IdentityError> {
        let everything = matches!(actor.role, Role::Administrator | Role::Auditor);
        Ok(sqlx::query_as!(
            Group,
            r#"
            SELECT id AS "id!: GroupId", name AS "name!", revision AS "revision!", active AS "active!",
                COALESCE((SELECT array_agg(m.account_id) FROM human_team_members m WHERE m.team_id=t.id),'{}') AS "members!",
                COALESCE((SELECT array_agg(m.account_id) FROM human_team_members m WHERE m.team_id=t.id AND m.manager),'{}') AS "managers!",
                source AS "source!", external_id, role,
                EXTRACT(EPOCH FROM synced_at)::bigint AS synced_at_unix
            FROM human_teams t
            WHERE ($1::uuid IS NULL OR id > $1)
              AND ($2 OR EXISTS(SELECT 1 FROM human_team_members m WHERE m.team_id=t.id AND m.account_id=$3 AND m.manager))
            ORDER BY id
            LIMIT 100
            "#,
            after.map(|after| after.0),
            everything,
            actor.id,
        )
        .fetch_all(&self.pool)
        .await?)
    }
}

async fn revise_group(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: GroupId,
    expected: i64,
) -> Result<i64, IdentityError> {
    let revision = sqlx::query_scalar!(
        r#"
        UPDATE human_teams
        SET revision = revision + 1
        WHERE id = $1 AND active AND revision = $2
        RETURNING revision AS "revision!"
        "#,
        id.0,
        expected,
    )
    .fetch_optional(&mut **transaction)
    .await?;
    revision.ok_or(IdentityError::Conflict)
}

/// Administrators change any local group's members and its managers. A group's managers add
/// and remove its members, unless the group carries a role. Mirrored groups follow their source.
async fn may_change_members(
    transaction: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    input: &MembershipChange,
) -> Result<(), IdentityError> {
    let group = sqlx::query!(
        r#"
        SELECT t.source, t.role,
               EXISTS(SELECT 1 FROM human_team_members m WHERE m.team_id = t.id AND m.account_id = $2 AND m.manager) AS "manages!"
        FROM human_teams t
        WHERE t.id = $1 AND t.active
        "#,
        input.group.0,
        actor.id,
    )
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(IdentityError::NotFound)?;
    if group.source != "local" {
        return Err(IdentityError::Conflict);
    }
    let allowed = match actor.role {
        Role::Administrator => true,
        Role::Member => group.manages && group.role.is_none() && input.manager.is_none(),
        Role::Auditor | Role::Guest => false,
    };
    if !allowed
        || (actor.role == Role::Administrator && !actor_active_admin(transaction, actor).await?)
    {
        return Err(IdentityError::Forbidden);
    }
    Ok(())
}

async fn actor_active_admin(
    transaction: &mut Transaction<'_, Postgres>,
    actor: &Actor,
) -> Result<bool, IdentityError> {
    Ok(sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM human_accounts WHERE id = $1 AND active AND role = 'administrator') AS "yes!""#,
        actor.id,
    )
    .fetch_one(&mut **transaction)
    .await?)
}

/// Sets these accounts' roles from the role-carrying groups they are in. An account in none
/// keeps its own role, or returns to member if a group gave it the one it has. The installation
/// owner's role never changes.
pub(crate) async fn apply_roles(
    transaction: &mut Transaction<'_, Postgres>,
    accounts: &[Uuid],
) -> Result<(), IdentityError> {
    sqlx::query!(
        r#"
        WITH mapped AS (
            SELECT a.id,
                (SELECT t.role FROM human_team_members m JOIN human_teams t ON t.id = m.team_id AND t.active AND t.role IS NOT NULL
                 WHERE m.account_id = a.id
                 ORDER BY array_position(ARRAY['administrator','auditor','member','guest'], t.role) LIMIT 1) AS role
            FROM human_accounts a
            WHERE a.id = ANY($1) AND a.id IS DISTINCT FROM (SELECT owner_id FROM installation WHERE singleton)
        )
        UPDATE human_accounts a
        SET role = COALESCE(mapped.role, 'member'), role_from_group = mapped.role IS NOT NULL,
            revision = a.revision + 1
        FROM mapped
        WHERE a.id = mapped.id
          AND (mapped.role IS NOT NULL OR a.role_from_group)
          AND (a.role IS DISTINCT FROM COALESCE(mapped.role, 'member') OR a.role_from_group IS DISTINCT FROM (mapped.role IS NOT NULL))
        "#,
        accounts,
    )
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

/// Makes `accounts` exactly the members of a mirrored group and updates roles of everyone
/// who joined or left.
pub(crate) async fn sync_members(
    transaction: &mut Transaction<'_, Postgres>,
    group: Uuid,
    accounts: &[Uuid],
) -> Result<(), IdentityError> {
    let left = sqlx::query_scalar!(
        "DELETE FROM human_team_members WHERE team_id = $1 AND account_id <> ALL($2) RETURNING account_id",
        group,
        accounts,
    )
    .fetch_all(&mut **transaction)
    .await?;
    sqlx::query!(
        "INSERT INTO human_team_members (team_id, account_id) SELECT $1, unnest($2::uuid[]) ON CONFLICT DO NOTHING",
        group,
        accounts,
    )
    .execute(&mut **transaction)
    .await?;
    sqlx::query!(
        "UPDATE human_teams SET synced_at = clock_timestamp() WHERE id = $1",
        group,
    )
    .execute(&mut **transaction)
    .await?;
    let touched: Vec<Uuid> = left.into_iter().chain(accounts.iter().copied()).collect();
    apply_roles(transaction, &touched).await
}

/// At single sign-on: puts the account in exactly the groups-claim groups it names.
#[doc(hidden)]
pub async fn sync_claimed(
    transaction: &mut Transaction<'_, Postgres>,
    account: Uuid,
    claimed: &[String],
) -> Result<(), IdentityError> {
    sqlx::query!(
        r#"
        DELETE FROM human_team_members m USING human_teams t
        WHERE m.team_id = t.id AND t.source = 'oidc' AND m.account_id = $1 AND t.external_id <> ALL($2)
        "#,
        account,
        claimed,
    )
    .execute(&mut **transaction)
    .await?;
    sqlx::query!(
        r#"
        INSERT INTO human_team_members (team_id, account_id)
        SELECT id, $1 FROM human_teams WHERE source = 'oidc' AND active AND external_id = ANY($2)
        ON CONFLICT DO NOTHING
        "#,
        account,
        claimed,
    )
    .execute(&mut **transaction)
    .await?;
    apply_roles(transaction, &[account]).await
}

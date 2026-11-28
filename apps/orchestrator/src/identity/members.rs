//! Roles, what members may create on their own, and who owns what.
//!
//! Owners hold every capability on what they own through `effective_human_grants`; the
//! installation owner is the one administrator other administrators cannot demote or suspend.

use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::{Actor, Identity, IdentityError, Receipt, commands::Command};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Runs the installation and reads everything.
    Administrator,
    /// Reads everything and changes nothing.
    Auditor,
    /// Creates and shares their own agents, rooms, teams and connections, as the policy allows.
    Member,
    /// Uses only what is shared with them.
    Guest,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Administrator => "administrator",
            Self::Auditor => "auditor",
            Self::Member => "member",
            Self::Guest => "guest",
        }
    }

    /// An unknown stored role gets the least authority.
    pub fn parse(text: &str) -> Self {
        match text {
            "administrator" => Self::Administrator,
            "auditor" => Self::Auditor,
            "member" => Self::Member,
            _ => Self::Guest,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Creation {
    Agent,
    Room,
    Team,
    Connection,
    Provider,
}

/// What members may do without an administrator. Administrators always may; auditors and
/// guests never create anything.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub members_create_agents: bool,
    pub members_create_rooms: bool,
    pub members_create_teams: bool,
    pub members_create_connections: bool,
    /// Members may add their own AI provider keys and plans for their own agents.
    pub members_bring_providers: bool,
    /// Live agents one member may own; `None` is no limit.
    pub member_agent_limit: Option<i32>,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SavePolicy {
    pub request_id: Uuid,
    pub policy: Policy,
}

/// What the signed-in person may create right now.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CanCreate {
    pub agents: bool,
    pub rooms: bool,
    pub teams: bool,
    pub connections: bool,
    pub providers: bool,
}

/// The signed-in person's place in the installation.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Standing {
    pub role: Role,
    /// The installation owner.
    pub owner: bool,
    pub can_create: CanCreate,
    /// Agents this person owns that are not retired, against `member_agent_limit`.
    pub owned_agents: i64,
    /// One active account and no single sign-on: sharing has no one to share with.
    pub solo: bool,
    /// Their sign-in asks for a code from an authenticator app.
    pub two_factor: bool,
    /// Live agents a member may own; `None` is no limit, and administrators have none.
    pub agent_limit: Option<i32>,
}

struct Row {
    role: String,
    owner: bool,
    two_factor: bool,
    owned: i64,
    solo: bool,
    policy: Policy,
}

impl Row {
    fn allows(&self, what: Creation) -> bool {
        let policy = &self.policy;
        match Role::parse(&self.role) {
            Role::Administrator => true,
            Role::Member => match what {
                Creation::Agent => {
                    policy.members_create_agents
                        && policy
                            .member_agent_limit
                            .is_none_or(|limit| self.owned < i64::from(limit))
                }
                Creation::Room => policy.members_create_rooms,
                Creation::Team => policy.members_create_teams,
                Creation::Connection => policy.members_create_connections,
                Creation::Provider => policy.members_bring_providers,
            },
            Role::Auditor | Role::Guest => false,
        }
    }
}

async fn standing_row(
    executor: impl sqlx::PgExecutor<'_>,
    actor: Uuid,
) -> Result<Row, IdentityError> {
    let row = sqlx::query!(
        r#"
        SELECT a.role,
               i.owner_id IS NOT DISTINCT FROM a.id AS "owner!",
               a.totp_enabled_at IS NOT NULL AS "two_factor!",
               (SELECT count(*) FROM agents g WHERE g.created_by = a.id AND g.desired_state <> 'retired') AS "owned!",
               (SELECT count(*) FROM human_accounts o WHERE o.active) = 1
                   AND NOT EXISTS(SELECT 1 FROM identity_oidc_policy p WHERE p.singleton AND p.enabled) AS "solo!",
               s.members_create_agents, s.members_create_rooms, s.members_create_teams,
               s.members_create_connections, s.members_bring_providers, s.member_agent_limit
        FROM human_accounts a
        CROSS JOIN installation i
        CROSS JOIN installation_settings s
        WHERE a.id = $1 AND a.active
        "#,
        actor,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(IdentityError::Unauthorized)?;
    Ok(Row {
        role: row.role,
        owner: row.owner,
        two_factor: row.two_factor,
        owned: row.owned,
        solo: row.solo,
        policy: Policy {
            members_create_agents: row.members_create_agents,
            members_create_rooms: row.members_create_rooms,
            members_create_teams: row.members_create_teams,
            members_create_connections: row.members_create_connections,
            members_bring_providers: row.members_bring_providers,
            member_agent_limit: row.member_agent_limit,
        },
    })
}

/// Refuses with `Forbidden` unless the actor may create this now. Agent creation runs with the
/// actor's row locked, so two concurrent creations cannot both pass the agent limit.
pub(crate) async fn may_create(
    executor: impl sqlx::PgExecutor<'_>,
    actor: Uuid,
    what: Creation,
) -> Result<(), IdentityError> {
    if !standing_row(executor, actor).await?.allows(what) {
        return Err(IdentityError::Forbidden);
    }
    Ok(())
}

/// The kinds of resource that have an owner who can hand it on.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Owned {
    Agent,
    Room,
    Team,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Transfer {
    pub request_id: Uuid,
    pub kind: Owned,
    pub id: Uuid,
    /// The new owner: an active administrator or member.
    pub to: Uuid,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TransferAll {
    pub request_id: Uuid,
    pub from: Uuid,
    pub to: Uuid,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeRole {
    pub request_id: Uuid,
    pub id: Uuid,
    pub role: Role,
    pub expected_revision: i64,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Reactivate {
    pub request_id: Uuid,
    pub id: Uuid,
    pub expected_revision: i64,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HandOver {
    pub request_id: Uuid,
    /// An active administrator who becomes the installation owner.
    pub to: Uuid,
}

/// Someone a member can share with.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DirectoryPerson {
    pub id: Uuid,
    pub login: String,
    pub name: Option<String>,
    pub role: Role,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DirectoryGroup {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct Directory {
    pub people: Vec<DirectoryPerson>,
    pub groups: Vec<DirectoryGroup>,
}

impl Identity {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn standing(&self, actor: &Actor) -> Result<Standing, IdentityError> {
        let row = standing_row(&self.pool, actor.id).await?;
        Ok(Standing {
            role: Role::parse(&row.role),
            owner: row.owner,
            can_create: CanCreate {
                agents: row.allows(Creation::Agent),
                rooms: row.allows(Creation::Room),
                teams: row.allows(Creation::Team),
                connections: row.allows(Creation::Connection),
                providers: row.allows(Creation::Provider),
            },
            owned_agents: row.owned,
            solo: row.solo,
            two_factor: row.two_factor,
            agent_limit: match Role::parse(&row.role) {
                Role::Member => row.policy.member_agent_limit,
                _ => None,
            },
        })
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn policy(&self, actor: &Actor) -> Result<Policy, IdentityError> {
        self.require_overseer(actor).await?;
        Ok(standing_row(&self.pool, actor.id).await?.policy)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id))]
    pub async fn save_policy(
        &self,
        actor: &Actor,
        input: &SavePolicy,
    ) -> Result<Receipt, IdentityError> {
        let policy = &input.policy;
        if policy
            .member_agent_limit
            .is_some_and(|limit| !(0..=10000).contains(&limit))
        {
            return Err(IdentityError::InvalidInput);
        }
        let command = Command::new(actor, "policy.save", input.request_id, input)?;
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        let revision = sqlx::query_scalar!(
            r#"
            UPDATE installation_settings
            SET members_create_agents = $1, members_create_rooms = $2, members_create_teams = $3,
                members_create_connections = $4, members_bring_providers = $5,
                member_agent_limit = $6, revision = revision + 1
            WHERE singleton
            RETURNING revision AS "revision!"
            "#,
            policy.members_create_agents,
            policy.members_create_rooms,
            policy.members_create_teams,
            policy.members_create_connections,
            policy.members_bring_providers,
            policy.member_agent_limit,
        )
        .fetch_one(&mut *transaction)
        .await?;
        let id = installation_id(&mut transaction).await?;
        command.commit(transaction, Receipt { id, revision }).await
    }

    /// Everyone active and every group, for choosing whom to share with. Guests share nothing.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn directory(&self, actor: &Actor) -> Result<Directory, IdentityError> {
        if actor.role == Role::Guest {
            return Err(IdentityError::Forbidden);
        }
        let people = sqlx::query!(
            r#"
            SELECT id, login, NULLIF(profile->>'name', '') AS name, role
            FROM human_accounts
            WHERE active
            ORDER BY login
            "#,
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| DirectoryPerson {
            id: row.id,
            login: row.login,
            name: row.name,
            role: Role::parse(&row.role),
        })
        .collect();
        let groups = sqlx::query_as!(
            DirectoryGroup,
            "SELECT id, name FROM human_teams WHERE active ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(Directory { people, groups })
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id))]
    pub async fn change_role(
        &self,
        actor: &Actor,
        input: &ChangeRole,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(actor, "account.role", input.request_id, input)?;
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        let subject = subject(&mut transaction, input.id).await?;
        let promoting = input.role == Role::Administrator;
        guard_subject(&mut transaction, actor, &subject, promoting).await?;
        // A role that comes from a group changes with the group.
        let grouped = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM human_team_members m JOIN human_teams t ON t.id = m.team_id AND t.active AND t.role IS NOT NULL
                WHERE m.account_id = $1
            ) AS "grouped!"
            "#,
            input.id,
        )
        .fetch_one(&mut *transaction)
        .await?;
        if grouped {
            return Err(IdentityError::Conflict);
        }
        let revision = sqlx::query_scalar!(
            r#"
            UPDATE human_accounts SET role = $3, role_from_group = false, revision = revision + 1
            WHERE id = $1 AND revision = $2
            RETURNING revision AS "revision!"
            "#,
            input.id,
            input.expected_revision,
            input.role.as_str(),
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IdentityError::Conflict)?;
        command
            .commit(
                transaction,
                Receipt {
                    id: input.id,
                    revision,
                },
            )
            .await
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id))]
    pub async fn reactivate(
        &self,
        actor: &Actor,
        input: &Reactivate,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(actor, "account.reactivate", input.request_id, input)?;
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        let subject = subject(&mut transaction, input.id).await?;
        guard_subject(&mut transaction, actor, &subject, false).await?;
        let revision = sqlx::query_scalar!(
            r#"
            UPDATE human_accounts SET active = true, auth_attempts = 0, revision = revision + 1
            WHERE id = $1 AND revision = $2 AND NOT active
            RETURNING revision AS "revision!"
            "#,
            input.id,
            input.expected_revision,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IdentityError::Conflict)?;
        command
            .commit(
                transaction,
                Receipt {
                    id: input.id,
                    revision,
                },
            )
            .await
    }

    /// Hands one agent, room or team to someone else: its owner or an administrator may.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id))]
    pub async fn transfer(
        &self,
        actor: &Actor,
        input: &Transfer,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(actor, "ownership.transfer", input.request_id, input)?;
        let (mut transaction, previous) = command.begin_account(&self.pool).await?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        may_own(&mut transaction, input.to).await?;
        let administrator = actor.role == Role::Administrator;
        let moved = match input.kind {
            Owned::Agent => sqlx::query_scalar!(
                "UPDATE agents SET created_by = $3 WHERE id = $1 AND desired_state <> 'retired' AND (created_by = $2 OR $4) RETURNING id",
                input.id, actor.id, input.to, administrator,
            )
            .fetch_optional(&mut *transaction)
            .await?,
            Owned::Room => sqlx::query_scalar!(
                "UPDATE rooms SET created_by = $3 WHERE id = $1 AND NOT task AND (created_by = $2 OR $4) RETURNING id",
                input.id, actor.id, input.to, administrator,
            )
            .fetch_optional(&mut *transaction)
            .await?,
            Owned::Team => {
                let room = sqlx::query_scalar!(
                    "UPDATE teams SET created_by = $3 WHERE id = $1 AND deleted_at IS NULL AND (created_by = $2 OR $4) RETURNING room_id",
                    input.id, actor.id, input.to, administrator,
                )
                .fetch_optional(&mut *transaction)
                .await?;
                if let Some(room) = room {
                    sqlx::query!("UPDATE rooms SET created_by = $2 WHERE id = $1", room, input.to)
                        .execute(&mut *transaction)
                        .await?;
                }
                room.map(|_| input.id)
            }
        };
        let id = moved.ok_or(IdentityError::NotFound)?;
        command
            .commit(transaction, Receipt { id, revision: 1 })
            .await
    }

    /// Moves everything one person owns to another, such as when someone leaves.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id))]
    pub async fn transfer_all(
        &self,
        actor: &Actor,
        input: &TransferAll,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(actor, "ownership.transfer_all", input.request_id, input)?;
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        if input.from == input.to {
            return Err(IdentityError::InvalidInput);
        }
        may_own(&mut transaction, input.to).await?;
        sqlx::query!(
            "UPDATE agents SET created_by = $2 WHERE created_by = $1 AND desired_state <> 'retired'",
            input.from,
            input.to,
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query!(
            "UPDATE teams SET created_by = $2 WHERE created_by = $1 AND deleted_at IS NULL",
            input.from,
            input.to,
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query!(
            "UPDATE rooms SET created_by = $2 WHERE created_by = $1 AND NOT task",
            input.from,
            input.to,
        )
        .execute(&mut *transaction)
        .await?;
        command
            .commit(
                transaction,
                Receipt {
                    id: input.from,
                    revision: 1,
                },
            )
            .await
    }

    /// The installation owner names another administrator as owner.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id))]
    pub async fn hand_over(
        &self,
        actor: &Actor,
        input: &HandOver,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(actor, "installation.hand_over", input.request_id, input)?;
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        let id = sqlx::query_scalar!(
            r#"
            UPDATE installation SET owner_id = $2
            WHERE singleton AND owner_id = $1
              AND EXISTS(SELECT 1 FROM human_accounts WHERE id = $2 AND active AND role = 'administrator')
            RETURNING id
            "#,
            actor.id,
            input.to,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IdentityError::Forbidden)?;
        command
            .commit(transaction, Receipt { id, revision: 1 })
            .await
    }
}

pub(super) struct Subject {
    pub id: Uuid,
    pub administrator: bool,
    pub owner: bool,
}

pub(super) async fn subject(
    transaction: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<Subject, IdentityError> {
    let row = sqlx::query!(
        r#"
        SELECT a.id, a.role = 'administrator' AS "administrator!",
               i.owner_id IS NOT DISTINCT FROM a.id AS "owner!"
        FROM human_accounts a CROSS JOIN installation i
        WHERE a.id = $1
        FOR UPDATE OF a
        "#,
        id,
    )
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(IdentityError::NotFound)?;
    Ok(Subject {
        id: row.id,
        administrator: row.administrator,
        owner: row.owner,
    })
}

pub(super) async fn is_owner(
    transaction: &mut Transaction<'_, Postgres>,
    actor: Uuid,
) -> Result<bool, IdentityError> {
    Ok(sqlx::query_scalar!(
        r#"SELECT owner_id IS NOT DISTINCT FROM $1 AS "owner!" FROM installation WHERE singleton"#,
        actor,
    )
    .fetch_one(&mut **transaction)
    .await?)
}

/// Nobody changes their own standing or the owner's, and only the installation owner changes
/// an administrator's standing or makes someone an administrator.
pub(super) async fn guard_subject(
    transaction: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    subject: &Subject,
    promoting: bool,
) -> Result<(), IdentityError> {
    if subject.id == actor.id || subject.owner {
        return Err(IdentityError::InvalidInput);
    }
    if (subject.administrator || promoting) && !is_owner(transaction, actor.id).await? {
        return Err(IdentityError::Forbidden);
    }
    Ok(())
}

async fn may_own(
    transaction: &mut Transaction<'_, Postgres>,
    to: Uuid,
) -> Result<(), IdentityError> {
    sqlx::query_scalar!(
        "SELECT id FROM human_accounts WHERE id = $1 AND active AND role IN ('administrator', 'member') FOR SHARE",
        to,
    )
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(IdentityError::InvalidInput)?;
    Ok(())
}

async fn installation_id(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<Uuid, IdentityError> {
    Ok(
        sqlx::query_scalar!("SELECT id FROM installation WHERE singleton")
            .fetch_one(&mut **transaction)
            .await?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(role: &str, owned: i64, limit: Option<i32>) -> Row {
        Row {
            role: role.into(),
            owner: false,
            two_factor: false,
            owned,
            solo: false,
            policy: Policy {
                members_create_agents: true,
                members_create_rooms: false,
                members_create_teams: true,
                members_create_connections: true,
                members_bring_providers: true,
                member_agent_limit: limit,
            },
        }
    }

    #[test]
    fn members_create_within_policy_and_limit() {
        assert!(row("member", 2, Some(3)).allows(Creation::Agent));
        assert!(!row("member", 3, Some(3)).allows(Creation::Agent));
        assert!(row("member", 300, None).allows(Creation::Agent));
        assert!(!row("member", 0, None).allows(Creation::Room));
        assert!(row("administrator", 300, Some(0)).allows(Creation::Room));
        assert!(!row("guest", 0, None).allows(Creation::Team));
        assert!(!row("auditor", 0, None).allows(Creation::Agent));
        assert!(!row("someone", 0, None).allows(Creation::Agent));
    }
}

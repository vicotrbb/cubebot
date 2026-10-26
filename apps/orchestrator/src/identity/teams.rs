use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Actor, Identity, IdentityError, Receipt, commands::Command};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, sqlx::Type)]
#[serde(transparent)]
#[sqlx(transparent)]
pub struct TeamId(pub Uuid);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTeam {
    pub request_id: Uuid,
    pub name: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MembershipChange {
    pub request_id: Uuid,
    pub team: TeamId,
    pub account_id: Uuid,
    pub expected_revision: i64,
    pub present: bool,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Team {
    pub id: TeamId,
    pub name: String,
    pub revision: i64,
    pub active: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteTeam {
    pub request_id: Uuid,
    pub team: TeamId,
    pub expected_revision: i64,
}

impl Identity {
    pub async fn create_team(
        &self,
        actor: &Actor,
        input: &CreateTeam,
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
        let id = sqlx::query_scalar("INSERT INTO human_teams (name) VALUES ($1) RETURNING id")
            .bind(name)
            .fetch_one(&mut *transaction)
            .await?;
        command
            .commit(transaction, Receipt { id, revision: 1 })
            .await
    }

    pub async fn change_membership(
        &self,
        actor: &Actor,
        input: &MembershipChange,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(actor, "team.membership", input.request_id, input)?;
        let (mut transaction, receipt) = command.begin(&self.pool).await?;
        if let Some(receipt) = receipt {
            return Ok(receipt);
        }
        let revision = revise_team(&mut transaction, input.team, input.expected_revision).await?;
        let exists: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM human_accounts WHERE id = $1 AND (active OR NOT $2) FOR SHARE",
        )
        .bind(input.account_id)
        .bind(input.present)
        .fetch_optional(&mut *transaction)
        .await?;
        exists.ok_or(IdentityError::InvalidInput)?;
        if input.present {
            sqlx::query("INSERT INTO human_team_members (team_id, account_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(input.team).bind(input.account_id).execute(&mut *transaction).await?;
        } else {
            sqlx::query("DELETE FROM human_team_members WHERE team_id = $1 AND account_id = $2")
                .bind(input.team)
                .bind(input.account_id)
                .execute(&mut *transaction)
                .await?;
        }
        command
            .commit(
                transaction,
                Receipt {
                    id: input.team.0,
                    revision,
                },
            )
            .await
    }

    pub async fn delete_team(
        &self,
        actor: &Actor,
        input: &DeleteTeam,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(actor, "team.delete", input.request_id, input)?;
        let (mut transaction, receipt) = command.begin(&self.pool).await?;
        if let Some(receipt) = receipt {
            return Ok(receipt);
        }
        let revision: Option<i64> = sqlx::query_scalar("UPDATE human_teams SET active = false, revision = revision + 1 WHERE id = $1 AND active AND revision = $2 RETURNING revision")
            .bind(input.team).bind(input.expected_revision).fetch_optional(&mut *transaction).await?;
        let revision = revision.ok_or(IdentityError::Conflict)?;
        command
            .commit(
                transaction,
                Receipt {
                    id: input.team.0,
                    revision,
                },
            )
            .await
    }

    pub async fn teams(
        &self,
        actor: &Actor,
        after: Option<TeamId>,
    ) -> Result<Vec<Team>, IdentityError> {
        self.require_administrator(actor).await?;
        Ok(sqlx::query_as(
            "SELECT id, name, revision, active FROM human_teams WHERE $1::uuid IS NULL OR id > $1 ORDER BY id LIMIT 100",
        )
        .bind(after)
        .fetch_all(&self.pool)
        .await?)
    }
}

async fn revise_team(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: TeamId,
    expected: i64,
) -> Result<i64, IdentityError> {
    let revision: Option<i64> = sqlx::query_scalar("UPDATE human_teams SET revision = revision + 1 WHERE id = $1 AND active AND revision = $2 RETURNING revision")
        .bind(id).bind(expected).fetch_optional(&mut **transaction).await?;
    revision.ok_or(IdentityError::Conflict)
}

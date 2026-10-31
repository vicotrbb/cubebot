use serde::Serialize;
use uuid::Uuid;

use super::{
    Actor, Identity, IdentityError, Receipt, accounts, commands::Command, passwords,
    sessions::digest,
};

#[derive(Serialize)]
pub struct Invitation {
    pub id: Uuid,
    /// Revealed only on issuance. A retried command returns its durable ID.
    pub token: Option<String>,
    pub revision: i64,
}

#[derive(Serialize)]
pub struct InvitationSummary {
    pub id: Uuid,
    pub login: String,
    pub revision: i64,
    pub status: String,
    pub expires_at_unix: i64,
}

struct Redemption {
    id: Uuid,
    login: String,
    accepted_by: Option<Uuid>,
    accept_request_id: Option<Uuid>,
}

impl Redemption {
    fn previous(&self, request_id: Uuid) -> Result<Option<Uuid>, IdentityError> {
        if self.accepted_by.is_some() && self.accept_request_id != Some(request_id) {
            return Err(IdentityError::Unauthorized);
        }
        Ok(self.accepted_by)
    }
}

impl Identity {
    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %request_id))]
    pub async fn invite(
        &self,
        actor: &Actor,
        login: &str,
        request_id: Uuid,
    ) -> Result<Invitation, IdentityError> {
        let command = Command::new(
            actor,
            "invitation.create",
            request_id,
            &serde_json::json!({"login": login}),
        )?;
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            return Ok(Invitation {
                id: receipt.id,
                token: None,
                revision: receipt.revision,
            });
        }
        passwords::validate_login(login)?;
        let exists = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1
                FROM human_accounts
                WHERE login = $1
            ) AS "exists!"
            "#,
            login,
        )
        .fetch_one(&mut *transaction)
        .await?;
        if exists {
            return Err(IdentityError::Conflict);
        }
        let token = passwords::token();
        let id = sqlx::query_scalar!(
            r#"
            INSERT INTO invitations (login, token_hash, created_by)
            VALUES ($1, $2, $3)
            RETURNING id AS "id!"
            "#,
            login,
            digest(&token),
            actor.id,
        )
        .fetch_one(&mut *transaction)
        .await?;
        command
            .commit(transaction, Receipt { id, revision: 1 })
            .await?;
        Ok(Invitation {
            id,
            token: Some(token),
            revision: 1,
        })
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, invitation = %id, request = %request_id))]
    pub async fn revoke_invitation(
        &self,
        actor: &Actor,
        id: Uuid,
        expected_revision: i64,
        request_id: Uuid,
    ) -> Result<Receipt, IdentityError> {
        let command = Command::new(
            actor,
            "invitation.revoke",
            request_id,
            &serde_json::json!({"id": id, "expected_revision": expected_revision}),
        )?;
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        let revision = sqlx::query_scalar!(
            r#"
            UPDATE invitations
            SET revoked_by = $1, revision = revision + 1
            WHERE id = $2
                AND revision = $3
                AND accepted_by IS NULL
                AND revoked_by IS NULL
            RETURNING revision AS "revision!"
            "#,
            actor.id,
            id,
            expected_revision,
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let revision = revision.ok_or(IdentityError::Conflict)?;
        command.commit(transaction, Receipt { id, revision }).await
    }

    #[tracing::instrument(skip_all, fields(request = %request_id))]
    pub async fn accept_invitation(
        &self,
        token: &str,
        password: &str,
        request_id: Uuid,
    ) -> Result<Uuid, IdentityError> {
        if request_id.is_nil() {
            return Err(IdentityError::InvalidInput);
        }
        let invitation = load(&self.pool, token).await?;
        if let Some(account) = invitation.previous(request_id)? {
            self.verify_command_password(account, "invitation.accept", request_id, password)
                .await?;
            return Ok(account);
        }
        passwords::validate(&invitation.login, password)?;
        let verifier = passwords::hash(password).await?;
        let mut transaction = self.pool.begin().await?;
        // Single line: tests find this waiting statement by its text in pg_stat_activity.
        sqlx::query!(
            "SELECT id FROM invitations WHERE token_hash = $1 FOR UPDATE",
            digest(token),
        )
        .fetch_one(&mut *transaction)
        .await?;
        // Fresh clock evaluation after the row lock, including after a competing redemption.
        let invitation = load(&mut *transaction, token).await?;
        if let Some(account) = invitation.previous(request_id)? {
            transaction.rollback().await?;
            self.verify_command_password(account, "invitation.accept", request_id, password)
                .await?;
            return Ok(account);
        }
        admit(transaction, invitation, verifier, request_id).await
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn invitations(
        &self,
        actor: &Actor,
        after: Option<Uuid>,
    ) -> Result<Vec<InvitationSummary>, IdentityError> {
        self.require_administrator(actor).await?;
        Ok(sqlx::query_as!(
            InvitationSummary,
            r#"
            SELECT
                id AS "id!",
                login AS "login!",
                revision AS "revision!",
                EXTRACT(EPOCH FROM expires_at)::bigint AS "expires_at_unix!",
                CASE
                    WHEN revoked_by IS NOT NULL THEN 'revoked'
                    WHEN accepted_by IS NOT NULL THEN 'accepted'
                    WHEN expires_at <= clock_timestamp() THEN 'expired'
                    ELSE 'pending'
                END AS "status!"
            FROM invitations
            WHERE $1::uuid IS NULL OR id > $1
            ORDER BY id
            LIMIT 100
            "#,
            after,
        )
        .fetch_all(&self.pool)
        .await?)
    }
}

async fn load<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Postgres>,
    token: &str,
) -> Result<Redemption, IdentityError> {
    if token.len() != 64 {
        return Err(IdentityError::Unauthorized);
    }
    sqlx::query_as!(
        Redemption,
        r#"
        SELECT
            id AS "id!",
            login AS "login!",
            accepted_by AS "accepted_by?",
            accept_request_id AS "accept_request_id?"
        FROM invitations
        WHERE token_hash = $1
            AND expires_at > clock_timestamp()
            AND revoked_by IS NULL
        "#,
        digest(token),
    )
    .fetch_optional(executor)
    .await?
    .ok_or(IdentityError::Unauthorized)
}

async fn admit(
    mut transaction: sqlx::Transaction<'_, sqlx::Postgres>,
    invitation: Redemption,
    verifier: String,
    request_id: Uuid,
) -> Result<Uuid, IdentityError> {
    let account =
        accounts::insert_account(&mut transaction, &invitation.login, &verifier, "member").await?;
    sqlx::query!(
        "
        UPDATE invitations
        SET
            accepted_by = $1,
            accepted_at = clock_timestamp(),
            accept_request_id = $2,
            revision = revision + 1
        WHERE id = $3
        ",
        account,
        request_id,
        invitation.id,
    )
    .execute(&mut *transaction)
    .await?;
    accounts::audit(&mut transaction, account, account, "invitation_accepted").await?;
    let command = Command::new(
        &Actor {
            id: account,
            administrator: false,
        },
        "invitation.accept",
        request_id,
        &serde_json::json!({"invitation_id": invitation.id}),
    )?
    .with_password_verifier(verifier);
    command
        .commit(
            transaction,
            Receipt {
                id: account,
                revision: 1,
            },
        )
        .await?;
    Ok(account)
}

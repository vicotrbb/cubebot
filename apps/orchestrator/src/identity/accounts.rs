use uuid::Uuid;

use super::{Actor, Identity, IdentityError, Receipt, commands::Command, passwords};

#[derive(serde::Serialize)]
pub struct AccountSummary {
    pub id: Uuid,
    pub login: String,
    pub administrator: bool,
    pub active: bool,
    pub revision: i64,
}

impl Identity {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn accounts(
        &self,
        actor: &Actor,
        after: Option<Uuid>,
    ) -> Result<Vec<AccountSummary>, IdentityError> {
        self.require_administrator(actor).await?;
        Ok(sqlx::query_as!(
            AccountSummary,
            r#"
            SELECT
                id AS "id!",
                login AS "login!",
                role = 'administrator' AS "administrator!",
                active AS "active!",
                revision AS "revision!"
            FROM human_accounts
            WHERE $1::uuid IS NULL OR id > $1
            ORDER BY id
            LIMIT 100
            "#,
            after,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    #[tracing::instrument(skip_all)]
    pub async fn bootstrap(&self, login: &str, password: &str) -> Result<Uuid, IdentityError> {
        passwords::validate(login, password)?;
        let verifier = passwords::hash(password).await?;
        let mut transaction = self.pool.begin().await?;
        let bootstrapped = sqlx::query_scalar!(
            r#"
            SELECT bootstrapped AS "bootstrapped!"
            FROM installation
            WHERE singleton
            FOR UPDATE
            "#,
        )
        .fetch_one(&mut *transaction)
        .await?;
        if bootstrapped {
            return Err(IdentityError::Conflict);
        }
        let id = insert_account(&mut transaction, login, &verifier, "administrator").await?;
        sqlx::query!(
            "
            UPDATE installation
            SET bootstrapped = true
            WHERE singleton
            ",
        )
        .execute(&mut *transaction)
        .await?;
        audit(&mut transaction, id, id, "bootstrap").await?;
        transaction.commit().await?;
        Ok(id)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %request_id))]
    pub async fn create_account(
        &self,
        actor: &Actor,
        login: &str,
        password: &str,
        administrator: bool,
        request_id: Uuid,
    ) -> Result<Uuid, IdentityError> {
        let input = serde_json::json!({"login": login, "administrator": administrator});
        let command = Command::new(actor, "account.create", request_id, &input)?;
        let (transaction, previous) = command.begin(&self.pool).await?;
        transaction.rollback().await?;
        if let Some(receipt) = previous {
            return self
                .replay_account(actor, request_id, password, receipt)
                .await;
        }
        passwords::validate(login, password)?;
        let verifier = passwords::hash(password).await?;
        let command = command.with_password_verifier(verifier.clone());
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            transaction.rollback().await?;
            return self
                .replay_account(actor, request_id, password, receipt)
                .await;
        }
        let role = if administrator {
            "administrator"
        } else {
            "member"
        };
        let id = insert_account(&mut transaction, login, &verifier, role).await?;
        audit(&mut transaction, actor.id, id, "account_created").await?;
        command
            .commit(transaction, Receipt { id, revision: 1 })
            .await?;
        Ok(id)
    }

    async fn replay_account(
        &self,
        actor: &Actor,
        request_id: Uuid,
        password: &str,
        receipt: Receipt,
    ) -> Result<Uuid, IdentityError> {
        self.verify_command_password(actor.id, "account.create", request_id, password)
            .await?;
        self.require_administrator(actor).await?;
        Ok(receipt.id)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, subject = %subject, request = %request_id))]
    pub async fn suspend(
        &self,
        actor: &Actor,
        subject: Uuid,
        expected_revision: i64,
        request_id: Uuid,
    ) -> Result<Receipt, IdentityError> {
        let input = serde_json::json!({"id": subject, "expected_revision": expected_revision});
        let command = Command::new(actor, "account.suspend", request_id, &input)?;
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        if actor.id == subject {
            return Err(IdentityError::InvalidInput);
        }
        let revision = sqlx::query_scalar!(
            r#"
            UPDATE human_accounts
            SET active = false, revision = revision + 1
            WHERE id = $1 AND revision = $2 AND active
            RETURNING revision AS "revision!"
            "#,
            subject,
            expected_revision,
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let revision = revision.ok_or(IdentityError::Conflict)?;
        sqlx::query!(
            "
            DELETE FROM human_sessions
            WHERE account_id = $1
            ",
            subject,
        )
        .execute(&mut *transaction)
        .await?;
        audit(&mut transaction, actor.id, subject, "account_suspended").await?;
        command
            .commit(
                transaction,
                Receipt {
                    id: subject,
                    revision,
                },
            )
            .await
    }
}

pub(super) async fn lock_administrator(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Uuid,
) -> Result<(), IdentityError> {
    let allowed = sqlx::query_scalar!(
        r#"
        SELECT id AS "id!"
        FROM human_accounts
        WHERE id = $1 AND active AND role = 'administrator'
        FOR UPDATE
        "#,
        actor,
    )
    .fetch_optional(&mut **transaction)
    .await?;
    allowed.ok_or(IdentityError::Forbidden)?;
    Ok(())
}

pub(super) async fn insert_account(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    login: &str,
    verifier: &str,
    role: &str,
) -> Result<Uuid, IdentityError> {
    Ok(sqlx::query_scalar!(
        r#"
        INSERT INTO human_accounts (login, password_hash, role)
        VALUES ($1, $2, $3)
        RETURNING id AS "id!"
        "#,
        login,
        verifier,
        role,
    )
    .fetch_one(&mut **transaction)
    .await?)
}

pub(super) async fn audit(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Uuid,
    subject: Uuid,
    action: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "
        INSERT INTO identity_audit (actor_id, subject_id, action)
        VALUES ($1, $2, $3)
        ",
        actor,
        subject,
        action,
    )
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

use uuid::Uuid;

use super::{Actor, Identity, IdentityError, passwords};

impl Identity {
    pub async fn bootstrap(&self, login: &str, password: &str) -> Result<Uuid, IdentityError> {
        passwords::validate(login, password)?;
        let verifier = passwords::hash(password).await?;
        let mut transaction = self.pool.begin().await?;
        let bootstrapped: bool =
            sqlx::query_scalar("SELECT bootstrapped FROM installation WHERE singleton FOR UPDATE")
                .fetch_one(&mut *transaction)
                .await?;
        if bootstrapped {
            return Err(IdentityError::Conflict);
        }
        let id = insert_account(&mut transaction, login, &verifier, "administrator").await?;
        sqlx::query("UPDATE installation SET bootstrapped = true WHERE singleton")
            .execute(&mut *transaction)
            .await?;
        audit(&mut transaction, id, id, "bootstrap").await?;
        transaction.commit().await?;
        Ok(id)
    }

    pub async fn create_account(
        &self,
        actor: &Actor,
        login: &str,
        password: &str,
        administrator: bool,
    ) -> Result<Uuid, IdentityError> {
        self.require_administrator(actor).await?;
        passwords::validate(login, password)?;
        let verifier = passwords::hash(password).await?;
        let role = if administrator {
            "administrator"
        } else {
            "member"
        };
        let mut transaction = self.pool.begin().await?;
        lock_administrator(&mut transaction, actor.id).await?;
        let id = insert_account(&mut transaction, login, &verifier, role).await?;
        audit(&mut transaction, actor.id, id, "account_created").await?;
        transaction.commit().await?;
        Ok(id)
    }

    pub async fn suspend(&self, actor: &Actor, subject: Uuid) -> Result<(), IdentityError> {
        if actor.id == subject {
            return Err(IdentityError::InvalidInput);
        }
        let mut transaction = self.pool.begin().await?;
        lock_administrator(&mut transaction, actor.id).await?;
        let changed = sqlx::query(
            "UPDATE human_accounts SET active = false, revision = revision + 1 WHERE id = $1",
        )
        .bind(subject)
        .execute(&mut *transaction)
        .await?;
        if changed.rows_affected() == 0 {
            return Err(IdentityError::InvalidInput);
        }
        sqlx::query("DELETE FROM human_sessions WHERE account_id = $1")
            .bind(subject)
            .execute(&mut *transaction)
            .await?;
        audit(&mut transaction, actor.id, subject, "account_suspended").await?;
        transaction.commit().await?;
        Ok(())
    }
}

pub(super) async fn lock_administrator(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Uuid,
) -> Result<(), IdentityError> {
    let allowed: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM human_accounts WHERE id = $1 AND active AND role = 'administrator' FOR UPDATE",
    )
    .bind(actor)
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
    Ok(sqlx::query_scalar(
        "INSERT INTO human_accounts (login, password_hash, role) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(login)
    .bind(verifier)
    .bind(role)
    .fetch_one(&mut **transaction)
    .await?)
}

pub(super) async fn audit(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Uuid,
    subject: Uuid,
    action: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO identity_audit (actor_id, subject_id, action) VALUES ($1, $2, $3)")
        .bind(actor)
        .bind(subject)
        .bind(action)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

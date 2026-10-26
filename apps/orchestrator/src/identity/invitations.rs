use uuid::Uuid;

use super::{Actor, Identity, IdentityError, accounts, passwords, sessions::digest};

pub struct Invitation {
    pub id: Uuid,
    pub token: String,
}

impl Identity {
    pub async fn invite(&self, actor: &Actor, login: &str) -> Result<Invitation, IdentityError> {
        passwords::validate_login(login)?;
        let token = passwords::token();
        let mut transaction = self.pool.begin().await?;
        accounts::lock_administrator(&mut transaction, actor.id).await?;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM human_accounts WHERE login = $1)")
                .bind(login)
                .fetch_one(&mut *transaction)
                .await?;
        if exists {
            return Err(IdentityError::Conflict);
        }
        let id = sqlx::query_scalar(
            "INSERT INTO invitations (login, token_hash, created_by) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(login)
        .bind(digest(&token))
        .bind(actor.id)
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(Invitation { id, token })
    }

    pub async fn revoke_invitation(&self, actor: &Actor, id: Uuid) -> Result<(), IdentityError> {
        let mut transaction = self.pool.begin().await?;
        accounts::lock_administrator(&mut transaction, actor.id).await?;
        let result = sqlx::query(
            "UPDATE invitations SET revoked_by = COALESCE(revoked_by, $1) WHERE id = $2 AND accepted_by IS NULL",
        )
        .bind(actor.id)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            return Err(IdentityError::Conflict);
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn accept_invitation(
        &self,
        token: &str,
        password: &str,
    ) -> Result<Uuid, IdentityError> {
        let login: String = sqlx::query_scalar(
            "SELECT login FROM invitations WHERE token_hash = $1 AND expires_at > now() AND revoked_by IS NULL AND accepted_by IS NULL",
        )
        .bind(digest(token))
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::Unauthorized)?;
        passwords::validate(&login, password)?;
        let verifier = passwords::hash(password).await?;
        let mut transaction = self.pool.begin().await?;
        let invitation: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM invitations WHERE token_hash = $1 AND expires_at > now() AND revoked_by IS NULL AND accepted_by IS NULL FOR UPDATE",
        )
        .bind(digest(token))
        .fetch_optional(&mut *transaction)
        .await?;
        let invitation = invitation.ok_or(IdentityError::Unauthorized)?;
        // The locking predicate may have been evaluated before waiting on another writer.
        let unexpired: bool = sqlx::query_scalar(
            "SELECT expires_at > clock_timestamp() FROM invitations WHERE id = $1",
        )
        .bind(invitation)
        .fetch_one(&mut *transaction)
        .await?;
        if !unexpired {
            return Err(IdentityError::Unauthorized);
        }
        let id = accounts::insert_account(&mut transaction, &login, &verifier, "member").await?;
        sqlx::query("UPDATE invitations SET accepted_by = $1, accepted_at = now() WHERE id = $2")
            .bind(id)
            .bind(invitation)
            .execute(&mut *transaction)
            .await?;
        accounts::audit(&mut transaction, id, id, "invitation_accepted").await?;
        transaction.commit().await?;
        Ok(id)
    }
}

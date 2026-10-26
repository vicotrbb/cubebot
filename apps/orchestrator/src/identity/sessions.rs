use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{Actor, Identity, IdentityError, Session, passwords};

impl Identity {
    pub async fn login(&self, login: &str, password: &str) -> Result<Session, IdentityError> {
        if login.len() > 64 || password.len() > 1024 {
            return Err(IdentityError::Unauthorized);
        }
        let account: Option<(Uuid, String)> = sqlx::query_as(
            "SELECT id, password_hash FROM human_accounts WHERE login = $1 AND active AND password_hash IS NOT NULL",
        )
        .bind(login)
        .fetch_optional(&self.pool)
        .await?;
        let (id, verifier) = account.ok_or(IdentityError::Unauthorized)?;
        passwords::verify(password, verifier).await?;
        let token = passwords::token();
        // Recheck activity under a row lock after the expensive password verification.
        let mut transaction = self.pool.begin().await?;
        let active: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM human_accounts WHERE id = $1 AND active FOR UPDATE")
                .bind(id)
                .fetch_optional(&mut *transaction)
                .await?;
        active.ok_or(IdentityError::Unauthorized)?;
        sqlx::query("INSERT INTO human_sessions (token_hash, account_id, expires_at) VALUES ($1, $2, now() + interval '1 day')")
            .bind(digest(&token)).bind(id).execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(Session { token })
    }

    pub async fn authenticate(&self, token: &str) -> Result<Actor, IdentityError> {
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(IdentityError::Unauthorized);
        }
        let account: Option<(Uuid, bool)> = sqlx::query_as(
            "SELECT a.id, a.role = 'administrator' FROM human_sessions s JOIN human_accounts a ON a.id = s.account_id WHERE s.token_hash = $1 AND s.expires_at > now() AND a.active",
        )
        .bind(digest(token))
        .fetch_optional(&self.pool)
        .await?;
        let (id, administrator) = account.ok_or(IdentityError::Unauthorized)?;
        Ok(Actor { id, administrator })
    }

    pub async fn logout(&self, token: &str) -> Result<(), IdentityError> {
        sqlx::query("DELETE FROM human_sessions WHERE token_hash = $1")
            .bind(digest(token))
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

pub(super) fn digest(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

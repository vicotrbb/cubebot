use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{Actor, Identity, IdentityError, Session, passwords};

impl Identity {
    #[tracing::instrument(skip_all)]
    pub async fn login(&self, login: &str, password: &str) -> Result<Session, IdentityError> {
        self.login_with(login, password, None).await
    }

    /// Signs in with a password and, when the account has two-factor on, its code.
    #[tracing::instrument(skip_all)]
    pub async fn login_with(
        &self,
        login: &str,
        password: &str,
        code: Option<&str>,
    ) -> Result<Session, IdentityError> {
        if login.len() > 64 || password.len() > 1024 {
            return Err(IdentityError::Unauthorized);
        }
        let account = sqlx::query!(
            r#"
            SELECT id AS "id!", password_hash AS "password_hash!"
            FROM human_accounts
            WHERE login = $1 AND active AND password_hash IS NOT NULL
            "#,
            login,
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(account) = account else {
            // The same Argon2 work as a real login, so timing does not reveal which logins exist.
            passwords::verify(password, passwords::DUMMY.into()).await?;
            return Err(IdentityError::Unauthorized);
        };
        let id = account.id;
        self.reserve_password_check(id).await?;
        passwords::verify(password, account.password_hash).await?;
        let token = passwords::token();
        // Recheck activity under a row lock after the expensive password verification.
        let mut transaction = self.pool.begin().await?;
        let active = sqlx::query_scalar!(
            r#"
            SELECT id AS "id!"
            FROM human_accounts
            WHERE id = $1 AND active
            FOR UPDATE
            "#,
            id,
        )
        .fetch_optional(&mut *transaction)
        .await?;
        active.ok_or(IdentityError::Unauthorized)?;
        if let Err(error) = self.second_factor(&mut transaction, id, code).await {
            // Commit the used-code bookkeeping only on success; a wrong code changes nothing.
            transaction.rollback().await?;
            return Err(error);
        }
        sqlx::query!(
            "
            INSERT INTO human_sessions (token_hash, account_id, expires_at)
            VALUES ($1, $2, clock_timestamp() + interval '1 day')
            ",
            digest(&token),
            id,
        )
        .execute(&mut *transaction)
        .await?;
        let event = super::audit::Event::by(id, "session.sign_in")
            .on(id)
            .detail(serde_json::json!({"method": "password"}));
        super::audit::record(&mut *transaction, &event).await?;
        transaction.commit().await?;
        Ok(Session { token })
    }

    #[tracing::instrument(skip_all)]
    pub async fn authenticate(&self, token: &str) -> Result<Actor, IdentityError> {
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(IdentityError::Unauthorized);
        }
        let account = sqlx::query!(
            r#"
            SELECT a.id AS "id!", a.role AS "role!",
                s.last_seen_at IS NULL OR s.last_seen_at < clock_timestamp() - interval '5 minutes' AS "stale!"
            FROM human_sessions s
            JOIN human_accounts a ON a.id = s.account_id
            WHERE s.token_hash = $1
                AND s.expires_at > clock_timestamp()
                AND a.active
                AND (
                    s.oidc_policy_revision IS NULL
                    OR EXISTS(
                        SELECT 1
                        FROM identity_oidc_policy p
                        WHERE p.singleton AND p.enabled AND p.revision = s.oidc_policy_revision
                    )
                )
            "#,
            digest(token),
        )
        .fetch_optional(&self.pool)
        .await?;
        let account = account.ok_or(IdentityError::Unauthorized)?;
        if account.stale {
            // At most one write per session every five minutes, for "last seen".
            sqlx::query!(
                "
                WITH seen AS (
                    UPDATE human_sessions SET last_seen_at = clock_timestamp() WHERE token_hash = $1
                )
                UPDATE human_accounts SET last_seen_at = clock_timestamp() WHERE id = $2
                ",
                digest(token),
                account.id,
            )
            .execute(&self.pool)
            .await?;
        }
        let role = super::Role::parse(&account.role);
        Ok(Actor {
            id: account.id,
            administrator: role == super::Role::Administrator,
            role,
        })
    }

    #[tracing::instrument(skip_all)]
    pub async fn logout(&self, token: &str) -> Result<(), IdentityError> {
        let ended = sqlx::query_scalar!(
            "
            DELETE FROM human_sessions
            WHERE token_hash = $1
            RETURNING account_id
            ",
            digest(token),
        )
        .fetch_optional(&self.pool)
        .await?;
        if let Some(account) = ended {
            self.audit(super::audit::Event::by(account, "session.sign_out").on(account))
                .await;
        }
        Ok(())
    }
}

pub(crate) fn digest(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

impl Identity {
    pub(super) async fn reserve_password_check(&self, account: Uuid) -> Result<(), IdentityError> {
        let reserved = sqlx::query_scalar!(
            r#"
            UPDATE human_accounts
            SET
                auth_window_started_at = CASE
                    WHEN auth_window_started_at <= clock_timestamp() - interval '60 seconds'
                    THEN clock_timestamp()
                    ELSE auth_window_started_at
                END,
                auth_attempts = CASE
                    WHEN auth_window_started_at <= clock_timestamp() - interval '60 seconds'
                    THEN 1
                    ELSE auth_attempts + 1
                END
            WHERE id = $1
                AND active
                AND (
                    auth_attempts < 10
                    OR auth_window_started_at <= clock_timestamp() - interval '60 seconds'
                )
            RETURNING id AS "id!"
            "#,
            account,
        )
        .fetch_optional(&self.pool)
        .await?;
        reserved.ok_or(IdentityError::Unauthorized)?;
        Ok(())
    }
}

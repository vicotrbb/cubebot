use super::{Config, provider::Verified};
use crate::identity::{IdentityError, Session, accounts, passwords, sessions::digest};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[tracing::instrument(skip_all)]
pub(super) async fn configure(pool: &PgPool, config: &Config) -> Result<i64, IdentityError> {
    let bytes = serde_json::to_vec(config).map_err(|error| IdentityError::Invalid(error.into()))?;
    let hash = Sha256::digest(bytes).to_vec();
    Ok(sqlx::query_scalar!(
        r#"
        UPDATE identity_oidc_policy
        SET
            revision = CASE WHEN enabled AND config_hash = $1 THEN revision ELSE revision + 1 END,
            enabled = true,
            config_hash = $1
        WHERE singleton
        RETURNING revision AS "revision!"
        "#,
        hash,
    )
    .fetch_one(pool)
    .await?)
}

async fn policy(
    transaction: &mut Transaction<'_, Postgres>,
    revision: i64,
) -> Result<(), IdentityError> {
    let current = sqlx::query_scalar!(
        r#"
        SELECT revision AS "revision!"
        FROM identity_oidc_policy
        WHERE singleton AND enabled
        FOR UPDATE
        "#,
    )
    .fetch_optional(&mut **transaction)
    .await?;
    if current != Some(revision) {
        return Err(IdentityError::Unauthorized);
    }
    Ok(())
}

#[tracing::instrument(skip_all, fields(revision = revision))]
pub(super) async fn store_flow(
    pool: &PgPool,
    revision: i64,
    state: &str,
    browser: &str,
    nonce: &str,
    link: Option<&str>,
) -> Result<(), IdentityError> {
    let mut transaction = pool.begin().await?;
    policy(&mut transaction, revision).await?;
    sqlx::query!(
        "
        DELETE FROM oidc_flows
        WHERE expires_at <= clock_timestamp()
        ",
    )
    .execute(&mut *transaction)
    .await?;
    let count = sqlx::query_scalar!(
        r#"
        SELECT count(*) AS "count!"
        FROM oidc_flows
        "#,
    )
    .fetch_one(&mut *transaction)
    .await?;
    if count >= 1000 {
        return Err(IdentityError::Busy);
    }
    let link = link.map(digest);
    if let Some(link) = &link {
        link_account(&mut transaction, link).await?;
    }
    sqlx::query!(
        "
        INSERT INTO oidc_flows (state_hash, browser_hash, nonce, policy_revision, link_session_hash)
        VALUES ($1, $2, $3, $4, $5)
        ",
        digest(state),
        digest(browser),
        nonce,
        revision,
        link,
    )
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

pub(super) struct Flow {
    pub nonce: String,
    pub policy_revision: i64,
    pub link_session_hash: Option<Vec<u8>>,
}

#[tracing::instrument(skip_all, fields(revision = revision))]
pub(super) async fn claim_flow(
    pool: &PgPool,
    revision: i64,
    state: &str,
    browser: &str,
) -> Result<Flow, IdentityError> {
    let mut transaction = pool.begin().await?;
    let found = sqlx::query_scalar!(
        r#"
        SELECT state_hash AS "state_hash!"
        FROM oidc_flows
        WHERE state_hash = $1 AND browser_hash = $2
        FOR UPDATE
        "#,
        digest(state),
        digest(browser),
    )
    .fetch_optional(&mut *transaction)
    .await?;
    found.ok_or(IdentityError::Unauthorized)?;
    let flow = sqlx::query_as!(
        Flow,
        r#"
        DELETE FROM oidc_flows f
        USING identity_oidc_policy p
        WHERE f.state_hash = $1
            AND f.expires_at > clock_timestamp()
            AND f.policy_revision = $2
            AND p.singleton
            AND p.enabled
            AND p.revision = f.policy_revision
        RETURNING
            f.nonce AS "nonce!",
            f.policy_revision AS "policy_revision!",
            f.link_session_hash AS "link_session_hash?"
        "#,
        digest(state),
        revision,
    )
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(IdentityError::Unauthorized)?;
    transaction.commit().await?;
    Ok(flow)
}

async fn link_account(
    transaction: &mut Transaction<'_, Postgres>,
    session: &[u8],
) -> Result<Uuid, IdentityError> {
    let account = sqlx::query_scalar!(
        r#"
        SELECT account_id AS "account_id!"
        FROM human_sessions
        WHERE token_hash = $1
        "#,
        session,
    )
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(IdentityError::Unauthorized)?;
    lock_account(transaction, account).await?;
    sqlx::query_scalar!(
        r#"
        SELECT a.id AS "id!"
        FROM human_sessions s
        JOIN human_accounts a ON a.id = s.account_id AND a.active
        WHERE s.token_hash = $1
            AND s.expires_at > clock_timestamp()
            AND s.created_at > clock_timestamp() - interval '10 minutes'
            AND (
                s.oidc_policy_revision IS NULL
                OR EXISTS(
                    SELECT 1
                    FROM identity_oidc_policy p
                    WHERE p.singleton AND p.enabled AND p.revision = s.oidc_policy_revision
                )
            )
        FOR SHARE OF s
        "#,
        session,
    )
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(IdentityError::Unauthorized)
}

#[tracing::instrument(skip_all, fields(revision = flow.policy_revision))]
pub(super) async fn admit(
    pool: &PgPool,
    config: &Config,
    flow: Flow,
    verified: Verified,
) -> Result<Session, IdentityError> {
    let mut transaction = pool.begin().await?;
    policy(&mut transaction, flow.policy_revision).await?;
    let link = match flow.link_session_hash.as_deref() {
        Some(session) => Some(link_account(&mut transaction, session).await?),
        None => None,
    };
    let account = resolve(&mut transaction, config, &verified, link).await?;
    let token = passwords::token();
    sqlx::query!(
        "
        INSERT INTO human_sessions (token_hash, account_id, expires_at, oidc_policy_revision)
        VALUES ($1, $2, clock_timestamp() + interval '1 day', $3)
        ",
        digest(&token),
        account,
        flow.policy_revision,
    )
    .execute(&mut *transaction)
    .await?;
    accounts::audit(&mut transaction, account, account, "oidc_sign_in").await?;
    transaction.commit().await?;
    Ok(Session { token })
}

async fn resolve(
    transaction: &mut Transaction<'_, Postgres>,
    config: &Config,
    verified: &Verified,
    link: Option<Uuid>,
) -> Result<Uuid, IdentityError> {
    let binding = sqlx::query!(
        r#"
        SELECT id AS "id!", account_id AS "account_id!", active AS "active!"
        FROM oidc_bindings
        WHERE issuer = $1 AND subject = $2
        "#,
        &config.issuer,
        &verified.subject,
    )
    .fetch_optional(&mut **transaction)
    .await?;
    if let Some(binding) = binding {
        let account = binding.account_id;
        if !binding.active || link.is_some_and(|link| link != account) {
            return Err(IdentityError::Forbidden);
        }
        lock_account(transaction, account).await?;
        let current = sqlx::query_scalar!(
            r#"
            SELECT active AS "active!"
            FROM oidc_bindings
            WHERE id = $1
            FOR SHARE
            "#,
            binding.id,
        )
        .fetch_one(&mut **transaction)
        .await?;
        if !current {
            return Err(IdentityError::Forbidden);
        }
        return Ok(account);
    }
    let account = new_binding_account(transaction, config, verified, link).await?;
    let inserted = sqlx::query_scalar!(
        r#"
        INSERT INTO oidc_bindings (issuer, subject, account_id)
        VALUES ($1, $2, $3)
        ON CONFLICT DO NOTHING
        RETURNING id AS "id!"
        "#,
        &config.issuer,
        &verified.subject,
        account,
    )
    .fetch_optional(&mut **transaction)
    .await?;
    inserted.ok_or(IdentityError::Conflict)?;
    accounts::audit(transaction, account, account, "oidc_identity_linked").await?;
    Ok(account)
}

async fn new_binding_account(
    transaction: &mut Transaction<'_, Postgres>,
    config: &Config,
    verified: &Verified,
    link: Option<Uuid>,
) -> Result<Uuid, IdentityError> {
    if let Some(account) = link {
        lock_account(transaction, account).await?;
        return Ok(account);
    }
    if !config.permits(
        &verified.subject,
        verified.email.as_deref(),
        verified.email_verified,
    ) {
        return Err(IdentityError::Forbidden);
    }
    let login = format!("oidc-{}", Uuid::new_v4().simple());
    let account = sqlx::query_scalar!(
        r#"
        INSERT INTO human_accounts (login, role)
        VALUES ($1, 'member')
        RETURNING id AS "id!"
        "#,
        login,
    )
    .fetch_one(&mut **transaction)
    .await?;
    Ok(account)
}

pub(super) async fn lock_account(
    transaction: &mut Transaction<'_, Postgres>,
    account: Uuid,
) -> Result<(), IdentityError> {
    let active = sqlx::query_scalar!(
        r#"
        SELECT id AS "id!"
        FROM human_accounts
        WHERE id = $1 AND active
        FOR UPDATE
        "#,
        account,
    )
    .fetch_optional(&mut **transaction)
    .await?;
    active.ok_or(IdentityError::Unauthorized)?;
    Ok(())
}

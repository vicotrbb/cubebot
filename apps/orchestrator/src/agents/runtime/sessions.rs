use super::*;
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

impl RuntimeIdentity {
    #[tracing::instrument(skip_all, fields(attempt = %attempt.id.0))]
    pub(super) async fn issue(
        &self,
        attempt: Attempt,
        reviewed: ReviewedPod,
        instance: Uuid,
    ) -> Result<RuntimeSession, Error> {
        let token = token();
        let mut transaction = self.pool.begin().await?;
        sqlx::query!("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
            .execute(&mut *transaction)
            .await?;
        // Agent-before-resource is shared with lifecycle and dispatch; never lock across HTTP.
        // Tests find this lock wait in pg_stat_activity by its `SELECT id FROM agents` prefix.
        sqlx::query_scalar!(
            "SELECT id FROM agents WHERE id = $1 FOR UPDATE",
            attempt.agent.0
        )
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query_scalar!(
            "
            SELECT id
            FROM agent_resources
            WHERE id = $1
              AND agent_id = $2
              AND kind = 'pod'
              AND phase = 'bound'
              AND uid = $3
            FOR UPDATE
            ",
            attempt.id.0,
            attempt.agent.0,
            reviewed.pod_uid,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(Error::Unauthorized)?;
        // The credential lasts for this sandbox incarnation; the same process may rotate it.
        let version = sqlx::query_scalar!(
            "
            INSERT INTO agent_runtime_sessions
                (attempt_id, instance_id, pod_uid, service_account_uid, token_hash)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (attempt_id) DO UPDATE
            SET token_hash = EXCLUDED.token_hash,
                version = agent_runtime_sessions.version + 1
            WHERE agent_runtime_sessions.instance_id = EXCLUDED.instance_id
              AND agent_runtime_sessions.pod_uid = EXCLUDED.pod_uid
              AND agent_runtime_sessions.service_account_uid = EXCLUDED.service_account_uid
              AND agent_runtime_sessions.revoked_at IS NULL
            RETURNING version
            ",
            attempt.id.0,
            instance,
            reviewed.pod_uid,
            reviewed.service_account_uid,
            digest(&token),
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(Error::Conflict)?;
        transaction.commit().await?;
        Ok(RuntimeSession {
            token,
            agent: attempt.agent,
            attempt: attempt.id,
            epoch: attempt.epoch,
            version,
        })
    }

    #[tracing::instrument(skip_all)]
    pub async fn authenticate(
        &self,
        token: &str,
        access: RuntimeAccess,
    ) -> Result<RuntimeActor, Error> {
        validate_token(token)?;
        let active = matches!(access, RuntimeAccess::Active);
        sqlx::query_as!(
            RuntimeActor,
            r#"
            SELECT agent_id AS "agent!: AgentId",
                   attempt_id AS "attempt!: AttemptId",
                   epoch AS "epoch!",
                   version AS "version!"
            FROM runtime_session_access
            WHERE token_hash = $1
              AND recovery
              AND (NOT $2 OR active)
            "#,
            digest(token),
            active,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(Error::Unauthorized)
    }
}
/// Revokes the agent credential of every incarnation of `agent` (ADR 0083). The sandbox keeps
/// running but can no longer write to the ledger, receive work or bootstrap again; a later
/// incarnation gets a new credential. Returns how many credentials were revoked.
#[tracing::instrument(skip_all, fields(agent = %agent.0))]
pub async fn revoke(pool: &sqlx::PgPool, agent: AgentId) -> Result<u64, Error> {
    let revoked = sqlx::query!(
        "
        UPDATE agent_runtime_sessions s
        SET revoked_at = clock_timestamp()
        FROM agent_resources r
        WHERE r.id = s.attempt_id
          AND r.agent_id = $1
          AND s.revoked_at IS NULL
        ",
        agent.0,
    )
    .execute(pool)
    .await?;
    Ok(revoked.rows_affected())
}
fn token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let secret: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("cb_runtime_{secret}")
}
pub(super) fn digest(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// A Docker sandbox bootstrap token: `cb_sandbox_` and 64 hex digits, minted per container.
pub(crate) fn sandbox_token() -> String {
    format!("cb_sandbox_{}", &token()["cb_runtime_".len()..])
}
pub(crate) fn sandbox_digest(token: &str) -> Result<Vec<u8>, Error> {
    let secret = token
        .strip_prefix("cb_sandbox_")
        .ok_or(Error::Unauthorized)?;
    if secret.len() != 64 || !secret.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::Unauthorized);
    }
    Ok(digest(token))
}

fn validate_token(token: &str) -> Result<(), Error> {
    let secret = token
        .strip_prefix("cb_runtime_")
        .ok_or(Error::Unauthorized)?;
    if secret.len() != 64 || !secret.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::Unauthorized);
    }
    Ok(())
}

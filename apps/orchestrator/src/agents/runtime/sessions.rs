use super::*;
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

impl RuntimeIdentity {
    pub(super) async fn issue(
        &self,
        attempt: Attempt,
        reviewed: ReviewedPod,
        instance: Uuid,
    ) -> Result<RuntimeSession, Error> {
        let token = token();
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
            .execute(&mut *transaction)
            .await?;
        // Agent-before-resource is shared with lifecycle and dispatch; never lock across HTTP.
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM agents WHERE id = $1 FOR UPDATE")
            .bind(attempt.agent)
            .fetch_one(&mut *transaction)
            .await?;
        sqlx::query_scalar::<_,Uuid>("SELECT id FROM agent_resources WHERE id = $1 AND agent_id = $2 AND kind = 'pod' AND phase = 'bound' AND uid = $3 FOR UPDATE")
            .bind(attempt.id).bind(attempt.agent).bind(&reviewed.pod_uid).fetch_optional(&mut *transaction).await?.ok_or(Error::Unauthorized)?;
        // The credential lasts for this sandbox incarnation; the same process may rotate it.
        let version: i64 = sqlx::query_scalar("INSERT INTO agent_runtime_sessions (attempt_id,instance_id,pod_uid,service_account_uid,token_hash) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (attempt_id) DO UPDATE SET token_hash = EXCLUDED.token_hash, version = agent_runtime_sessions.version + 1 WHERE agent_runtime_sessions.instance_id = EXCLUDED.instance_id AND agent_runtime_sessions.pod_uid = EXCLUDED.pod_uid AND agent_runtime_sessions.service_account_uid = EXCLUDED.service_account_uid RETURNING version")
            .bind(attempt.id).bind(instance).bind(&reviewed.pod_uid).bind(&reviewed.service_account_uid).bind(digest(&token)).fetch_optional(&mut *transaction).await?.ok_or(Error::Conflict)?;
        transaction.commit().await?;
        Ok(RuntimeSession {
            token,
            agent: attempt.agent,
            attempt: attempt.id,
            epoch: attempt.epoch,
            version,
        })
    }

    pub async fn authenticate(
        &self,
        token: &str,
        access: RuntimeAccess,
    ) -> Result<RuntimeActor, Error> {
        validate_token(token)?;
        let active = matches!(access, RuntimeAccess::Active);
        sqlx::query_as("SELECT agent_id AS agent,attempt_id AS attempt,epoch,version FROM runtime_session_access WHERE token_hash = $1 AND recovery AND (NOT $2 OR active)")
            .bind(digest(token)).bind(active).fetch_optional(&self.pool).await?.ok_or(Error::Unauthorized)
    }
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

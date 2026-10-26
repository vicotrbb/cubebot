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
        let version: i64 = sqlx::query_scalar("INSERT INTO agent_runtime_sessions (attempt_id,instance_id,pod_uid,service_account_uid,token_hash,expires_at) VALUES ($1,$2,$3,$4,$5,clock_timestamp() + interval '120 seconds') ON CONFLICT (attempt_id) DO UPDATE SET token_hash = EXCLUDED.token_hash, expires_at = EXCLUDED.expires_at, version = agent_runtime_sessions.version + 1 WHERE agent_runtime_sessions.instance_id = EXCLUDED.instance_id AND agent_runtime_sessions.pod_uid = EXCLUDED.pod_uid AND agent_runtime_sessions.service_account_uid = EXCLUDED.service_account_uid RETURNING version")
            .bind(attempt.id).bind(instance).bind(&reviewed.pod_uid).bind(&reviewed.service_account_uid).bind(digest(&token)).fetch_optional(&mut *transaction).await?.ok_or(Error::Conflict)?;
        transaction.commit().await?;
        Ok(RuntimeSession {
            token,
            agent: attempt.agent,
            attempt: attempt.id,
            epoch: attempt.epoch,
            version,
            expires_in_seconds: 120,
        })
    }

    pub async fn authenticate(
        &self,
        token: &str,
        access: RuntimeAccess,
    ) -> Result<RuntimeActor, Error> {
        let Some(secret) = token.strip_prefix("cb_runtime_") else {
            return Err(Error::Unauthorized);
        };
        if secret.len() != 64 || !secret.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error::Unauthorized);
        }
        let active = matches!(access, RuntimeAccess::Active);
        sqlx::query_as("SELECT a.id AS agent,r.id AS attempt,r.epoch,s.version FROM agent_runtime_sessions s JOIN agent_resources r ON r.id = s.attempt_id JOIN agents a ON a.id = r.agent_id WHERE s.token_hash = $1 AND s.expires_at > clock_timestamp() AND r.kind = 'pod' AND r.phase = 'bound' AND r.uid = s.pod_uid AND (NOT $2 OR (a.desired_state = 'running' AND a.restart_generation = r.target_generation AND r.blocked_reason IS NULL))")
            .bind(digest(token)).bind(active).fetch_optional(&self.pool).await?.ok_or(Error::Unauthorized)
    }
}
fn token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let secret: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("cb_runtime_{secret}")
}
fn digest(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

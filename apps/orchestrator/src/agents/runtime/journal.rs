//! Immutable recovery custody. Typed outcome reconciliation remains separate.
use super::{Error, RuntimeAccess, RuntimeIdentity};
use cubebot_protocol::runtime::{JournalReceipt, JournalRecord};
use sha2::{Digest, Sha256};
impl RuntimeIdentity {
    pub async fn retain_journal(
        &self,
        token: &str,
        record: &JournalRecord,
    ) -> Result<JournalReceipt, Error> {
        if record.sequence == 0 || record.sequence > i64::MAX as u64 || record.payload.len() > 65536
        {
            return Err(Error::InvalidConfiguration);
        }
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.retain_locked(token, record),
        )
        .await
        .map_err(|_| Error::DeadlineExceeded)?
    }
    async fn retain_locked(
        &self,
        token: &str,
        record: &JournalRecord,
    ) -> Result<JournalReceipt, Error> {
        let actor = self.authenticate(token, RuntimeAccess::Recovery).await?;
        let mut transaction = self.lock_runtime(&actor).await?;
        self.recheck(token, &actor, &mut transaction).await?;
        sqlx::query(
            "INSERT INTO agent_journal_streams (agent_id) VALUES ($1) ON CONFLICT DO NOTHING",
        )
        .bind(actor.agent)
        .execute(&mut *transaction)
        .await?;
        let through: i64 = sqlx::query_scalar(
            "SELECT received_through FROM agent_journal_streams WHERE agent_id = $1",
        )
        .bind(actor.agent)
        .fetch_one(&mut *transaction)
        .await?;
        let sequence = record.sequence as i64;
        let digest: [u8; 32] = Sha256::digest(&record.payload).into();
        if sequence <= through {
            let payload: Vec<u8> = sqlx::query_scalar(
                "SELECT payload FROM agent_journal_records WHERE agent_id = $1 AND sequence = $2",
            )
            .bind(actor.agent)
            .bind(sequence)
            .fetch_one(&mut *transaction)
            .await?;
            if payload != record.payload {
                return Err(Error::Conflict);
            }
        } else {
            if through.checked_add(1) != Some(sequence) {
                return Err(Error::Conflict);
            }
            insert(
                &mut transaction,
                actor.agent,
                actor.attempt,
                record,
                &digest,
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(JournalReceipt {
            actor: actor.into(),
            sequence: record.sequence,
            payload_digest: digest,
        })
    }
}
async fn insert(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    agent: super::AgentId,
    attempt: super::AttemptId,
    record: &JournalRecord,
    digest: &[u8; 32],
) -> Result<(), Error> {
    sqlx::query("INSERT INTO agent_journal_records (agent_id,sequence,payload,payload_digest,reported_attempt) VALUES ($1,$2,$3,$4,$5)").bind(agent).bind(record.sequence as i64).bind(&record.payload).bind(digest.as_slice()).bind(attempt).execute(&mut **transaction).await?;
    sqlx::query("UPDATE agent_journal_streams SET received_through = $2 WHERE agent_id = $1")
        .bind(agent)
        .bind(record.sequence as i64)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

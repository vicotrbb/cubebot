//! Atomic receipts and notifications for non-secret identity mutations.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::{Actor, IdentityError, accounts};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub id: Uuid,
    pub revision: i64,
}

pub(super) struct Command {
    actor: Uuid,
    kind: &'static str,
    request_id: Uuid,
    digest: Vec<u8>,
    input: serde_json::Value,
}

impl Command {
    pub fn new(
        actor: &Actor,
        kind: &'static str,
        request_id: Uuid,
        input: &impl Serialize,
    ) -> Result<Self, IdentityError> {
        if request_id.is_nil() {
            return Err(IdentityError::InvalidInput);
        }
        let input = serde_json::to_value(input).map_err(|_| IdentityError::InvalidInput)?;
        let bytes = serde_json::to_vec(&input).map_err(|_| IdentityError::InvalidInput)?;
        Ok(Self {
            actor: actor.id,
            kind,
            request_id,
            digest: Sha256::digest(bytes).to_vec(),
            input,
        })
    }

    pub async fn begin<'a>(
        &self,
        pool: &'a PgPool,
    ) -> Result<(Transaction<'a, Postgres>, Option<Receipt>), IdentityError> {
        let mut transaction = pool.begin().await?;
        // This lock also serializes command retries from the same administrator.
        accounts::lock_administrator(&mut transaction, self.actor).await?;
        let previous: Option<(Vec<u8>, Uuid, i64)> = sqlx::query_as(
            "SELECT input_digest, outcome_id, outcome_revision FROM identity_commands WHERE actor_id = $1 AND kind = $2 AND request_id = $3",
        )
        .bind(self.actor).bind(self.kind).bind(self.request_id)
        .fetch_optional(&mut *transaction).await?;
        let receipt = self.replay(previous)?;
        Ok((transaction, receipt))
    }

    fn replay(
        &self,
        previous: Option<(Vec<u8>, Uuid, i64)>,
    ) -> Result<Option<Receipt>, IdentityError> {
        let Some((digest, id, revision)) = previous else {
            return Ok(None);
        };
        if digest != self.digest {
            return Err(IdentityError::Conflict);
        }
        Ok(Some(Receipt { id, revision }))
    }

    pub async fn commit(
        &self,
        mut transaction: Transaction<'_, Postgres>,
        receipt: Receipt,
    ) -> Result<Receipt, IdentityError> {
        // A cursor must never advance beyond an uncommitted lower event sequence.
        // ponytail: serialize this short publication; add a watermark if measured throughput requires it.
        sqlx::query("SELECT singleton FROM installation WHERE singleton FOR UPDATE")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("INSERT INTO identity_commands (actor_id, kind, request_id, input_digest, outcome_id, outcome_revision, input_body) VALUES ($1, $2, $3, $4, $5, $6, $7)")
            .bind(self.actor).bind(self.kind).bind(self.request_id).bind(&self.digest)
            .bind(receipt.id).bind(receipt.revision).bind(sqlx::types::Json(&self.input)).execute(&mut *transaction).await?;
        sqlx::query("INSERT INTO identity_events (actor_id, kind, request_id, resource_id, revision) VALUES ($1, $2, $3, $4, $5)")
            .bind(self.actor).bind(self.kind).bind(self.request_id).bind(receipt.id).bind(receipt.revision)
            .execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(receipt)
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct IdentityEvent {
    pub sequence: i64,
    pub actor_id: Uuid,
    pub kind: String,
    pub request_id: Uuid,
    pub resource_id: Uuid,
    pub revision: i64,
    pub input: sqlx::types::Json<serde_json::Value>,
}

impl super::Identity {
    pub async fn command_receipt(
        &self,
        actor: &Actor,
        kind: &str,
        request_id: Uuid,
    ) -> Result<Receipt, IdentityError> {
        self.require_administrator(actor).await?;
        let receipt: Option<(Uuid, i64)> = sqlx::query_as("SELECT outcome_id, outcome_revision FROM identity_commands WHERE actor_id = $1 AND kind = $2 AND request_id = $3")
            .bind(actor.id).bind(kind).bind(request_id).fetch_optional(&self.pool).await?;
        let (id, revision) = receipt.ok_or(IdentityError::NotFound)?;
        Ok(Receipt { id, revision })
    }

    pub async fn access_events(
        &self,
        actor: &Actor,
        after: i64,
    ) -> Result<Vec<IdentityEvent>, IdentityError> {
        self.require_administrator(actor).await?;
        Ok(sqlx::query_as("SELECT e.sequence, e.actor_id, e.kind, e.request_id, e.resource_id, e.revision, c.input_body AS input FROM identity_events e JOIN identity_commands c USING (actor_id, kind, request_id) WHERE e.sequence > $1 ORDER BY e.sequence LIMIT 100")
            .bind(after).fetch_all(&self.pool).await?)
    }
}

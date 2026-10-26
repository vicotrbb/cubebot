//! Atomic identity receipts and notifications with non-secret audit payloads.

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
    secret_verifier: Option<String>,
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
            secret_verifier: None,
        })
    }

    pub fn with_password_verifier(mut self, verifier: String) -> Self {
        self.secret_verifier = Some(verifier);
        self
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
        sqlx::query("INSERT INTO identity_commands (actor_id, kind, request_id, input_digest, outcome_id, outcome_revision, input_body, secret_verifier) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
            .bind(self.actor).bind(self.kind).bind(self.request_id).bind(&self.digest)
            .bind(receipt.id).bind(receipt.revision).bind(sqlx::types::Json(&self.input)).bind(&self.secret_verifier).execute(&mut *transaction).await?;
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

impl super::Identity {
    pub(super) async fn verify_command_password(
        &self,
        account: Uuid,
        kind: &str,
        request_id: Uuid,
        password: &str,
    ) -> Result<(), IdentityError> {
        if password.len() > 1024 {
            return Err(IdentityError::Conflict);
        }
        let verifier: Option<String> = sqlx::query_scalar("SELECT c.secret_verifier FROM identity_commands c JOIN human_accounts a ON a.id = c.actor_id AND a.active WHERE c.actor_id = $1 AND c.kind = $2 AND c.request_id = $3 AND c.secret_verifier IS NOT NULL")
            .bind(account).bind(kind).bind(request_id).fetch_optional(&self.pool).await?;
        let verifier = verifier.ok_or(IdentityError::Unauthorized)?;
        self.reserve_password_check(account).await?;
        let verified = super::passwords::verify(password, verifier).await;
        if matches!(verified, Err(IdentityError::Unauthorized)) {
            return Err(IdentityError::Conflict);
        }
        verified?;
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM human_accounts WHERE id = $1 AND active)",
        )
        .bind(account)
        .fetch_one(&self.pool)
        .await?;
        if !active {
            return Err(IdentityError::Unauthorized);
        }
        Ok(())
    }
}

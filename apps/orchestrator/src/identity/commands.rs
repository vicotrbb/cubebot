//! Atomic identity receipts and notifications with non-secret audit payloads.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::{Actor, IdentityError, accounts};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
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
        let input =
            serde_json::to_value(input).map_err(|error| IdentityError::Invalid(error.into()))?;
        let bytes =
            serde_json::to_vec(&input).map_err(|error| IdentityError::Invalid(error.into()))?;
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
        let previous = sqlx::query!(
            r#"
            SELECT
                input_digest AS "input_digest!",
                outcome_id AS "outcome_id!",
                outcome_revision AS "outcome_revision!"
            FROM identity_commands
            WHERE actor_id = $1 AND kind = $2 AND request_id = $3
            "#,
            self.actor,
            self.kind,
            self.request_id,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .map(|row| (row.input_digest, row.outcome_id, row.outcome_revision));
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
        sqlx::query!(
            "
            SELECT singleton
            FROM installation
            WHERE singleton
            FOR UPDATE
            ",
        )
        .fetch_all(&mut *transaction)
        .await?;
        self.record(&mut transaction, receipt).await?;
        transaction.commit().await?;
        Ok(receipt)
    }

    async fn record(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        receipt: Receipt,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "
            INSERT INTO identity_commands
                (actor_id, kind, request_id, input_digest, outcome_id, outcome_revision, input_body, secret_verifier)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ",
            self.actor,
            self.kind,
            self.request_id,
            &self.digest,
            receipt.id,
            receipt.revision,
            sqlx::types::Json(&self.input) as _,
            self.secret_verifier.as_deref(),
        )
        .execute(&mut **transaction)
        .await?;
        sqlx::query!(
            "
            INSERT INTO identity_events (actor_id, kind, request_id, resource_id, revision)
            VALUES ($1, $2, $3, $4, $5)
            ",
            self.actor,
            self.kind,
            self.request_id,
            receipt.id,
            receipt.revision,
        )
        .execute(&mut **transaction)
        .await?;
        Ok(())
    }
}

#[derive(Debug, Serialize)]
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
    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %request_id))]
    pub async fn command_receipt(
        &self,
        actor: &Actor,
        kind: &str,
        request_id: Uuid,
    ) -> Result<Receipt, IdentityError> {
        self.require_administrator(actor).await?;
        let receipt = sqlx::query_as!(
            Receipt,
            r#"
            SELECT outcome_id AS "id!", outcome_revision AS "revision!"
            FROM identity_commands
            WHERE actor_id = $1 AND kind = $2 AND request_id = $3
            "#,
            actor.id,
            kind,
            request_id,
        )
        .fetch_optional(&self.pool)
        .await?;
        receipt.ok_or(IdentityError::NotFound)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn access_events(
        &self,
        actor: &Actor,
        after: i64,
    ) -> Result<Vec<IdentityEvent>, IdentityError> {
        self.require_administrator(actor).await?;
        Ok(sqlx::query_as!(
            IdentityEvent,
            r#"
            SELECT
                e.sequence AS "sequence!",
                e.actor_id AS "actor_id!",
                e.kind AS "kind!",
                e.request_id AS "request_id!",
                e.resource_id AS "resource_id!",
                e.revision AS "revision!",
                c.input_body AS "input!: sqlx::types::Json<serde_json::Value>"
            FROM identity_events e
            JOIN identity_commands c USING (actor_id, kind, request_id)
            WHERE e.sequence > $1
            ORDER BY e.sequence
            LIMIT 100
            "#,
            after,
        )
        .fetch_all(&self.pool)
        .await?)
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
        let verifier = sqlx::query_scalar!(
            r#"
            SELECT c.secret_verifier AS "secret_verifier!"
            FROM identity_commands c
            JOIN human_accounts a ON a.id = c.actor_id AND a.active
            WHERE c.actor_id = $1
                AND c.kind = $2
                AND c.request_id = $3
                AND c.secret_verifier IS NOT NULL
            "#,
            account,
            kind,
            request_id,
        )
        .fetch_optional(&self.pool)
        .await?;
        let verifier = verifier.ok_or(IdentityError::Unauthorized)?;
        self.reserve_password_check(account).await?;
        let verified = super::passwords::verify(password, verifier).await;
        if matches!(verified, Err(IdentityError::Unauthorized)) {
            return Err(IdentityError::Conflict);
        }
        verified?;
        let active = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1
                FROM human_accounts
                WHERE id = $1 AND active
            ) AS "active!"
            "#,
            account,
        )
        .fetch_one(&self.pool)
        .await?;
        if !active {
            return Err(IdentityError::Unauthorized);
        }
        Ok(())
    }
}

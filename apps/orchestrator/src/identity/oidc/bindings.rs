use crate::identity::{
    Actor, Identity, IdentityError, accounts,
    commands::{Command, Receipt},
};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
pub struct Binding {
    pub id: Uuid,
    pub account_id: Uuid,
    pub issuer: String,
    pub subject: String,
    pub active: bool,
    pub revision: i64,
}

impl Identity {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn oidc_bindings(
        &self,
        actor: &Actor,
        after: Option<Uuid>,
    ) -> Result<Vec<Binding>, IdentityError> {
        self.require_administrator(actor).await?;
        Ok(sqlx::query_as!(
            Binding,
            r#"
            SELECT
                id AS "id!",
                account_id AS "account_id!",
                issuer AS "issuer!",
                subject AS "subject!",
                active AS "active!",
                revision AS "revision!"
            FROM oidc_bindings
            WHERE ($1::uuid IS NULL OR id > $1)
            ORDER BY id
            LIMIT 100
            "#,
            after,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, binding = %id, request = %request_id))]
    pub async fn revoke_oidc_binding(
        &self,
        actor: &Actor,
        id: Uuid,
        expected_revision: i64,
        request_id: Uuid,
    ) -> Result<Receipt, IdentityError> {
        let input = serde_json::json!({"id": id, "expected_revision": expected_revision});
        let command = Command::new(actor, "oidc.revoke", request_id, &input)?;
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        let account = sqlx::query_scalar!(
            r#"
            SELECT account_id AS "account_id!"
            FROM oidc_bindings
            WHERE id = $1
            "#,
            id,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IdentityError::NotFound)?;
        sqlx::query!(
            "
            SELECT id
            FROM human_accounts
            WHERE id = $1
            FOR UPDATE
            ",
            account,
        )
        .fetch_all(&mut *transaction)
        .await?;
        let revision = sqlx::query_scalar!(
            r#"
            UPDATE oidc_bindings
            SET active = false, revision = revision + 1
            WHERE id = $1 AND active AND revision = $2
            RETURNING revision AS "revision!"
            "#,
            id,
            expected_revision,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IdentityError::Conflict)?;
        sqlx::query!(
            "
            DELETE FROM human_sessions
            WHERE account_id = $1 AND oidc_policy_revision IS NOT NULL
            ",
            account,
        )
        .execute(&mut *transaction)
        .await?;
        accounts::audit(&mut transaction, actor.id, account, "oidc_identity_revoked").await?;
        command.commit(transaction, Receipt { id, revision }).await
    }
}

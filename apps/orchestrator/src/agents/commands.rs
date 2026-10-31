//! Agent command authorization and atomic receipt/event publication.
use super::AgentId;
use crate::identity::{Actor, IdentityError, Receipt};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub(super) struct Command {
    actor: Uuid,
    resource: Option<AgentId>,
    kind: &'static str,
    request_id: Uuid,
    digest: Vec<u8>,
    body: serde_json::Value,
}

impl Command {
    pub fn new(
        actor: &Actor,
        resource: Option<AgentId>,
        kind: &'static str,
        request_id: Uuid,
        input: &impl Serialize,
    ) -> Result<Self, IdentityError> {
        if request_id.is_nil() {
            return Err(IdentityError::InvalidInput);
        }
        let body = serde_json::json!({"agent":resource, "input":input});
        let bytes = match resource {
            None => serde_json::to_vec(input),
            Some(_) => serde_json::to_vec(&body),
        }
        .map_err(|error| IdentityError::Invalid(error.into()))?;
        let digest = Sha256::digest(bytes).to_vec();
        Ok(Self {
            actor: actor.id,
            resource,
            kind,
            request_id,
            digest,
            body,
        })
    }

    #[tracing::instrument(skip_all, fields(kind = self.kind))]
    pub async fn begin<'a>(
        &self,
        pool: &'a PgPool,
    ) -> Result<(Transaction<'a, Postgres>, Option<Receipt>), IdentityError> {
        let mut transaction = pool.begin().await?;
        sqlx::query!("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
            .execute(&mut *transaction)
            .await?;
        let admin = sqlx::query_scalar!(
            r#"
            SELECT role = 'administrator' AS "administrator!"
            FROM human_accounts
            WHERE id = $1
              AND active
            FOR UPDATE
            "#,
            self.actor,
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let admin = admin.ok_or(IdentityError::Unauthorized)?;
        match self.resource {
            Some(id) => lock_agent(&mut transaction, self.actor, id).await?,
            None if !admin => return Err(IdentityError::Forbidden),
            None => (),
        }
        let previous = sqlx::query!(
            "
            SELECT input_digest, agent_id, revision
            FROM agent_commands
            WHERE actor_id = $1
              AND kind = $2
              AND request_id = $3
            ",
            self.actor,
            self.kind,
            self.request_id,
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let previous = match previous {
            Some(row) if row.input_digest == self.digest => Some(Receipt {
                id: row.agent_id,
                revision: row.revision,
            }),
            Some(_) => return Err(IdentityError::Conflict),
            None => None,
        };
        Ok((transaction, previous))
    }

    #[tracing::instrument(skip_all, fields(kind = self.kind))]
    pub async fn commit(
        self,
        mut transaction: Transaction<'_, Postgres>,
        receipt: Receipt,
    ) -> Result<Receipt, IdentityError> {
        sqlx::query!(
            "
            INSERT INTO agent_commands
                (actor_id, kind, request_id, input_digest, agent_id, revision, input_body)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ",
            self.actor,
            self.kind,
            self.request_id,
            self.digest,
            receipt.id,
            receipt.revision,
            self.body,
        )
        .execute(&mut *transaction)
        .await?;
        // ponytail: share the short publication barrier; use watermarks if throughput requires it.
        sqlx::query!(
            "
            SELECT singleton
            FROM installation
            WHERE singleton
            FOR UPDATE
            "
        )
        .fetch_all(&mut *transaction)
        .await?;
        sqlx::query!(
            "
            INSERT INTO agent_events (actor_id, kind, request_id, agent_id, revision)
            VALUES ($1, $2, $3, $4, $5)
            ",
            self.actor,
            self.kind,
            self.request_id,
            receipt.id,
            receipt.revision,
        )
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(receipt)
    }
}

async fn lock_agent(
    transaction: &mut Transaction<'_, Postgres>,
    actor: Uuid,
    id: AgentId,
) -> Result<(), IdentityError> {
    let found = sqlx::query_scalar!(
        "
        SELECT g.id
        FROM agents g
        JOIN human_accounts a ON a.id = $1 AND a.active
        WHERE g.id = $2
          AND (
            a.role = 'administrator'
            OR EXISTS(
                SELECT 1
                FROM effective_human_grants p
                WHERE p.actor_id = a.id
                  AND p.scope_kind = 'agent'
                  AND p.scope_id = g.id
                  AND p.capability = 'agent.manage'
            )
          )
        FOR UPDATE OF g
        ",
        actor,
        id.0,
    )
    .fetch_optional(&mut **transaction)
    .await?;
    found.ok_or(IdentityError::NotFound)?;
    // The locking SELECT may have waited after evaluating its grant snapshot.
    let allowed = sqlx::query_scalar!(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM human_accounts a
            WHERE a.id = $1
              AND a.active
              AND (
                a.role = 'administrator'
                OR EXISTS(
                    SELECT 1
                    FROM effective_human_grants p
                    WHERE p.actor_id = a.id
                      AND p.scope_kind = 'agent'
                      AND p.scope_id = $2
                      AND p.capability = 'agent.manage'
                )
              )
        ) AS "allowed!"
        "#,
        actor,
        id.0,
    )
    .fetch_one(&mut **transaction)
    .await?;
    if !allowed {
        return Err(IdentityError::NotFound);
    }
    Ok(())
}

pub(super) async fn snapshot(
    transaction: &mut Transaction<'_, Postgres>,
    actor: Uuid,
    id: AgentId,
) -> Result<(), IdentityError> {
    sqlx::query!(
        "
        INSERT INTO agent_definition_revisions
            (agent_id, revision, instructions_revision, name, instructions, idle_policy,
             idle_delay_seconds, dreaming_model, task_lanes, actor_id)
        SELECT id, revision, instructions_revision, name, instructions, idle_policy,
               idle_delay_seconds, dreaming_model, task_lanes, $2
        FROM agents
        WHERE id = $1
        ",
        id.0,
        actor,
    )
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn creation_digest_remains_compatible_with_existing_receipts() {
        let actor = Actor {
            id: Uuid::new_v4(),
            administrator: true,
        };
        let input = crate::agents::CreateAgent {
            request_id: Uuid::new_v4(),
            name: "Agent".into(),
            instructions: "Existing definition".into(),
        };
        let command = Command::new(&actor, None, "agent.create", input.request_id, &input).unwrap();
        let original = Sha256::digest(serde_json::to_vec(&input).unwrap()).to_vec();
        assert_eq!(command.digest, original);
    }
}

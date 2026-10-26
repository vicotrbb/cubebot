use super::Agents;
use crate::identity::{Actor, IdentityError, Receipt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(transparent)]
#[sqlx(transparent)]
pub struct AgentId(pub Uuid);

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Agent {
    pub id: AgentId,
    pub name: String,
    pub instructions: String,
    pub revision: i64,
    pub instructions_revision: i64,
    pub desired_state: String,
    pub idle_policy: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAgent {
    pub request_id: Uuid,
    pub name: String,
    pub instructions: String,
}

impl Agents {
    pub async fn create(
        &self,
        actor: &Actor,
        input: &CreateAgent,
    ) -> Result<Receipt, IdentityError> {
        validate(input)?;
        let mut transaction = self.pool.begin().await?;
        let allowed: Option<Uuid> = sqlx::query_scalar("SELECT id FROM human_accounts WHERE id = $1 AND active AND role = 'administrator' FOR UPDATE")
            .bind(actor.id).fetch_optional(&mut *transaction).await?;
        allowed.ok_or(IdentityError::Forbidden)?;
        let digest =
            Sha256::digest(serde_json::to_vec(input).map_err(|_| IdentityError::InvalidInput)?)
                .to_vec();
        let previous: Option<(Vec<u8>, Uuid, i64)> = sqlx::query_as("SELECT input_digest, agent_id, revision FROM agent_commands WHERE actor_id = $1 AND kind = 'agent.create' AND request_id = $2")
            .bind(actor.id).bind(input.request_id).fetch_optional(&mut *transaction).await?;
        if let Some((previous, id, revision)) = previous {
            if previous != digest {
                return Err(IdentityError::Conflict);
            }
            return Ok(Receipt { id, revision });
        }
        let id = sqlx::query_scalar(
            "INSERT INTO agents (name, instructions, created_by) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(&input.name)
        .bind(&input.instructions)
        .bind(actor.id)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query("INSERT INTO agent_commands (actor_id, kind, request_id, input_digest, agent_id, revision) VALUES ($1, 'agent.create', $2, $3, $4, 1)")
            .bind(actor.id).bind(input.request_id).bind(digest).bind(id).execute(&mut *transaction).await?;
        // Share the existing short publication barrier so committed sequence cursors cannot skip events.
        sqlx::query("SELECT singleton FROM installation WHERE singleton FOR UPDATE")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("INSERT INTO agent_events (actor_id, kind, request_id, agent_id, revision) VALUES ($1, 'agent.create', $2, $3, 1)")
            .bind(actor.id).bind(input.request_id).bind(id).execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(Receipt { id, revision: 1 })
    }

    pub async fn list(
        &self,
        actor: &Actor,
        after: Option<AgentId>,
    ) -> Result<Vec<Agent>, IdentityError> {
        Ok(sqlx::query_as("SELECT g.id, g.name, g.instructions, g.revision, g.instructions_revision, g.desired_state, g.idle_policy FROM agents g JOIN human_accounts a ON a.id = $1 AND a.active WHERE ($2::uuid IS NULL OR g.id > $2) AND (a.role = 'administrator' OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id = a.id AND p.scope_kind = 'agent' AND p.scope_id = g.id AND p.capability = 'agent.manage')) ORDER BY g.id LIMIT 100")
            .bind(actor.id).bind(after).fetch_all(&self.pool).await?)
    }

    pub async fn get(&self, actor: &Actor, id: AgentId) -> Result<Agent, IdentityError> {
        sqlx::query_as("SELECT g.id, g.name, g.instructions, g.revision, g.instructions_revision, g.desired_state, g.idle_policy FROM agents g JOIN human_accounts a ON a.id = $1 AND a.active WHERE g.id = $2 AND (a.role = 'administrator' OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id = a.id AND p.scope_kind = 'agent' AND p.scope_id = g.id AND p.capability = 'agent.manage'))")
            .bind(actor.id).bind(id).fetch_optional(&self.pool).await?.ok_or(IdentityError::NotFound)
    }
}

fn validate(input: &CreateAgent) -> Result<(), IdentityError> {
    if input.request_id.is_nil()
        || input.name.trim().is_empty()
        || input.name.len() > 255
        || input.instructions.len() > 65536
        || input.instructions.contains('\0')
        || input.name.chars().any(char::is_control)
    {
        return Err(IdentityError::InvalidInput);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instructions_reject_null_bytes_before_postgresql_encoding() {
        let input = CreateAgent {
            request_id: Uuid::new_v4(),
            name: "Researcher".into(),
            instructions: "Evidence\0with null".into(),
        };
        assert!(matches!(validate(&input), Err(IdentityError::InvalidInput)));
    }
}

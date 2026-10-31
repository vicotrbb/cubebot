use crate::{
    agents::AgentId,
    identity::{Actor, IdentityError},
};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(super) async fn lock_actor(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
) -> Result<(), IdentityError> {
    sqlx::query_scalar!(
        "
        SELECT id
        FROM human_accounts
        WHERE id=$1 AND active
        FOR UPDATE
        ",
        actor.id,
    )
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(IdentityError::Unauthorized)?;
    Ok(())
}

pub(super) async fn conversation(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    id: Uuid,
    capability: &str,
) -> Result<AgentId, IdentityError> {
    sqlx::query_scalar!(
        r#"
        SELECT c.agent_id AS "agent_id!: AgentId"
        FROM conversations c
        JOIN human_accounts a ON a.id=$1 AND a.active
        WHERE c.id=COALESCE((SELECT id FROM agents WHERE id=$2),(SELECT agent_id FROM conversation_origins WHERE id=$2))
            AND (a.role='administrator' OR EXISTS(SELECT 1 FROM effective_human_grants g WHERE g.actor_id=a.id AND g.scope_kind='conversation' AND g.scope_id=c.id AND g.capability=$3))
        "#,
        actor.id,
        id,
        capability,
    )
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(IdentityError::NotFound)
}

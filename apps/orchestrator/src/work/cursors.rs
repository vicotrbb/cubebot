use super::{Work, access, rooms};
use crate::identity::{Actor, IdentityError};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadCursor {
    pub sequence: i64,
}

impl Work {
    #[tracing::instrument(skip_all, fields(actor = %actor.id, conversation = %conversation))]
    pub async fn read_conversation(
        &self,
        actor: &Actor,
        conversation: Uuid,
        sequence: i64,
    ) -> Result<ReadCursor, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let agent = access::conversation(&mut tx, actor, conversation, "conversation.read").await?;
        let cursor = store(&mut tx, actor, "conversation", agent.0, sequence).await?;
        tx.commit().await?;
        Ok(cursor)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, room = %room))]
    pub async fn read_room(
        &self,
        actor: &Actor,
        room: Uuid,
        sequence: i64,
    ) -> Result<ReadCursor, IdentityError> {
        let mut tx = self.pool.begin().await?;
        rooms::authorize(&mut tx, actor, room, "conversation.read").await?;
        let cursor = store(&mut tx, actor, "room", room, sequence).await?;
        tx.commit().await?;
        Ok(cursor)
    }
}

pub(super) async fn store(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    kind: &str,
    scope: Uuid,
    sequence: i64,
) -> Result<ReadCursor, IdentityError> {
    if sequence < 0 {
        return Err(IdentityError::InvalidInput);
    }
    let sequence = sqlx::query_scalar!(
        "
        INSERT INTO read_cursors (account_id, scope_kind, scope_id, sequence)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (account_id, scope_kind, scope_id)
        DO UPDATE SET sequence = GREATEST(read_cursors.sequence, EXCLUDED.sequence)
        RETURNING sequence
        ",
        actor.id,
        kind,
        scope,
        sequence,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(ReadCursor { sequence })
}

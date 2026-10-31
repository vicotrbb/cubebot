//! Browser view of a conversation's turn events: replay after a cursor, then follow live.
use super::{Turn, TurnRow, Work, access};
use crate::identity::{Actor, IdentityError};
use cubebot_protocol::runtime::TurnEvent;
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize, utoipa::ToSchema)]
pub struct Event {
    /// Conversation-wide cursor; resume with `after` or `Last-Event-ID`.
    pub id: i64,
    pub turn_id: Uuid,
    #[schema(value_type = TurnEvent)]
    pub payload: serde_json::Value,
}

/// One poll of a conversation: events after the cursor and the turns that may have changed.
pub struct Page {
    pub events: Vec<Event>,
    pub turns: Vec<Turn>,
}

impl Work {
    /// Without a cursor, replay starts at the running turn's events so its partial text shows,
    /// otherwise at the latest event.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, conversation = %conversation))]
    pub async fn event_cursor(
        &self,
        actor: &Actor,
        conversation: Uuid,
    ) -> Result<i64, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let canonical =
            access::conversation(&mut tx, actor, conversation, "conversation.read").await?;
        let cursor = sqlx::query_scalar!(
            r#"
            SELECT COALESCE(
                (SELECT min(e.id)-1 FROM turn_events e JOIN chat_turns t ON t.id=e.turn_id WHERE t.conversation_id=$1 AND t.status='running'),
                (SELECT max(e.id) FROM turn_events e JOIN chat_turns t ON t.id=e.turn_id WHERE t.conversation_id=$1),
                0
            ) AS "cursor!"
            "#,
            canonical.0,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(cursor)
    }

    /// Rechecks access on every poll. `touched` names turns whose last state the caller holds.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, conversation = %conversation))]
    pub async fn events(
        &self,
        actor: &Actor,
        conversation: Uuid,
        after: i64,
        touched: &[Uuid],
    ) -> Result<Page, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let canonical =
            access::conversation(&mut tx, actor, conversation, "conversation.read").await?;
        let events = sqlx::query_as!(
            Event,
            r#"
            SELECT e.id, e.turn_id, e.payload #- '{outcome,image}' AS "payload!"
            FROM turn_events e
            JOIN chat_turns t ON t.id=e.turn_id
            WHERE t.conversation_id=$1 AND e.id>$2
            ORDER BY e.id
            LIMIT 100
            "#,
            canonical.0,
            after,
        )
        .fetch_all(&mut *tx)
        .await?;
        let ids: Vec<Uuid> = events
            .iter()
            .map(|event| event.turn_id)
            .chain(touched.iter().copied())
            .collect();
        let mut turns: Vec<Turn> = sqlx::query_as!(
            TurnRow,
            "
            SELECT id, sequence, message, status, response, error
            FROM chat_turns
            WHERE conversation_id=$1 AND (status IN ('queued','running','waiting','delegated') OR id=ANY($2))
            ORDER BY sequence
            ",
            canonical.0,
            &ids,
        )
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(Turn::from)
        .collect();
        Self::progress(&mut tx, &mut turns).await?;
        tx.commit().await?;
        Ok(Page { events, turns })
    }
}

use super::{Work, access};
use crate::identity::{Actor, IdentityError};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Clone, Copy, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessagePart {
    Message,
    Response,
}

impl Work {
    #[tracing::instrument(skip_all, fields(actor = %actor.id, conversation = %conversation, turn = %turn))]
    pub async fn delete_message(
        &self,
        actor: &Actor,
        conversation: Uuid,
        turn: Uuid,
        part: MessagePart,
    ) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        let agent = access::conversation(&mut tx, actor, conversation, "conversation.send").await?;
        super::tasks::guard(&mut tx, actor, turn).await?;
        let response = matches!(part, MessagePart::Response);
        let deleted = sqlx::query_scalar!(
            "
            UPDATE chat_turns
            SET message=CASE WHEN $3 THEN message ELSE '' END,
                message_deleted=message_deleted OR NOT $3,
                response=CASE WHEN $3 AND response IS NOT NULL THEN '' ELSE response END,
                response_deleted=response_deleted OR $3
            WHERE id=$1 AND conversation_id=$2 AND status IN ('complete','failed','cancelled')
            RETURNING id
            ",
            turn,
            agent.0,
            response,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if deleted.is_none() {
            return Err(IdentityError::Conflict);
        }
        sqlx::query!(
            "DELETE FROM turn_attachments WHERE turn_id=$1 AND inbound<>$2",
            turn,
            response,
        )
        .execute(&mut *tx)
        .await?;
        if response {
            sqlx::query!(
                r#"
                UPDATE turn_events
                SET payload=CASE WHEN payload->>'kind'='done' THEN jsonb_set(payload, '{response}', '""') ELSE jsonb_set(payload, '{text}', '""') END
                WHERE turn_id=$1 AND payload->>'kind' IN ('done','text_delta')
                "#,
                turn,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

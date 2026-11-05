//! At most one per turn and never from a broadcast's turn, so broadcasts cannot answer each other
//! in a loop. Deliveries carry the authority of the person behind them.
use super::{SendMessage, Work, access, rooms::Delivery};
use crate::{
    agents::{AgentId, kubernetes::Error, runtime::RuntimeActor},
    identity::{
        Actor, IdentityError,
        audit::{Event, record},
    },
};
use cubebot_protocol::runtime::Broadcast;
use serde::Serialize;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Serialize, utoipa::ToSchema)]
pub struct BroadcastMessage {
    pub id: Uuid,
    pub sequence: i64,
    /// The sending person's login, or the agent's name.
    pub author: String,
    /// The person who sent it, or on whose behalf the agent did.
    pub account: Uuid,
    /// The agent that sent it; `None` when a person did.
    pub agent: Option<Uuid>,
    pub message: String,
    pub at: String,
    pub deliveries: Vec<Delivery>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct BroadcastFeed {
    pub messages: Vec<BroadcastMessage>,
    /// The caller's read cursor, 0 until set.
    pub read_sequence: i64,
}

struct Row {
    id: Uuid,
    sequence: i64,
    author: String,
    account: Uuid,
    agent: Option<Uuid>,
    message: String,
    at: String,
}

impl From<Row> for BroadcastMessage {
    fn from(row: Row) -> Self {
        Self {
            id: row.id,
            sequence: row.sequence,
            author: row.author,
            account: row.account,
            agent: row.agent,
            message: row.message,
            at: row.at,
            deliveries: Vec::new(),
        }
    }
}

impl Work {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn broadcasts(
        &self,
        actor: &Actor,
        after: i64,
    ) -> Result<BroadcastFeed, IdentityError> {
        let mut tx = self.pool.begin().await?;
        crate::identity::administrator(&mut *tx, actor.id).await?;
        let mut messages: Vec<BroadcastMessage> = sqlx::query_as!(
            Row,
            r#"
            SELECT id AS "id!", sequence AS "sequence!", author AS "author!", account AS "account!",
                agent, message AS "message!", at AS "at!"
            FROM (
                SELECT b.id, b.sequence, COALESCE(g.name, h.login) AS author, b.actor_id AS account,
                    b.agent_id AS agent, b.message, (to_jsonb(b.created_at) #>> '{}') AS at
                FROM broadcasts b
                JOIN human_accounts h ON h.id = b.actor_id
                LEFT JOIN agents g ON g.id = b.agent_id
                WHERE b.sequence > $1
                ORDER BY b.sequence DESC
                LIMIT 100
            ) newest
            ORDER BY sequence
            "#,
            after,
        )
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(BroadcastMessage::from)
        .collect();
        attach(&mut tx, &mut messages).await?;
        let read_sequence = sqlx::query_scalar!(
            "
            SELECT sequence
            FROM read_cursors
            WHERE account_id=$1 AND scope_kind='broadcast' AND scope_id=$2
            ",
            actor.id,
            Uuid::nil(),
        )
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(0);
        tx.commit().await?;
        Ok(BroadcastFeed {
            messages,
            read_sequence,
        })
    }

    pub async fn broadcast_sequence(&self, actor: &Actor) -> Result<Option<i64>, IdentityError> {
        if !actor.administrator {
            return Ok(None);
        }
        Ok(
            sqlx::query_scalar!(r#"SELECT COALESCE(max(sequence), 0) AS "last!" FROM broadcasts"#)
                .fetch_one(&self.pool)
                .await
                .map(Some)?,
        )
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn broadcast(
        &self,
        actor: &Actor,
        input: &SendMessage,
    ) -> Result<BroadcastMessage, IdentityError> {
        super::validate(input)?;
        let mut tx = self.pool.begin().await?;
        access::lock_actor(&mut tx, actor).await?;
        crate::identity::administrator(&mut *tx, actor.id).await?;
        let id = match previous(&mut tx, actor, input).await? {
            Some(id) => id,
            None => {
                let id = insert(&mut tx, actor.id, None, input.request_id, &input.message).await?;
                let agents = deliver(&mut tx, id, None).await?;
                let event = Event::by(actor.id, "broadcast.send").on(id);
                record(&mut *tx, &event.detail(json!({ "agents": agents }))).await?;
                id
            }
        };
        let message = one(&mut tx, id).await?;
        tx.commit().await?;
        Ok(message)
    }

    #[tracing::instrument(skip_all, fields(agent = %actor.agent.0, turn = %turn))]
    pub(crate) async fn agent_broadcast(
        &self,
        actor: &RuntimeActor,
        turn: Uuid,
        input: &Broadcast,
    ) -> Result<Result<u32, String>, Error> {
        let message = sendable(&input.message).ok_or(Error::InvalidConfiguration)?;
        let mut tx = self.pool.begin().await?;
        let enabled = sqlx::query_scalar!(
            "SELECT broadcast FROM agents WHERE id=$1 FOR UPDATE",
            actor.agent.0,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::Conflict)?;
        if let Some(agents) = resent(&mut tx, actor, input.call).await? {
            return Ok(Ok(agents));
        }
        let person = match sender(&mut tx, actor, turn, enabled).await? {
            Ok(person) => person,
            Err(refusal) => return Ok(Err(refusal.into())),
        };
        let id = insert(
            &mut tx,
            person,
            Some((actor.agent, turn)),
            input.call,
            message,
        )
        .await
        .map_err(|_| Error::Conflict)?;
        let agents = deliver(&mut tx, id, Some(actor.agent))
            .await
            .map_err(|_| Error::Conflict)?;
        let event = Event::by_agent(actor.agent.0, "broadcast.send").on(id);
        record(&mut *tx, &event.detail(json!({ "agents": agents }))).await?;
        tx.commit().await?;
        Ok(Ok(agents))
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn read_broadcasts(
        &self,
        actor: &Actor,
        sequence: i64,
    ) -> Result<super::ReadCursor, IdentityError> {
        let mut tx = self.pool.begin().await?;
        crate::identity::administrator(&mut *tx, actor.id).await?;
        let cursor =
            super::cursors::store(&mut tx, actor, "broadcast", Uuid::nil(), sequence).await?;
        tx.commit().await?;
        Ok(cursor)
    }
}

async fn previous(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    input: &SendMessage,
) -> Result<Option<Uuid>, IdentityError> {
    let previous = sqlx::query!(
        "SELECT id, actor_id, message FROM broadcasts WHERE request_id=$1",
        input.request_id,
    )
    .fetch_optional(&mut **tx)
    .await?;
    match previous {
        Some(row) if row.actor_id == actor.id && row.message == input.message => Ok(Some(row.id)),
        Some(_) => Err(IdentityError::Conflict),
        None => Ok(None),
    }
}

fn sendable(message: &str) -> Option<&str> {
    let trimmed = message.trim();
    (!trimmed.is_empty() && message.len() <= 16384 && !message.contains('\0')).then_some(trimmed)
}

async fn resent(
    tx: &mut Transaction<'_, Postgres>,
    actor: &RuntimeActor,
    call: Uuid,
) -> Result<Option<u32>, sqlx::Error> {
    let agents = sqlx::query_scalar!(
        r#"
        SELECT count(t.id) AS "agents!"
        FROM broadcasts b
        LEFT JOIN chat_turns t ON t.broadcast_id = b.id
        WHERE b.request_id=$1 AND b.agent_id=$2
        GROUP BY b.id
        "#,
        call,
        actor.agent.0,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(agents.map(|agents| agents as u32))
}

async fn sender(
    tx: &mut Transaction<'_, Postgres>,
    actor: &RuntimeActor,
    turn: Uuid,
    enabled: bool,
) -> Result<Result<Uuid, &'static str>, Error> {
    let current = sqlx::query!(
        "
        SELECT actor_id, broadcast_id,
            EXISTS(SELECT 1 FROM broadcasts b WHERE b.turn_id=t.id) AS already
        FROM chat_turns t
        WHERE id=$1 AND conversation_id=$2 AND status='running' AND attempt_id=$3
        ",
        turn,
        actor.agent.0,
        actor.attempt.0,
    )
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(Error::Conflict)?;
    Ok(
        match refusal(enabled, current.broadcast_id, current.already) {
            Some(refusal) => Err(refusal),
            None => Ok(current.actor_id),
        },
    )
}

fn refusal(
    enabled: bool,
    from_broadcast: Option<Uuid>,
    already: Option<bool>,
) -> Option<&'static str> {
    if !enabled {
        Some("broadcasting is not enabled for you; a person can turn it on in your settings")
    } else if from_broadcast.is_some() {
        Some(
            "this turn answers a broadcast, and broadcasts cannot answer broadcasts; reply in your own conversation instead",
        )
    } else if already == Some(true) {
        Some("this turn already broadcast once")
    } else {
        None
    }
}

async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    account: Uuid,
    agent: Option<(AgentId, Uuid)>,
    request: Uuid,
    message: &str,
) -> Result<Uuid, IdentityError> {
    Ok(sqlx::query_scalar!(
        "
        INSERT INTO broadcasts (actor_id, agent_id, turn_id, request_id, message)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        ",
        account,
        agent.map(|(agent, _)| agent.0),
        agent.map(|(_, turn)| turn),
        request,
        message,
    )
    .fetch_one(&mut **tx)
    .await?)
}

async fn deliver(
    tx: &mut Transaction<'_, Postgres>,
    broadcast: Uuid,
    sender: Option<AgentId>,
) -> Result<u32, IdentityError> {
    let recipients = sqlx::query_scalar!(
        "
        SELECT id
        FROM agents
        WHERE desired_state<>'retired' AND id IS DISTINCT FROM $1
        ORDER BY id
        FOR NO KEY UPDATE
        ",
        sender.map(|sender| sender.0),
    )
    .fetch_all(&mut **tx)
    .await?;
    for agent in &recipients {
        super::wake(tx, AgentId(*agent)).await?;
    }
    sqlx::query!(
        r#"
        INSERT INTO chat_turns (conversation_id, actor_id, request_id, message, broadcast_id)
        SELECT r, b.actor_id, gen_random_uuid(),
            format('Broadcast received from %s. Every agent already has it, so do not forward or rebroadcast it; respond only for yourself:%s%s',
                COALESCE(g.name || ' (an agent)', h.login), E'\n\n', b.message),
            b.id
        FROM broadcasts b
        JOIN human_accounts h ON h.id = b.actor_id
        LEFT JOIN agents g ON g.id = b.agent_id
        CROSS JOIN unnest($2::uuid[]) r
        WHERE b.id=$1
        ON CONFLICT (broadcast_id, conversation_id) WHERE broadcast_id IS NOT NULL DO NOTHING
        "#,
        broadcast,
        &recipients,
    )
    .execute(&mut **tx)
    .await?;
    Ok(recipients.len() as u32)
}

async fn one(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<BroadcastMessage, IdentityError> {
    let row = sqlx::query_as!(
        Row,
        r#"
        SELECT b.id, b.sequence, COALESCE(g.name, h.login) AS "author!", b.actor_id AS account,
            b.agent_id AS agent, b.message, (to_jsonb(b.created_at) #>> '{}') AS "at!"
        FROM broadcasts b
        JOIN human_accounts h ON h.id = b.actor_id
        LEFT JOIN agents g ON g.id = b.agent_id
        WHERE b.id=$1
        "#,
        id,
    )
    .fetch_one(&mut **tx)
    .await?;
    let mut messages = [BroadcastMessage::from(row)];
    attach(tx, &mut messages).await?;
    let [message] = messages;
    Ok(message)
}

async fn attach(
    tx: &mut Transaction<'_, Postgres>,
    messages: &mut [BroadcastMessage],
) -> Result<(), IdentityError> {
    let ids: Vec<Uuid> = messages.iter().map(|message| message.id).collect();
    // `room_message_id` is the broadcast here: Delivery's grouping key, never serialized.
    let deliveries = sqlx::query!(
        r#"
        SELECT t.id AS turn, t.broadcast_id AS "room_message_id!", t.conversation_id AS agent_id,
            a.name AS agent, t.status, t.response, t.error
        FROM chat_turns t
        JOIN agents a ON a.id=t.conversation_id
        WHERE t.broadcast_id=ANY($1)
        ORDER BY a.name, t.sequence
        "#,
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    let turns: Vec<Uuid> = deliveries.iter().map(|delivery| delivery.turn).collect();
    let mut attachments = super::terminal::attachments(tx, &turns).await?;
    for delivery in deliveries {
        let delivery = Delivery {
            attachments: attachments.remove(&delivery.turn).unwrap_or_default(),
            terminal: Vec::new(),
            partial: None,
            turn: delivery.turn,
            room_message_id: delivery.room_message_id,
            agent_id: delivery.agent_id,
            agent: delivery.agent,
            status: delivery.status,
            response: delivery.response,
            error: delivery.error,
        };
        if let Some(message) = messages
            .iter_mut()
            .find(|message| message.id == delivery.room_message_id)
        {
            message.deliveries.push(delivery);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::refusal;
    use uuid::Uuid;

    #[test]
    fn a_broadcast_carries_trimmed_text_of_at_most_16_kib() {
        assert_eq!(super::sendable("  hi all \n"), Some("hi all"));
        assert_eq!(super::sendable("   "), None);
        assert_eq!(super::sendable("a\0b"), None);
        assert_eq!(super::sendable(&"x".repeat(16385)), None);
    }

    #[test]
    fn only_an_enabled_agent_broadcasts_once_and_never_in_reply_to_a_broadcast() {
        assert_eq!(refusal(true, None, Some(false)), None);
        assert!(
            refusal(false, None, Some(false))
                .unwrap()
                .contains("not enabled")
        );
        assert!(
            refusal(true, Some(Uuid::nil()), Some(false))
                .unwrap()
                .contains("cannot answer")
        );
        assert!(refusal(true, None, Some(true)).unwrap().contains("already"));
    }
}

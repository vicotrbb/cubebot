//! Rooms (ADRs 0014, 0015). A message mentioning members delivers one turn to each, without a
//! model call; any other message waits for the coordinator. Deliveries are turns in each
//! recipient's permanent conversation, unique per message and recipient.
use super::{SendMessage, Work, access};
use crate::identity::{Actor, IdentityError};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateRoom {
    pub request_id: Uuid,
    pub name: String,
    pub agents: Vec<Uuid>,
}

/// The room's whole new member set; putting the same set again changes nothing.
#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RoomMembers {
    pub agents: Vec<Uuid>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Room {
    pub id: Uuid,
    pub name: String,
    pub members: Vec<Member>,
    /// The viewer may change the members.
    pub manage: bool,
}

#[derive(Clone, Serialize, utoipa::ToSchema)]
pub struct Member {
    pub id: Uuid,
    pub name: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct RoomMessage {
    pub id: Uuid,
    pub sequence: i64,
    pub author: String,
    pub message: String,
    /// `routing` until the coordinator settles it as `routed`, `continued`, `clarify`,
    /// `no_action` or `failed`; mentioned messages are `routed` at once.
    pub routing: String,
    /// The coordinator's question, the continued task's agent, or the failure reason.
    pub note: Option<String>,
    pub deliveries: Vec<Delivery>,
}

/// A room message's own columns; its deliveries are attached after.
struct MessageRow {
    id: Uuid,
    sequence: i64,
    author: String,
    message: String,
    routing: String,
    note: Option<String>,
}

impl From<MessageRow> for RoomMessage {
    fn from(row: MessageRow) -> Self {
        Self {
            id: row.id,
            sequence: row.sequence,
            author: row.author,
            message: row.message,
            routing: row.routing,
            note: row.note,
            deliveries: Vec::new(),
        }
    }
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Delivery {
    pub turn: Uuid,
    #[serde(skip)]
    pub room_message_id: Uuid,
    pub agent_id: Uuid,
    pub agent: String,
    pub status: String,
    pub response: Option<String>,
    pub error: Option<String>,
}

impl RoomMessage {
    /// Nothing about the message can change any more.
    pub fn settled(&self) -> bool {
        self.routing != "routing"
            && self.deliveries.iter().all(|delivery| {
                matches!(
                    delivery.status.as_str(),
                    "complete" | "failed" | "cancelled"
                )
            })
    }
}

impl Work {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn create_room(
        &self,
        actor: &Actor,
        input: &CreateRoom,
    ) -> Result<Room, IdentityError> {
        let name = input.name.trim();
        if input.request_id.is_nil() || name.is_empty() || name.len() > 128 || !sized(&input.agents)
        {
            return Err(IdentityError::InvalidInput);
        }
        let mut tx = self.pool.begin().await?;
        access::lock_actor(&mut tx, actor).await?;
        administrator(&mut tx, actor).await?;
        let previous = sqlx::query_scalar!(
            "
            SELECT id
            FROM rooms
            WHERE created_by=$1 AND request_id=$2
            ",
            actor.id,
            input.request_id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let id = match previous {
            Some(id) => id,
            None => insert_room(&mut tx, actor, name, input).await?,
        };
        let room = room(&mut tx, actor, id).await?;
        tx.commit().await?;
        Ok(room)
    }

    /// Rooms the person may read.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn rooms(&self, actor: &Actor) -> Result<Vec<Room>, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let ids = sqlx::query_scalar!(
            "
            SELECT r.id
            FROM rooms r
            JOIN human_accounts a ON a.id=$1 AND a.active
            WHERE a.role='administrator'
                OR EXISTS(SELECT 1 FROM effective_human_grants g WHERE g.actor_id=a.id AND g.scope_kind='room' AND g.scope_id=r.id AND g.capability='conversation.read')
            ORDER BY r.name, r.id
            ",
            actor.id,
        )
        .fetch_all(&mut *tx)
        .await?;
        let mut rooms = Vec::with_capacity(ids.len());
        for id in ids {
            rooms.push(room(&mut tx, actor, id).await?);
        }
        tx.commit().await?;
        Ok(rooms)
    }

    /// Administrators and `room.manage` holders replace the member set. Removed members keep
    /// the work already delivered to them.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, room = %room))]
    pub async fn set_room_members(
        &self,
        actor: &Actor,
        room: Uuid,
        input: &RoomMembers,
    ) -> Result<Room, IdentityError> {
        if !sized(&input.agents) {
            return Err(IdentityError::InvalidInput);
        }
        let mut tx = self.pool.begin().await?;
        access::lock_actor(&mut tx, actor).await?;
        authorize(&mut tx, actor, room, "conversation.read").await?;
        if !allowed(&mut tx, actor, room, "room.manage").await? {
            return Err(IdentityError::Forbidden);
        }
        lock(&mut tx, room).await?;
        sqlx::query!(
            "
            DELETE FROM room_members
            WHERE room_id=$1 AND agent_id<>ALL($2)
            ",
            room,
            &input.agents,
        )
        .execute(&mut *tx)
        .await?;
        add_members(&mut tx, room, &input.agents).await?;
        let room = self::room(&mut tx, actor, room).await?;
        tx.commit().await?;
        Ok(room)
    }

    pub async fn room_messages(
        &self,
        actor: &Actor,
        room: Uuid,
        after: i64,
    ) -> Result<Vec<RoomMessage>, IdentityError> {
        self.room_feed(actor, room, after, &[]).await
    }

    /// Messages after the cursor plus the touched ones whose last state the caller holds.
    /// Rechecks access on every call.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, room = %room))]
    pub async fn room_feed(
        &self,
        actor: &Actor,
        room: Uuid,
        after: i64,
        touched: &[Uuid],
    ) -> Result<Vec<RoomMessage>, IdentityError> {
        let mut tx = self.pool.begin().await?;
        authorize(&mut tx, actor, room, "conversation.read").await?;
        let mut messages: Vec<RoomMessage> = sqlx::query_as!(
            MessageRow,
            r#"
            SELECT id AS "id!", sequence AS "sequence!", author AS "author!", message AS "message!", routing AS "routing!", note
            FROM (
                (
                    SELECT m.id, m.sequence, a.login AS author, m.message, m.routing, m.note
                    FROM room_messages m
                    JOIN human_accounts a ON a.id=m.actor_id
                    WHERE m.room_id=$1 AND m.sequence>$2
                    ORDER BY m.sequence
                    LIMIT 100
                )
                UNION
                (
                    SELECT m.id, m.sequence, a.login AS author, m.message, m.routing, m.note
                    FROM room_messages m
                    JOIN human_accounts a ON a.id=m.actor_id
                    WHERE m.room_id=$1 AND m.id=ANY($3)
                )
            ) messages
            ORDER BY sequence
            "#,
            room,
            after,
            touched,
        )
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(RoomMessage::from)
        .collect();
        attach(&mut tx, &mut messages).await?;
        tx.commit().await?;
        Ok(messages)
    }

    /// Mentions of members deliver at once; otherwise the coordinator routes it later.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, room = %room))]
    pub async fn post_room_message(
        &self,
        actor: &Actor,
        room: Uuid,
        input: &SendMessage,
    ) -> Result<RoomMessage, IdentityError> {
        super::validate(input)?;
        let mut tx = self.pool.begin().await?;
        access::lock_actor(&mut tx, actor).await?;
        authorize(&mut tx, actor, room, "conversation.send").await?;
        // Serializes retries of one request, so both see a single message.
        lock(&mut tx, room).await?;
        let id = match previous(&mut tx, actor, room, input).await? {
            Some(id) => id,
            None => post(&mut tx, actor, room, input).await?,
        };
        let mut messages: Vec<RoomMessage> = sqlx::query_as!(
            MessageRow,
            r#"
            SELECT m.id, m.sequence, a.login AS author, m.message, m.routing, m.note
            FROM room_messages m
            JOIN human_accounts a ON a.id=m.actor_id
            WHERE m.id=$1
            "#,
            id,
        )
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(RoomMessage::from)
        .collect();
        attach(&mut tx, &mut messages).await?;
        tx.commit().await?;
        messages.pop().ok_or(IdentityError::NotFound)
    }
}

async fn lock(tx: &mut Transaction<'_, Postgres>, room: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "
        SELECT 1 AS locked
        FROM rooms
        WHERE id=$1
        FOR UPDATE
        ",
        room,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(())
}

/// The message this request already posted; a different message under it is a conflict.
async fn previous(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    room: Uuid,
    input: &SendMessage,
) -> Result<Option<Uuid>, IdentityError> {
    let previous = sqlx::query!(
        "
        SELECT id, room_id, message
        FROM room_messages
        WHERE actor_id=$1 AND request_id=$2
        ",
        actor.id,
        input.request_id,
    )
    .fetch_optional(&mut **tx)
    .await?;
    match previous {
        Some(previous) if previous.room_id == room && previous.message == input.message => {
            Ok(Some(previous.id))
        }
        Some(_) => Err(IdentityError::Conflict),
        None => Ok(None),
    }
}

async fn insert_room(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    name: &str,
    input: &CreateRoom,
) -> Result<Uuid, IdentityError> {
    let id = sqlx::query_scalar!(
        "
        INSERT INTO rooms (name, created_by, request_id)
        VALUES ($1, $2, $3)
        RETURNING id
        ",
        name,
        actor.id,
        input.request_id,
    )
    .fetch_one(&mut **tx)
    .await?;
    add_members(tx, id, &input.agents).await?;
    Ok(id)
}

fn sized(agents: &[Uuid]) -> bool {
    (1..=32).contains(&agents.len())
}

/// Adds the agents not yet members; every one must exist and not be retired.
async fn add_members(
    tx: &mut Transaction<'_, Postgres>,
    room: Uuid,
    agents: &[Uuid],
) -> Result<(), IdentityError> {
    let eligible = sqlx::query_scalar!(
        r#"
        SELECT count(*) AS "eligible!"
        FROM agents
        WHERE id=ANY($1) AND desired_state<>'retired'
        "#,
        agents,
    )
    .fetch_one(&mut **tx)
    .await?;
    let mut distinct = agents.to_vec();
    distinct.sort();
    distinct.dedup();
    if eligible != distinct.len() as i64 {
        return Err(IdentityError::InvalidInput);
    }
    sqlx::query!(
        "
        INSERT INTO room_members (room_id, agent_id)
        SELECT $1, unnest($2::uuid[])
        ON CONFLICT DO NOTHING
        ",
        room,
        &distinct,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn room(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    id: Uuid,
) -> Result<Room, IdentityError> {
    let row = sqlx::query!(
        "
        SELECT id, name
        FROM rooms
        WHERE id=$1
        ",
        id,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(Room {
        id: row.id,
        name: row.name,
        members: members(tx, id).await?,
        manage: allowed(tx, actor, id, "room.manage").await?,
    })
}

/// Earlier room messages and the agents' replies to them, for a delivery's context. They are
/// added when the turn is claimed, not stored with it, so the conversation keeps only the request.
pub(super) async fn context(
    tx: &mut Transaction<'_, Postgres>,
    message: Uuid,
) -> Result<String, sqlx::Error> {
    let lines = sqlx::query_scalar!(
        r#"
        SELECT format('%s: %s%s',a.login,left(m.message,1000),COALESCE((SELECT string_agg(format(E'\n  %s replied: %s',g.name,left(t.response,1000)),'' ORDER BY t.sequence) FROM chat_turns t JOIN agents g ON g.id=t.conversation_id WHERE t.room_message_id=m.id AND t.status='complete'),'')) AS "line!"
        FROM room_messages m
        JOIN human_accounts a ON a.id=m.actor_id
        JOIN room_messages c ON c.id=$1
        WHERE m.room_id=c.room_id AND m.sequence<c.sequence
        ORDER BY m.sequence DESC
        LIMIT 10
        "#,
        message,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(lines.into_iter().rev().collect::<Vec<_>>().join("\n"))
}

/// Current members that can still take work.
pub(super) async fn members(
    tx: &mut Transaction<'_, Postgres>,
    room: Uuid,
) -> Result<Vec<Member>, sqlx::Error> {
    sqlx::query_as!(
        Member,
        "
        SELECT a.id, a.name
        FROM room_members m
        JOIN agents a ON a.id=m.agent_id
        WHERE m.room_id=$1 AND a.desired_state<>'retired'
        ORDER BY a.name, a.id
        ",
        room,
    )
    .fetch_all(&mut **tx)
    .await
}

async fn post(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    room: Uuid,
    input: &SendMessage,
) -> Result<Uuid, IdentityError> {
    let members = members(tx, room).await?;
    let mentioned = mentioned(&input.message, &members);
    let routing = if mentioned.is_empty() {
        "routing"
    } else {
        "routed"
    };
    let id = sqlx::query_scalar!(
        "
        INSERT INTO room_messages (room_id, actor_id, request_id, message, routing)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        ",
        room,
        actor.id,
        input.request_id,
        &input.message,
        routing,
    )
    .fetch_one(&mut **tx)
    .await?;
    for agent in mentioned {
        deliver(tx, id, agent).await?;
    }
    Ok(id)
}

/// One turn for `agent` from the room message, at most once per message and agent.
pub(super) async fn deliver(
    tx: &mut Transaction<'_, Postgres>,
    message: Uuid,
    agent: Uuid,
) -> Result<(), IdentityError> {
    super::wake(tx, crate::agents::AgentId(agent)).await?;
    sqlx::query!(
        r#"
        INSERT INTO chat_turns (conversation_id, actor_id, request_id, message, room_message_id)
        SELECT $2, m.actor_id, gen_random_uuid(), format('In room %s, %s wrote:%s%s',r.name,a.login,E'\n\n',m.message), m.id
        FROM room_messages m
        JOIN rooms r ON r.id=m.room_id
        JOIN human_accounts a ON a.id=m.actor_id
        WHERE m.id=$1
        ON CONFLICT (room_message_id,conversation_id) WHERE room_message_id IS NOT NULL DO NOTHING
        "#,
        message,
        agent,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Members named as `@name`, case-insensitively, not inside a word or an address.
pub(super) fn mentioned(message: &str, members: &[Member]) -> Vec<Uuid> {
    let text = message.to_lowercase();
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    members
        .iter()
        .filter(|member| {
            let tag = format!("@{}", member.name.to_lowercase());
            text.match_indices(&tag).any(|(at, _)| {
                !text[..at].ends_with(word) && !text[at + tag.len()..].starts_with(word)
            })
        })
        .map(|member| member.id)
        .collect()
}

pub(super) async fn authorize(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    room: Uuid,
    capability: &str,
) -> Result<(), IdentityError> {
    allowed(tx, actor, room, capability)
        .await?
        .then_some(())
        .ok_or(IdentityError::NotFound)
}

async fn allowed(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    room: Uuid,
    capability: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM rooms r
            JOIN human_accounts a ON a.id=$1 AND a.active
            WHERE r.id=$2
                AND (a.role='administrator' OR EXISTS(SELECT 1 FROM effective_human_grants g WHERE g.actor_id=a.id AND g.scope_kind='room' AND g.scope_id=r.id AND g.capability=$3))
        ) AS "allowed!"
        "#,
        actor.id,
        room,
        capability,
    )
    .fetch_one(&mut **tx)
    .await
}

async fn administrator(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
) -> Result<(), IdentityError> {
    let admin = sqlx::query_scalar!(
        r#"
        SELECT EXISTS(SELECT 1 FROM human_accounts WHERE id=$1 AND active AND role='administrator') AS "admin!"
        "#,
        actor.id,
    )
    .fetch_one(&mut **tx)
    .await?;
    admin.then_some(()).ok_or(IdentityError::Forbidden)
}

async fn attach(
    tx: &mut Transaction<'_, Postgres>,
    messages: &mut [RoomMessage],
) -> Result<(), IdentityError> {
    let ids: Vec<Uuid> = messages.iter().map(|message| message.id).collect();
    let deliveries = sqlx::query_as!(
        Delivery,
        r#"
        SELECT t.id AS turn, t.room_message_id AS "room_message_id!", t.conversation_id AS agent_id, a.name AS agent, t.status, t.response, t.error
        FROM chat_turns t
        JOIN agents a ON a.id=t.conversation_id
        WHERE t.room_message_id=ANY($1)
        ORDER BY t.sequence
        "#,
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    for delivery in deliveries {
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
    use super::*;

    #[test]
    fn mentions_match_whole_member_names_once_each() {
        let member = |name: &str| Member {
            id: Uuid::new_v4(),
            name: name.into(),
        };
        let members = [member("Ada"), member("Grace Hopper"), member("Bob")];
        let ids = |text| mentioned(text, &members);
        assert_eq!(ids("@ada and @ADA, please"), vec![members[0].id]);
        assert_eq!(
            ids("@Grace Hopper with @Bob."),
            vec![members[1].id, members[2].id]
        );
        assert!(ids("mail bob@ada.example or @adam or @bobby").is_empty());
        assert!(ids("no mentions at all").is_empty());
    }
}

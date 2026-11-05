use super::{SendMessage, Work, access};
use crate::identity::{Actor, IdentityError};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use std::collections::HashMap;
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
    /// The newest message's sequence, 0 without messages.
    pub last_sequence: i64,
    /// The viewer's read cursor, 0 until set.
    pub read_sequence: i64,
}

/// A room's newest message, for the installation stream.
#[derive(Serialize, utoipa::ToSchema)]
pub struct RoomSequence {
    pub id: Uuid,
    pub last_sequence: i64,
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
    /// In a task's room: the task orchestrator wrote this.
    pub orchestrator: bool,
    /// In a task's room: the agent that wrote this.
    pub author_agent_id: Option<Uuid>,
    pub message: String,
    /// `routing` until the coordinator settles it as `routed`, `continued` or `failed`;
    /// mentioned messages are `routed` at once. Messages from before 2026-11-03 may also be
    /// `clarify` or `no_action`.
    pub routing: String,
    /// The coordinator's question, the continued task's agent, or the failure reason.
    pub note: Option<String>,
    pub deliveries: Vec<Delivery>,
}

struct MessageRow {
    id: Uuid,
    sequence: i64,
    author: String,
    orchestrator: bool,
    author_agent_id: Option<Uuid>,
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
            orchestrator: row.orchestrator,
            author_agent_id: row.author_agent_id,
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
    /// Files the agent sent with this reply.
    pub attachments: Vec<super::terminal::ReplyAttachment>,
    /// What the agent has done so far, tool by tool.
    pub terminal: Vec<super::terminal::Progress>,
    /// The reply as far as it has been written, while there is no `response`.
    pub partial: Option<String>,
}

impl RoomMessage {
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
        crate::identity::administrator(&mut *tx, actor.id).await?;
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

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn rooms(&self, actor: &Actor) -> Result<Vec<Room>, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let ids = sqlx::query_scalar!(
            "
            SELECT r.id
            FROM rooms r
            JOIN human_accounts a ON a.id=$1 AND a.active
            WHERE NOT r.task AND (a.role='administrator'
                OR EXISTS(SELECT 1 FROM effective_human_grants g WHERE g.actor_id=a.id AND g.scope_kind='room' AND g.scope_id=r.id AND g.capability='conversation.read'))
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

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn room_sequences(&self, actor: &Actor) -> Result<Vec<RoomSequence>, IdentityError> {
        Ok(sqlx::query_as!(
            RoomSequence,
            r#"
            SELECT r.id, COALESCE(max(m.sequence),0) AS "last_sequence!"
            FROM rooms r
            JOIN human_accounts a ON a.id=$1 AND a.active
            LEFT JOIN room_messages m ON m.room_id=r.id
            WHERE NOT r.task AND (a.role='administrator'
                OR EXISTS(SELECT 1 FROM effective_human_grants g WHERE g.actor_id=a.id AND g.scope_kind='room' AND g.scope_id=r.id AND g.capability='conversation.read'))
            GROUP BY r.id
            ORDER BY r.id
            "#,
            actor.id,
        )
        .fetch_all(&self.pool)
        .await?)
    }

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
        let task = sqlx::query_scalar!("SELECT task FROM rooms WHERE id=$1", room)
            .fetch_one(&mut *tx)
            .await?;
        if task {
            return Err(IdentityError::Forbidden);
        }
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
            SELECT id AS "id!", sequence AS "sequence!", author AS "author!", orchestrator AS "orchestrator!", author_agent_id, message AS "message!", routing AS "routing!", note
            FROM (
                (
                    SELECT m.id, m.sequence, room_message_author(m.orchestrator,m.author_agent_id,m.actor_id) AS author, m.orchestrator, m.author_agent_id, m.message, m.routing, m.note
                    FROM room_messages m
                    WHERE m.room_id=$1 AND m.sequence>$2
                    ORDER BY m.sequence
                    LIMIT 100
                )
                UNION
                (
                    SELECT m.id, m.sequence, room_message_author(m.orchestrator,m.author_agent_id,m.actor_id) AS author, m.orchestrator, m.author_agent_id, m.message, m.routing, m.note
                    FROM room_messages m
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
            SELECT m.id, m.sequence, room_message_author(m.orchestrator,m.author_agent_id,m.actor_id) AS "author!", m.orchestrator, m.author_agent_id, m.message, m.routing, m.note
            FROM room_messages m
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
        r#"
        SELECT r.id, r.name,
            COALESCE((SELECT max(m.sequence) FROM room_messages m WHERE m.room_id=r.id),0) AS "last_sequence!",
            COALESCE((SELECT c.sequence FROM read_cursors c WHERE c.account_id=$2 AND c.scope_kind='room' AND c.scope_id=r.id),0) AS "read_sequence!"
        FROM rooms r
        WHERE r.id=$1
        "#,
        id,
        actor.id,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(Room {
        id: row.id,
        name: row.name,
        members: members(tx, id).await?,
        manage: allowed(tx, actor, id, "room.manage").await?,
        last_sequence: row.last_sequence,
        read_sequence: row.read_sequence,
    })
}

pub(super) async fn delivery_context(
    tx: &mut Transaction<'_, Postgres>,
    message: Uuid,
    limit: i64,
) -> Result<String, sqlx::Error> {
    let room = sqlx::query!(
        "SELECT room_id, sequence FROM room_messages WHERE id=$1",
        message,
    )
    .fetch_one(&mut **tx)
    .await?;
    lines(tx, room.room_id, room.sequence, limit).await
}

pub(super) async fn context(
    tx: &mut Transaction<'_, Postgres>,
    room: Uuid,
    limit: i64,
) -> Result<String, sqlx::Error> {
    lines(tx, room, i64::MAX, limit).await
}

async fn lines(
    tx: &mut Transaction<'_, Postgres>,
    room: Uuid,
    before: i64,
    limit: i64,
) -> Result<String, sqlx::Error> {
    let lines = sqlx::query_scalar!(
        r#"
        SELECT format('%s: %s%s',room_message_author(m.orchestrator,m.author_agent_id,m.actor_id),left(m.message,1000),COALESCE((SELECT string_agg(format(E'\n  %s replied: %s',g.name,left(t.response,1000)),'' ORDER BY t.sequence) FROM chat_turns t JOIN agents g ON g.id=t.conversation_id WHERE t.room_message_id=m.id AND t.status='complete'),'')) AS "line!"
        FROM room_messages m
        WHERE m.room_id=$1 AND m.sequence<$2
        ORDER BY m.sequence DESC
        LIMIT $3
        "#,
        room,
        before,
        limit,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(lines.into_iter().rev().collect::<Vec<_>>().join("\n"))
}

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
    let mentioned = match super::tasks::person_wrote(tx, room, &input.message).await? {
        Some(mentioned) => mentioned,
        None => mentioned(&input.message, &members(tx, room).await?),
    };
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

pub(super) async fn deliver(
    tx: &mut Transaction<'_, Postgres>,
    message: Uuid,
    agent: Uuid,
) -> Result<(), IdentityError> {
    super::wake(tx, crate::agents::AgentId(agent)).await?;
    sqlx::query!(
        r#"
        INSERT INTO chat_turns (conversation_id, actor_id, request_id, message, room_message_id, in_task)
        SELECT $2, COALESCE((SELECT k.created_by FROM tasks k WHERE k.room_id=m.room_id), m.actor_id), gen_random_uuid(), format('In room %s, %s wrote:%s%s',r.name,a.login,E'\n\n',m.message), m.id, r.task
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
                AND (a.role='administrator'
                    OR EXISTS(SELECT 1 FROM tasks t WHERE t.room_id=r.id AND t.created_by=a.id AND $3<>'room.manage')
                    OR EXISTS(SELECT 1 FROM effective_human_grants g WHERE g.actor_id=a.id AND g.scope_kind='room' AND g.scope_id=r.id AND g.capability=$3))
        ) AS "allowed!"
        "#,
        actor.id,
        room,
        capability,
    )
    .fetch_one(&mut **tx)
    .await
}

async fn attach(
    tx: &mut Transaction<'_, Postgres>,
    messages: &mut [RoomMessage],
) -> Result<(), IdentityError> {
    let ids: Vec<Uuid> = messages.iter().map(|message| message.id).collect();
    let deliveries = sqlx::query!(
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
    let turns: Vec<Uuid> = deliveries.iter().map(|delivery| delivery.turn).collect();
    let mut attachments = super::terminal::attachments(tx, &turns).await?;
    let mut terminal = super::terminal::tool_progress(tx, &turns).await?;
    let mut partial = partial_replies(tx, &turns).await?;
    for delivery in deliveries {
        let delivery = Delivery {
            attachments: attachments.remove(&delivery.turn).unwrap_or_default(),
            terminal: terminal.remove(&delivery.turn).unwrap_or_default(),
            partial: partial.remove(&delivery.turn),
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

async fn partial_replies(
    tx: &mut Transaction<'_, Postgres>,
    turns: &[Uuid],
) -> Result<HashMap<Uuid, String>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT e.turn_id, string_agg(e.payload->>'text','' ORDER BY e.sequence) AS "text!"
        FROM turn_events e
        JOIN chat_turns t ON t.id=e.turn_id
        WHERE e.turn_id=ANY($1) AND t.status='running' AND e.payload->>'kind'='text_delta'
        GROUP BY e.turn_id
        "#,
        turns,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.turn_id, row.text))
        .collect())
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

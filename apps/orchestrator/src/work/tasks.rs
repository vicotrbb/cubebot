//! A delivery is a turn that belongs to the room, so nothing here reaches an agent's own conversation.
mod orchestrator;
mod tools;

use super::{Work, access, rooms};
use crate::identity::{Actor, IdentityError};
use cubebot_protocol::runtime::ProviderKind;
pub use orchestrator::{advance, advance_replying, orchestrate};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
pub(super) use tools::tool;
use uuid::Uuid;

const OPEN_TURNS: [&str; 4] = ["queued", "running", "waiting", "delegated"];

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateTask {
    pub request_id: Uuid,
    pub request: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Task {
    pub id: Uuid,
    /// The room whose messages and events carry the task's conversation.
    pub room_id: Uuid,
    /// The orchestrator's name for the task; `None` until its first answer.
    pub title: Option<String>,
    pub request: String,
    /// `working`, `needs_you`, `done` or `stopped`.
    pub status: String,
    /// A turn of this task waits for a person: to retry it, raise a budget or stop it.
    pub blocked: bool,
    /// A person's message waits for the orchestrator.
    pub reading: bool,
    /// The login of the person it belongs to.
    pub owner: String,
    pub created_at: String,
    pub updated_at: String,
    pub agents: Vec<rooms::Member>,
    pub items: Vec<TaskItem>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct TaskItem {
    pub id: Uuid,
    #[serde(skip)]
    pub task_id: Uuid,
    pub agent_id: Uuid,
    /// The number the agent knows it by.
    pub position: i32,
    pub title: String,
    /// `todo`, `doing`, `done` or `skipped`.
    pub status: String,
    pub note: Option<String>,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct TaskSettings {
    /// With `model`, what the orchestrator thinks with; empty means the connection agents use.
    pub provider: Option<ProviderKind>,
    pub model: Option<String>,
    /// Most agents one round of work may involve.
    pub max_agents: i32,
    /// Rounds of follow-up work the orchestrator may start before it asks the person.
    pub max_rounds: i32,
    /// Added to the orchestrator's own instructions.
    pub instructions: String,
    pub revision: i64,
}

fn parse_settings(row: SettingsRow) -> TaskSettings {
    TaskSettings {
        provider: row.provider.as_deref().and_then(ProviderKind::parse),
        model: row.model,
        max_agents: row.max_agents,
        max_rounds: row.max_rounds,
        instructions: row.instructions,
        revision: row.revision,
    }
}

struct SettingsRow {
    provider: Option<String>,
    model: Option<String>,
    max_agents: i32,
    max_rounds: i32,
    instructions: String,
    revision: i64,
}

fn valid_settings(input: &TaskSettings) -> bool {
    let model_valid = input.model.as_deref().is_none_or(|model| {
        (1..=128).contains(&model.len())
            && model
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-._:/~".contains(&byte))
    });
    input.provider.is_none() == input.model.is_none()
        && model_valid
        && (1..=16).contains(&input.max_agents)
        && (1..=16).contains(&input.max_rounds)
        && input.instructions.len() <= 8192
}

struct Row {
    id: Uuid,
    room_id: Uuid,
    title: Option<String>,
    request: String,
    status: String,
    blocked: bool,
    reading: bool,
    owner: String,
    created_at: String,
    updated_at: String,
}

impl Work {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn create_task(
        &self,
        actor: &Actor,
        input: &CreateTask,
    ) -> Result<Task, IdentityError> {
        let request = input.request.trim();
        if input.request_id.is_nil()
            || request.is_empty()
            || request.len() > 16384
            || request.contains('\0')
        {
            return Err(IdentityError::InvalidInput);
        }
        let mut tx = self.pool.begin().await?;
        access::lock_actor(&mut tx, actor).await?;
        if eligible(&mut tx, actor.id).await?.is_empty() {
            return Err(IdentityError::Conflict);
        }
        let previous = sqlx::query_scalar!(
            "SELECT id FROM tasks WHERE created_by=$1 AND request_id=$2",
            actor.id,
            input.request_id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let id = match previous {
            Some(id) => id,
            None => insert(&mut tx, actor.id, input.request_id, request).await?,
        };
        let task = read(&mut tx, actor, Some(id)).await?.pop();
        tx.commit().await?;
        task.ok_or(IdentityError::NotFound)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn tasks(&self, actor: &Actor) -> Result<Vec<Task>, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let tasks = read(&mut tx, actor, None).await?;
        tx.commit().await?;
        Ok(tasks)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, task = %task))]
    pub async fn task(&self, actor: &Actor, task: Uuid) -> Result<Task, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let found = read(&mut tx, actor, Some(task)).await?.pop();
        tx.commit().await?;
        found.ok_or(IdentityError::NotFound)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, task = %task))]
    pub async fn stop_task(&self, actor: &Actor, task: Uuid) -> Result<Task, IdentityError> {
        let mut tx = self.pool.begin().await?;
        access::lock_actor(&mut tx, actor).await?;
        let room = read(&mut tx, actor, Some(task))
            .await?
            .pop()
            .ok_or(IdentityError::NotFound)?
            .room_id;
        // Waits out an orchestrator decision being applied, so its new turns are stopped too.
        lock_room(&mut tx, room).await?;
        let turns = sqlx::query!(
            r#"
            SELECT c.id, c.conversation_id
            FROM chat_turns c
            JOIN room_messages m ON m.id=c.room_message_id
            WHERE m.room_id=$1 AND c.status=ANY($2)
            "#,
            room,
            &OPEN_TURNS.map(str::to_owned),
        )
        .fetch_all(&mut *tx)
        .await?;
        for turn in turns {
            super::collaboration::cancel_subtree(
                &mut tx,
                crate::agents::AgentId(turn.conversation_id),
                turn.id,
            )
            .await?;
        }
        sqlx::query!(
            "UPDATE room_messages SET routing='routed' WHERE room_id=$1 AND routing='routing'",
            room,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "UPDATE tasks SET status='stopped', attempts=0, updated_at=clock_timestamp() WHERE id=$1",
            task,
        )
        .execute(&mut *tx)
        .await?;
        let stopped = read(&mut tx, actor, Some(task)).await?.pop();
        tx.commit().await?;
        stopped.ok_or(IdentityError::NotFound)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn task_settings(&self, actor: &Actor) -> Result<TaskSettings, IdentityError> {
        let mut tx = self.pool.begin().await?;
        crate::identity::administrator(&mut *tx, actor.id).await?;
        let settings = settings(&mut tx).await?;
        tx.commit().await?;
        Ok(settings)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn save_task_settings(
        &self,
        actor: &Actor,
        input: &TaskSettings,
    ) -> Result<TaskSettings, IdentityError> {
        if !valid_settings(input) {
            return Err(IdentityError::InvalidInput);
        }
        let mut tx = self.pool.begin().await?;
        crate::identity::administrator(&mut *tx, actor.id).await?;
        let saved = sqlx::query_as!(
            SettingsRow,
            "
            UPDATE task_settings
            SET provider=$1, model=$2, max_agents=$3, max_rounds=$4, instructions=$5, revision=revision+1
            WHERE revision=$6
            RETURNING provider, model, max_agents, max_rounds, instructions, revision
            ",
            input.provider.map(ProviderKind::as_str),
            input.model,
            input.max_agents,
            input.max_rounds,
            input.instructions,
            input.revision,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::Conflict)?;
        tx.commit().await?;
        Ok(parse_settings(saved))
    }
}

pub(super) async fn settings(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<TaskSettings, sqlx::Error> {
    sqlx::query_as!(
        SettingsRow,
        "SELECT provider, model, max_agents, max_rounds, instructions, revision FROM task_settings",
    )
    .fetch_one(&mut **tx)
    .await
    .map(parse_settings)
}

async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    actor: Uuid,
    request_id: Uuid,
    request: &str,
) -> Result<Uuid, IdentityError> {
    let room = sqlx::query_scalar!(
        "INSERT INTO rooms (name, created_by, request_id, task) VALUES ('Task', $1, $2, true) RETURNING id",
        actor,
        request_id,
    )
    .fetch_one(&mut **tx)
    .await?;
    let id = sqlx::query_scalar!(
        "INSERT INTO tasks (room_id, created_by, request_id, request) VALUES ($1, $2, $3, $4) RETURNING id",
        room,
        actor,
        request_id,
        request,
    )
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query!(
        "INSERT INTO room_messages (room_id, actor_id, request_id, message, routing) VALUES ($1, $2, $3, $4, 'routing')",
        room,
        actor,
        request_id,
        request,
    )
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

async fn read(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    task: Option<Uuid>,
) -> Result<Vec<Task>, IdentityError> {
    let rows = sqlx::query_as!(
        Row,
        r#"
        SELECT t.id, t.room_id, t.title, t.request, t.status,
            EXISTS(SELECT 1 FROM chat_turns c JOIN room_messages m ON m.id=c.room_message_id WHERE m.room_id=t.room_id AND c.status='waiting') AS "blocked!",
            EXISTS(SELECT 1 FROM room_messages m WHERE m.room_id=t.room_id AND m.routing='routing') AS "reading!",
            o.login AS owner,
            to_jsonb(t.created_at) #>> '{}' AS "created_at!",
            to_jsonb(t.updated_at) #>> '{}' AS "updated_at!"
        FROM tasks t
        JOIN human_accounts o ON o.id=t.created_by
        JOIN human_accounts a ON a.id=$1 AND a.active
        WHERE (a.role='administrator' OR t.created_by=a.id) AND ($2::uuid IS NULL OR t.id=$2)
        ORDER BY t.updated_at DESC
        LIMIT 100
        "#,
        actor.id,
        task,
    )
    .fetch_all(&mut **tx)
    .await?;
    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let mut items = sqlx::query_as!(
        TaskItem,
        "SELECT id, task_id, agent_id, position, title, status, note FROM task_items WHERE task_id=ANY($1) ORDER BY position",
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut tasks = Vec::with_capacity(rows.len());
    for row in rows {
        let (mine, rest) = items.into_iter().partition(|item| item.task_id == row.id);
        items = rest;
        tasks.push(Task {
            agents: rooms::members(tx, row.room_id).await?,
            items: mine,
            id: row.id,
            room_id: row.room_id,
            title: row.title,
            request: row.request,
            status: row.status,
            blocked: row.blocked,
            reading: row.reading,
            owner: row.owner,
            created_at: row.created_at,
            updated_at: row.updated_at,
        });
    }
    Ok(tasks)
}

/// Locks in the order every writer of a task uses: its room, then its row.
async fn lock_room(tx: &mut Transaction<'_, Postgres>, room: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query_scalar!("SELECT id FROM rooms WHERE id=$1 FOR UPDATE", room)
        .fetch_one(&mut **tx)
        .await?;
    sqlx::query_scalar!("SELECT id FROM tasks WHERE room_id=$1 FOR UPDATE", room)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn guard(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    turn: Uuid,
) -> Result<(), IdentityError> {
    let refused = sqlx::query_scalar!(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM chat_turns c
            JOIN room_messages m ON m.id=c.room_message_id
            JOIN tasks t ON t.room_id=m.room_id
            JOIN human_accounts a ON a.id=$1
            WHERE c.id=$2 AND t.created_by<>a.id AND a.role<>'administrator'
        ) AS "refused!"
        "#,
        actor.id,
        turn,
    )
    .fetch_one(&mut **tx)
    .await?;
    if refused {
        return Err(IdentityError::NotFound);
    }
    Ok(())
}

#[derive(Clone)]
pub(super) struct Eligible {
    pub id: Uuid,
    pub name: String,
    pub about: String,
    pub skills: Vec<String>,
    pub open: i64,
    pub lanes: i16,
}

pub(super) async fn eligible(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
) -> Result<Vec<Eligible>, sqlx::Error> {
    sqlx::query_as!(
        Eligible,
        r#"
        SELECT g.id, g.name, left(g.instructions,300) AS "about!", g.task_lanes AS lanes,
            ARRAY(SELECT s.name FROM skill_installations i JOIN skills s ON s.id=i.skill_id WHERE i.agent_id=g.id ORDER BY s.name) AS "skills!",
            (SELECT count(*) FROM chat_turns c WHERE c.conversation_id=g.id AND c.status IN ('queued','running','waiting','delegated')) AS "open!"
        FROM agents g
        JOIN human_accounts a ON a.id=$1 AND a.active
        WHERE g.desired_state<>'retired'
            AND (a.role='administrator' OR EXISTS(SELECT 1 FROM effective_human_grants e WHERE e.actor_id=a.id AND e.scope_kind='conversation' AND e.scope_id=g.id AND e.capability='conversation.send'))
        ORDER BY "open!", random()
        "#,
        owner,
    )
    .fetch_all(&mut **tx)
    .await
}

pub(super) async fn person_wrote(
    tx: &mut Transaction<'_, Postgres>,
    room: Uuid,
    message: &str,
) -> Result<Option<Vec<Uuid>>, IdentityError> {
    let woken = sqlx::query_scalar!(
        "
        UPDATE tasks
        SET status='working', rounds=0, attempts=0, updated_at=clock_timestamp()
        WHERE room_id=$1
        RETURNING created_by
        ",
        room,
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(owner) = woken else {
        return Ok(None);
    };
    let reachable: Vec<rooms::Member> = eligible(tx, owner)
        .await?
        .into_iter()
        .map(|agent| rooms::Member {
            id: agent.id,
            name: agent.name,
        })
        .collect();
    let mentioned = rooms::mentioned(message, &reachable);
    join(tx, room, &mentioned).await?;
    Ok(Some(mentioned))
}

async fn join(
    tx: &mut Transaction<'_, Postgres>,
    room: Uuid,
    agents: &[Uuid],
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "
        INSERT INTO room_members (room_id, agent_id)
        SELECT $1, unnest($2::uuid[])
        ON CONFLICT DO NOTHING
        ",
        room,
        agents,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn post(
    tx: &mut Transaction<'_, Postgres>,
    room: Uuid,
    owner: Uuid,
    request_id: Uuid,
    agent: Option<Uuid>,
    message: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "
        INSERT INTO room_messages (room_id, actor_id, request_id, message, routing, orchestrator, author_agent_id)
        VALUES ($1, $2, $3, $4, 'routed', $5, $6)
        ON CONFLICT (actor_id, request_id) DO NOTHING
        RETURNING id
        ",
        room,
        owner,
        request_id,
        clipped(message, 16384),
        agent.is_none(),
        agent,
    )
    .fetch_optional(&mut **tx)
    .await
}

async fn deliver(
    tx: &mut Transaction<'_, Postgres>,
    message: Uuid,
    owner: Uuid,
    agent: Uuid,
    frame: &str,
) -> Result<(), IdentityError> {
    super::wake(tx, crate::agents::AgentId(agent)).await?;
    sqlx::query!(
        "
        INSERT INTO chat_turns (conversation_id, actor_id, request_id, message, room_message_id, in_task)
        VALUES ($1, $2, gen_random_uuid(), $3, $4, true)
        ON CONFLICT (room_message_id,conversation_id) WHERE room_message_id IS NOT NULL DO NOTHING
        ",
        agent,
        owner,
        clipped(frame, 20480),
        message,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn clipped(text: &str, bytes: usize) -> &str {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping_keeps_whole_characters() {
        assert_eq!(clipped("héllo", 2), "h");
        assert_eq!(clipped("héllo", 3), "hé");
        assert_eq!(clipped("hi", 9), "hi");
    }

    #[test]
    fn a_chosen_model_names_its_provider_and_limits_stay_in_range() {
        let settings = |provider, model: Option<&str>, agents| TaskSettings {
            provider,
            model: model.map(str::to_owned),
            max_agents: agents,
            max_rounds: 3,
            instructions: String::new(),
            revision: 1,
        };
        assert!(valid_settings(&settings(None, None, 3)));
        assert!(valid_settings(&settings(
            Some(ProviderKind::Gemini),
            Some("gemini-3.8-flash"),
            3
        )));
        assert!(!valid_settings(&settings(
            Some(ProviderKind::Gemini),
            None,
            3
        )));
        assert!(!valid_settings(&settings(None, Some("m"), 3)));
        assert!(!valid_settings(&settings(None, None, 0)));
        assert!(!valid_settings(&settings(None, None, 17)));
    }
}

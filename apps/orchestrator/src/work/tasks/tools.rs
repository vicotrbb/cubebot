//! What an agent can do from a task turn: tick its items and talk in the room. A message naming
//! other agents starts a turn for each, so agents coordinate where people can see it.
use super::{clipped, deliver, eligible, join, post};
use crate::{agents::runtime::RuntimeActor, identity::IdentityError};
use cubebot_protocol::runtime::RoutineCall;
use serde::Deserialize;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Turns a task's agents may start each other in, so two agents cannot talk forever.
const DELIVERIES: i64 = 64;

type Answer = Result<String, String>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    item: i32,
    status: String,
    note: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Said {
    message: String,
    to: Vec<String>,
}

/// One `task_item` or `task_message` call for the running `turn`; `Err` is the refusal the model
/// reads.
pub(in crate::work) async fn tool(
    tx: &mut Transaction<'_, Postgres>,
    actor: &RuntimeActor,
    turn: Uuid,
    call: &RoutineCall,
) -> Result<Answer, IdentityError> {
    let task = sqlx::query!(
        "
        SELECT t.id, t.room_id, t.created_by
        FROM chat_turns c
        JOIN room_messages m ON m.id=c.room_message_id
        JOIN tasks t ON t.room_id=m.room_id
        WHERE c.id=$1
        ",
        turn,
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(task) = task else {
        return Ok(Err("this turn is not part of a task".into()));
    };
    match call.tool.as_str() {
        "task_item" => match Item::deserialize(&call.input) {
            Ok(item) => item_update(tx, actor, task.id, &item).await,
            Err(_) => Ok(Err(
                "item, status and note are required; status is doing, done or skipped".into(),
            )),
        },
        "task_message" => match Said::deserialize(&call.input) {
            Ok(said) if valid_message(&said) => {
                let room = (task.id, task.room_id);
                say(tx, actor, task.created_by, room, call.call, &said).await
            }
            _ => Ok(Err(
                "message and to are required; a message is at most 16 KiB".into(),
            )),
        },
        _ => Ok(Err("no task tool has this name".into())),
    }
}

fn valid_message(said: &Said) -> bool {
    !said.message.trim().is_empty()
        && !said.message.contains('\0')
        && said.message.len() <= 16384
        && said.to.len() <= 16
}

async fn item_update(
    tx: &mut Transaction<'_, Postgres>,
    actor: &RuntimeActor,
    task: Uuid,
    item: &Item,
) -> Result<Answer, IdentityError> {
    if !["doing", "done", "skipped"].contains(&item.status.as_str()) {
        return Ok(Err("status is doing, done or skipped".into()));
    }
    let note = item
        .note
        .as_deref()
        .filter(|note| !note.contains('\0'))
        .map(|note| clipped(note, 2048));
    let updated = sqlx::query!(
        "
        UPDATE task_items
        SET status=$4, note=COALESCE($5, note)
        WHERE task_id=$1 AND position=$2 AND agent_id=$3
        ",
        task,
        item.item,
        actor.agent.0,
        item.status,
        note,
    )
    .execute(&mut **tx)
    .await?;
    if updated.rows_affected() == 0 {
        return Ok(Err(format!("item #{} is not one of yours", item.item)));
    }
    sqlx::query!(
        "UPDATE tasks SET updated_at=clock_timestamp() WHERE id=$1",
        task
    )
    .execute(&mut **tx)
    .await?;
    Ok(Ok(format!("Item #{} is {}.", item.item, item.status)))
}

/// Posts to the room and starts a turn for each named agent. The call's id makes a resent call post
/// nothing new.
async fn say(
    tx: &mut Transaction<'_, Postgres>,
    actor: &RuntimeActor,
    owner: Uuid,
    (task, room): (Uuid, Uuid),
    call: Uuid,
    said: &Said,
) -> Result<Answer, IdentityError> {
    let reachable = eligible(tx, owner).await?;
    let mut targets = Vec::new();
    for name in &said.to {
        let matching: Vec<&super::Eligible> = reachable
            .iter()
            .filter(|agent| agent.name.eq_ignore_ascii_case(name) && agent.id != actor.agent.0)
            .collect();
        let [target] = matching.as_slice() else {
            return Ok(Err(format!("no other agent here is named exactly {name}")));
        };
        if !targets.contains(&target.id) {
            targets.push(target.id);
        }
    }
    if !targets.is_empty() && deliveries(tx, room).await? >= DELIVERIES {
        return Ok(Err(
            "this task has reached its limit of agent messages; finish with what you have".into(),
        ));
    }
    let Some(message) = post(tx, room, owner, call, Some(actor.agent.0), &said.message).await?
    else {
        return Ok(Ok("Already posted.".into()));
    };
    join(tx, room, &targets).await?;
    let name = sqlx::query_scalar!("SELECT name FROM agents WHERE id=$1", actor.agent.0)
        .fetch_one(&mut **tx)
        .await?;
    let frame = format!(
        "In a task's private room, {name} wrote to you:\n\n{}\n\nAnswer by ending your turn; your reply shows in the room.",
        said.message
    );
    for target in &targets {
        deliver(tx, message, owner, *target, &frame).await?;
    }
    sqlx::query!(
        "UPDATE tasks SET status='working', updated_at=clock_timestamp() WHERE id=$1 AND status<>'stopped'",
        task
    )
    .execute(&mut **tx)
    .await?;
    Ok(Ok(if targets.is_empty() {
        "Posted to the room.".to_owned()
    } else {
        format!(
            "Posted; {} will answer in the room. You do not wait for them.",
            said.to.join(", ")
        )
    }))
}

async fn deliveries(tx: &mut Transaction<'_, Postgres>, room: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"
        SELECT count(*) AS "count!"
        FROM chat_turns c
        JOIN room_messages m ON m.id=c.room_message_id
        WHERE m.room_id=$1 AND m.author_agent_id IS NOT NULL
        "#,
        room,
    )
    .fetch_one(&mut **tx)
    .await
}

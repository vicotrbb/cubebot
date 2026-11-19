use super::clipped;
use crate::{agents::runtime::RuntimeActor, identity::IdentityError};
use cubebot_protocol::runtime::RoutineCall;
use serde::Deserialize;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

type Answer = Result<String, String>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    item: i32,
    status: String,
    note: Option<String>,
}

pub(in crate::work) async fn tool(
    tx: &mut Transaction<'_, Postgres>,
    actor: &RuntimeActor,
    turn: Uuid,
    call: &RoutineCall,
) -> Result<Answer, IdentityError> {
    let task = sqlx::query!(
        "
        SELECT t.id
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
        _ => Ok(Err("no task tool has this name".into())),
    }
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

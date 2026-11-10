use super::{Locked, SaveRoutine, insert, list, lock, occur, remove, update};
use crate::{
    agents::{AgentId, runtime::RuntimeActor},
    identity::IdentityError,
    work::Work,
};
use cubebot_protocol::runtime::RoutineCall;
use serde::Deserialize;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    id: Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    id: Uuid,
    expected_revision: i64,
    routine: SaveRoutine,
}

const INVALID: &str = "name is 1 to 128 bytes; prompt up to 8 KiB and command up to 4 KiB, at least one of them; schedule is five-field cron or null; time_zone is an IANA zone name or null for UTC";

type Answer = Result<String, String>;

impl Work {
    #[tracing::instrument(skip_all, fields(agent = %actor.agent.0, turn = %turn, tool = %call.tool))]
    pub(crate) async fn routine_tool(
        &self,
        actor: &RuntimeActor,
        turn: Uuid,
        call: &RoutineCall,
    ) -> Result<Answer, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let human = sqlx::query_scalar!(
            "
            SELECT actor_id
            FROM chat_turns
            WHERE id=$1 AND conversation_id=$2 AND status='running' AND attempt_id=$3
            ",
            turn,
            actor.agent.0,
            actor.attempt.0,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(human) = human else {
            return Ok(Err("this turn is no longer running".into()));
        };
        if let Some(answer) = family(&mut tx, actor, (turn, human), call).await? {
            tx.commit().await?;
            return Ok(answer);
        }
        let answer = match dispatch(&mut tx, actor, human, call).await {
            Err(IdentityError::InvalidInput) => Err(INVALID.into()),
            Err(IdentityError::NotFound) => Err("no routine of yours has this id".into()),
            Err(IdentityError::Conflict) => {
                Err("the routine changed or the agent cannot run now; list it and retry".into())
            }
            other => other?,
        };
        tx.commit().await?;
        Ok(answer)
    }
}

/// Creation, team and task tools, which each answer on their own terms.
async fn family(
    tx: &mut Transaction<'_, Postgres>,
    actor: &RuntimeActor,
    (turn, human): (Uuid, Uuid),
    call: &RoutineCall,
) -> Result<Option<Answer>, IdentityError> {
    let tool = call.tool.as_str();
    if tool == "platform_list" || tool.ends_with("_create") && tool != "routine_create" {
        return Ok(Some(
            crate::work::creations::tool(tx, actor, (turn, human), call).await?,
        ));
    }
    if tool.starts_with("team_") {
        return Ok(Some(
            crate::work::teams::tool(tx, actor, human, call).await?,
        ));
    }
    if tool.starts_with("task_") {
        return Ok(Some(crate::work::tasks::tool(tx, actor, turn, call).await?));
    }
    Ok(None)
}

async fn dispatch(
    tx: &mut Transaction<'_, Postgres>,
    actor: &RuntimeActor,
    human: Uuid,
    call: &RoutineCall,
) -> Result<Answer, IdentityError> {
    match call.tool.as_str() {
        "routine_list" => {
            let routines = list(tx, actor.agent).await?;
            Ok(Ok(serde_json::to_string(&routines).unwrap_or_default()))
        }
        "routine_create" => create(tx, actor.agent, human, call).await,
        "routine_update" => change(tx, actor.agent, &call.input).await,
        "routine_delete" => delete(tx, actor.agent, &call.input).await,
        "routine_run" => run(tx, actor.agent, human, call).await,
        _ => Ok(Err("no routine tool has this name".into())),
    }
}

async fn create(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    human: Uuid,
    call: &RoutineCall,
) -> Result<Answer, IdentityError> {
    let Ok(input) = SaveRoutine::deserialize(&call.input) else {
        return Err(IdentityError::InvalidInput);
    };
    Ok(
        match insert(tx, agent, (human, "agent", call.call), &input).await? {
            Ok((id, _)) => Ok(format!("Created routine {id}.")),
            Err(reason) => Err(reason.into()),
        },
    )
}

async fn change(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    input: &serde_json::Value,
) -> Result<Answer, IdentityError> {
    let Ok(change) = Change::deserialize(input) else {
        return Ok(Err("arguments are id, expected_revision and routine".into()));
    };
    let locked = match managed(tx, agent, change.id).await? {
        Ok(locked) => locked,
        Err(reason) => return Ok(Err(reason)),
    };
    if locked.revision != change.expected_revision {
        return Err(IdentityError::Conflict);
    }
    update(tx, &locked, &change.routine, "agent").await?;
    Ok(Ok(format!("Updated routine {}.", change.id)))
}

async fn delete(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    input: &serde_json::Value,
) -> Result<Answer, IdentityError> {
    let Ok(target) = Target::deserialize(input) else {
        return Ok(Err("the argument is the routine's id".into()));
    };
    Ok(match managed(tx, agent, target.id).await? {
        Ok(locked) => {
            remove(tx, locked.id).await?;
            Ok(format!("Deleted routine {}.", target.id))
        }
        Err(reason) => Err(reason),
    })
}

async fn run(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    human: Uuid,
    call: &RoutineCall,
) -> Result<Answer, IdentityError> {
    let Ok(target) = Target::deserialize(&call.input) else {
        return Ok(Err("the argument is the routine's id".into()));
    };
    let locked = lock(tx, agent, target.id).await?;
    Ok(Ok(
        match occur(tx, &locked, "agent", human, call.call, None).await? {
            Some(turn) => format!(
                "Queued routine {} as turn {turn}; it starts after this turn.",
                target.id
            ),
            None => "A run of this routine is already pending; nothing new was queued.".into(),
        },
    ))
}

async fn managed(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    routine: Uuid,
) -> Result<Result<Locked, String>, IdentityError> {
    let locked = lock(tx, agent, routine).await?;
    Ok(match locked.managed_by.as_str() {
        "agent" => Ok(locked),
        _ => Err("a person manages this routine; only they can change it".into()),
    })
}

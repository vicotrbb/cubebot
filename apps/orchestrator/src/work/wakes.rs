use super::{
    Work, access,
    routines::{
        authorize,
        cron::{self, Cron},
        now,
    },
    wake,
};
use crate::{
    agents::AgentId,
    identity::{Actor, IdentityError},
};
use chrono::DateTime;
use cubebot_protocol::runtime::RoutineCall;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

const PER_AGENT: i64 = 32;
const YEAR: i64 = 366 * 86_400;
const INVALID: &str = "instructions are 1 to 4096 bytes; give exactly one of at (RFC 3339, within a year), delay_minutes (1 to 527040) or schedule (five-field cron); time_zone is an IANA zone name or null for UTC";

type Answer = Result<String, String>;

#[derive(Serialize, utoipa::ToSchema)]
pub struct Wake {
    pub id: Uuid,
    pub instructions: String,
    pub schedule: Option<String>,
    pub time_zone: String,
    pub next_at: String,
    pub set_by: String,
    pub created_at: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveWake {
    pub instructions: String,
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default)]
    pub delay_minutes: Option<i64>,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub time_zone: Option<String>,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateWake {
    pub request_id: Uuid,
    pub wake: SaveWake,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    id: Uuid,
}

impl Work {
    #[tracing::instrument(skip_all, fields(actor = %actor.id, agent = %agent.0))]
    pub async fn wakes(&self, actor: &Actor, agent: AgentId) -> Result<Vec<Wake>, IdentityError> {
        let mut tx = self.pool.begin().await?;
        authorize(&mut tx, actor, agent, "conversation.read").await?;
        let wakes = list(&mut tx, agent).await?;
        tx.commit().await?;
        Ok(wakes)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, agent = %agent.0))]
    pub async fn create_wake(
        &self,
        actor: &Actor,
        agent: AgentId,
        input: &CreateWake,
    ) -> Result<Wake, IdentityError> {
        let mut tx = self.pool.begin().await?;
        access::lock_actor(&mut tx, actor).await?;
        authorize(&mut tx, actor, agent, "conversation.send").await?;
        let made = (actor.id, "person", input.request_id);
        let id = insert(&mut tx, agent, made, &input.wake)
            .await?
            .map_err(|_| IdentityError::Conflict)?;
        let wake = one(&mut tx, agent, id).await?;
        tx.commit().await?;
        Ok(wake)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, agent = %agent.0, wake = %wake))]
    pub async fn update_wake(
        &self,
        actor: &Actor,
        agent: AgentId,
        wake: Uuid,
        input: &SaveWake,
    ) -> Result<Wake, IdentityError> {
        let mut tx = self.pool.begin().await?;
        authorize(&mut tx, actor, agent, "conversation.send").await?;
        let next = next_at(now(&mut tx).await?, input).ok_or(IdentityError::InvalidInput)?;
        sqlx::query_scalar!(
            "
            UPDATE wakes
            SET instructions=$3, schedule=$4, time_zone=$5, next_at=to_timestamp($6), set_by='person', created_by=$7
            WHERE id=$1 AND agent_id=$2
            RETURNING id
            ",
            wake,
            agent.0,
            input.instructions.trim(),
            input.schedule.as_deref().map(str::trim),
            zone_name(input),
            next as f64,
            actor.id,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        let wake = one(&mut tx, agent, wake).await?;
        tx.commit().await?;
        Ok(wake)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, agent = %agent.0, wake = %wake))]
    pub async fn delete_wake(
        &self,
        actor: &Actor,
        agent: AgentId,
        wake: Uuid,
    ) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        authorize(&mut tx, actor, agent, "conversation.send").await?;
        sqlx::query_scalar!(
            "DELETE FROM wakes WHERE id=$1 AND agent_id=$2 RETURNING id",
            wake,
            agent.0,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        tx.commit().await?;
        Ok(())
    }
}

pub(crate) async fn tool(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    human: Uuid,
    call: &RoutineCall,
) -> Result<Answer, IdentityError> {
    match call.tool.as_str() {
        "wake_list" => Ok(Ok(
            serde_json::to_string(&list(tx, agent).await?).unwrap_or_default()
        )),
        "wake_set" => set(tx, agent, human, call).await,
        "wake_cancel" => cancel(tx, agent, &call.input).await,
        _ => Ok(Err("no wake tool has this name".into())),
    }
}

async fn set(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    human: Uuid,
    call: &RoutineCall,
) -> Result<Answer, IdentityError> {
    let Ok(input) = SaveWake::deserialize(&call.input) else {
        return Ok(Err(INVALID.into()));
    };
    let id = match insert(tx, agent, (human, "agent", call.call), &input).await {
        Ok(Ok(id)) => id,
        Ok(Err(reason)) => return Ok(Err(reason.into())),
        Err(IdentityError::InvalidInput) => return Ok(Err(INVALID.into())),
        Err(error) => return Err(error),
    };
    let wake = one(tx, agent, id).await?;
    Ok(Ok(format!(
        "Wake {id} set. It next wakes you at {} with a message in this conversation.",
        wake.next_at
    )))
}

async fn cancel(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    input: &serde_json::Value,
) -> Result<Answer, IdentityError> {
    let Ok(target) = Target::deserialize(input) else {
        return Ok(Err("the argument is the wake's id".into()));
    };
    let deleted = sqlx::query_scalar!(
        "DELETE FROM wakes WHERE id=$1 AND agent_id=$2 AND set_by='agent' RETURNING id",
        target.id,
        agent.0,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(match deleted {
        Some(_) => Ok(format!("Cancelled wake {}.", target.id)),
        None => Err("no wake you set has this id; a person's wake is theirs to change".into()),
    })
}

pub async fn tick(pool: &PgPool) -> Result<usize, IdentityError> {
    let mut tx = pool.begin().await?;
    let now = now(&mut tx).await?;
    let due = sqlx::query!(
        r#"
        SELECT w.id, w.agent_id AS "agent: AgentId", w.instructions, w.schedule, w.time_zone,
            w.set_by, w.created_by, h.login
        FROM wakes w
        JOIN agents g ON g.id=w.agent_id AND g.desired_state<>'retired'
        JOIN human_accounts h ON h.id=w.created_by
        WHERE w.next_at <= clock_timestamp()
        ORDER BY w.next_at
        LIMIT 64
        FOR UPDATE OF w SKIP LOCKED
        "#
    )
    .fetch_all(&mut *tx)
    .await?;
    for wake in &due {
        let message = message(&wake.set_by, &wake.login, &wake.instructions);
        fire(&mut tx, (wake.id, wake.agent, wake.created_by), &message).await?;
        let next = wake
            .schedule
            .as_deref()
            .and_then(Cron::parse)
            .zip(cron::zone(&wake.time_zone))
            .and_then(|(schedule, zone)| schedule.after_in(now, zone));
        match next {
            Some(next) => {
                sqlx::query!(
                    "UPDATE wakes SET next_at=to_timestamp($2) WHERE id=$1",
                    wake.id,
                    next as f64,
                )
                .execute(&mut *tx)
                .await?;
            }
            None => {
                sqlx::query!("DELETE FROM wakes WHERE id=$1", wake.id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }
    tx.commit().await?;
    Ok(due.len())
}

async fn fire(
    tx: &mut Transaction<'_, Postgres>,
    (id, agent, actor): (Uuid, AgentId, Uuid),
    message: &str,
) -> Result<(), IdentityError> {
    let pending = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM chat_turns WHERE wake_id=$1 AND status='queued') AS "pending!""#,
        id,
    )
    .fetch_one(&mut **tx)
    .await?;
    if pending {
        return Ok(());
    }
    match wake(tx, agent).await {
        Err(IdentityError::Conflict) => return Ok(()),
        other => other?,
    }
    sqlx::query!(
        "
        INSERT INTO chat_turns (conversation_id, actor_id, request_id, message, origin, wake_id)
        VALUES ($1, $2, gen_random_uuid(), $3, 'wake', $4)
        ",
        agent.0,
        actor,
        message,
        id,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn message(set_by: &str, login: &str, instructions: &str) -> String {
    let by = match set_by {
        "agent" => "you set".to_string(),
        _ => format!("{login} set for you"),
    };
    format!("Wake-up {by}:\n\n{instructions}")
}

fn zone_name(input: &SaveWake) -> &str {
    input.time_zone.as_deref().unwrap_or("UTC")
}

fn next_at(now: i64, input: &SaveWake) -> Option<i64> {
    let text = input.instructions.trim();
    if text.is_empty() || text.len() > 4096 || text.contains('\0') {
        return None;
    }
    let zone = cron::zone(zone_name(input))?;
    let at = match (&input.at, input.delay_minutes, &input.schedule) {
        (Some(at), None, None) => DateTime::parse_from_rfc3339(at.trim()).ok()?.timestamp(),
        (None, Some(minutes), None) => now + minutes.clamp(0, YEAR / 60) * 60,
        (None, None, Some(schedule)) => Cron::parse(schedule)?.after_in(now, zone)?,
        _ => return None,
    };
    (at > now && at <= now + YEAR).then_some(at)
}

async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    (human, set_by, request_id): (Uuid, &str, Uuid),
    input: &SaveWake,
) -> Result<Result<Uuid, &'static str>, IdentityError> {
    let next = next_at(now(tx).await?, input).ok_or(IdentityError::InvalidInput)?;
    match admit(tx, agent, request_id).await? {
        Ok(Some(existing)) => return Ok(Ok(existing)),
        Ok(None) => (),
        Err(reason) => return Ok(Err(reason)),
    }
    let id = sqlx::query_scalar!(
        "
        INSERT INTO wakes (agent_id, instructions, schedule, time_zone, next_at, set_by, created_by, request_id)
        VALUES ($1, $2, $3, $4, to_timestamp($5), $6, $7, $8)
        RETURNING id
        ",
        agent.0,
        input.instructions.trim(),
        input.schedule.as_deref().map(str::trim),
        zone_name(input),
        next as f64,
        set_by,
        human,
        request_id,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(Ok(id))
}

async fn admit(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    request_id: Uuid,
) -> Result<Result<Option<Uuid>, &'static str>, IdentityError> {
    sqlx::query!("SELECT id FROM agents WHERE id=$1 FOR UPDATE", agent.0)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
    let existing = sqlx::query_scalar!(
        "SELECT id FROM wakes WHERE agent_id=$1 AND request_id=$2",
        agent.0,
        request_id,
    )
    .fetch_optional(&mut **tx)
    .await?;
    if existing.is_some() {
        return Ok(Ok(existing));
    }
    let held = sqlx::query_scalar!(
        r#"SELECT count(*) AS "held!" FROM wakes WHERE agent_id=$1"#,
        agent.0,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(if held >= PER_AGENT {
        Err("this agent already holds 32 wakes")
    } else {
        Ok(None)
    })
}

async fn list(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
) -> Result<Vec<Wake>, sqlx::Error> {
    sqlx::query_as!(
        Wake,
        r#"
        SELECT id, instructions, schedule, time_zone, set_by,
            (to_jsonb(next_at) #>> '{}') AS "next_at!",
            (to_jsonb(created_at) #>> '{}') AS "created_at!"
        FROM wakes
        WHERE agent_id=$1
        ORDER BY next_at, id
        "#,
        agent.0,
    )
    .fetch_all(&mut **tx)
    .await
}

async fn one(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    wake: Uuid,
) -> Result<Wake, IdentityError> {
    list(tx, agent)
        .await?
        .into_iter()
        .find(|listed| listed.id == wake)
        .ok_or(IdentityError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::{SaveWake, YEAR, next_at};

    fn save(at: Option<&str>, delay_minutes: Option<i64>, schedule: Option<&str>) -> SaveWake {
        SaveWake {
            instructions: "Check the deploy".into(),
            at: at.map(Into::into),
            delay_minutes,
            schedule: schedule.map(Into::into),
            time_zone: None,
        }
    }

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn a_wake_needs_exactly_one_time() {
        assert_eq!(next_at(NOW, &save(None, Some(30), None)), Some(NOW + 1800));
        assert_eq!(next_at(NOW, &save(None, None, None)), None);
        assert_eq!(next_at(NOW, &save(None, Some(30), Some("0 9 * * *"))), None);
    }

    #[test]
    fn a_wake_is_in_the_future_and_within_a_year() {
        assert_eq!(
            next_at(NOW, &save(Some("2020-01-01T00:00:00Z"), None, None)),
            None
        );
        assert_eq!(next_at(NOW, &save(None, Some(0), None)), None);
        assert_eq!(
            next_at(NOW, &save(None, Some(YEAR), None)),
            Some(NOW + YEAR)
        );
        assert_eq!(
            next_at(NOW, &save(Some("2027-01-15T12:00:00-03:00"), None, None)),
            Some(NOW + 7 * 3600)
        );
    }

    #[test]
    fn a_schedule_runs_next_on_its_cron() {
        let next = next_at(NOW, &save(None, None, Some("0 9 * * *"))).unwrap();
        assert!(next > NOW && next <= NOW + 86_400);
        assert_eq!(next_at(NOW, &save(None, None, Some("not cron"))), None);
    }
}

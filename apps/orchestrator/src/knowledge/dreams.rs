//! Dream runs (ADR 0025). The agent decides when it is idle enough to dream; the ledger admits a
//! run only while no turn is queued, running or waiting, tags every revision the run commits and
//! keeps its report. Commits go through the same stale-write and human-rejection checks as a turn,
//! so rerunning a dream over changes already made adds nothing.
use super::{Author, Knowledge, authorize, commit_change, search};
use crate::{
    agents::{AgentId, runtime::RuntimeActor},
    connections::Connections,
    identity::{Actor, IdentityError},
};
use cubebot_protocol::runtime::{Dream, DreamEnd, DreamOutcome, MemoryChange};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// shortcut: a run sees the 50 most recently changed memories, which bounds its prompt; page
/// through older memories once agents hold more than a run can consolidate.
const DREAM_MEMORIES: i64 = 50;
/// Runs the console lists, newest first.
const LISTED_RUNS: i64 = 50;

#[derive(Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct DreamRun {
    pub id: Uuid,
    pub model: String,
    /// `running` or `finished`.
    pub status: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    /// `completed`, `yielded`, `failed` or `no_op`; `None` while running.
    pub outcome: Option<String>,
    /// The model's account of the run; the committed revisions are the evidence.
    pub report: String,
    #[sqlx(skip)]
    pub revisions: Vec<DreamRevision>,
}

/// A revision the run committed, with the content it replaced.
#[derive(Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct DreamRevision {
    #[serde(skip)]
    pub run: Uuid,
    pub memory: Uuid,
    pub revision: i64,
    pub content: String,
    pub deleted: bool,
    /// `None` for a memory the run created.
    pub previous: Option<String>,
}

impl Knowledge {
    /// The agent's latest runs with the revisions each committed.
    pub async fn dreams(
        &self,
        actor: &Actor,
        agent: AgentId,
    ) -> Result<Vec<DreamRun>, IdentityError> {
        let mut tx = self.pool.begin().await?;
        authorize(&mut tx, actor, agent, "memory.read").await?;
        let mut runs: Vec<DreamRun> = sqlx::query_as("SELECT id,model,status,(to_jsonb(started_at) #>> '{}') AS started_at,(to_jsonb(ended_at) #>> '{}') AS ended_at,outcome,report FROM dream_runs WHERE agent_id=$1 ORDER BY started_at DESC LIMIT $2")
            .bind(agent).bind(LISTED_RUNS).fetch_all(&mut *tx).await?;
        let ids: Vec<Uuid> = runs.iter().map(|run| run.id).collect();
        let revisions: Vec<DreamRevision> = sqlx::query_as("SELECT r.dream_run_id AS run,r.memory_id AS memory,r.revision,r.content,r.deleted,p.content AS previous FROM memory_revisions r LEFT JOIN memory_revisions p ON p.memory_id=r.memory_id AND p.revision=r.revision-1 WHERE r.dream_run_id=ANY($1) ORDER BY r.created_at")
            .bind(&ids).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        for revision in revisions {
            if let Some(run) = runs.iter_mut().find(|run| run.id == revision.run) {
                run.revisions.push(revision);
            }
        }
        Ok(runs)
    }
}

/// Admits run `run` for the authenticated agent, or `None` when it may not dream now: work is
/// queued, running or waiting on an outcome, nothing changed since the last finished run, the id
/// was used, or no model is available. An unfinished earlier run is closed as failed.
pub(crate) async fn start_dream(
    pool: &PgPool,
    connections: Option<&Connections>,
    actor: &RuntimeActor,
    run: Uuid,
) -> Result<Option<Dream>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    // The agent row lock is the one a turn claim holds, so a message queued first wins.
    let (eligible, dreaming_model): (bool, Option<String>) = sqlx::query_as("SELECT g.desired_state='running' AND NOT EXISTS(SELECT 1 FROM chat_turns WHERE conversation_id=g.id AND status IN ('queued','running','waiting')) AND NOT EXISTS(SELECT 1 FROM dream_runs WHERE id=$2) AND EXISTS(SELECT 1 FROM memory_revisions r JOIN memories m ON m.id=r.memory_id WHERE m.agent_id=g.id AND r.dream_run_id IS NULL AND r.created_at > COALESCE((SELECT max(started_at) FROM dream_runs WHERE agent_id=g.id AND outcome IN ('completed','no_op')),'-infinity')), g.dreaming_model FROM agents g WHERE g.id=$1 FOR UPDATE")
        .bind(actor.agent).bind(run).fetch_one(&mut *tx).await?;
    if !eligible || run.is_nil() {
        return Ok(None);
    }
    let inference = match connections {
        Some(connections) => connections.inference(&mut tx, actor.agent).await.ok(),
        None => None,
    };
    let Some(inference) = inference else {
        return Ok(None);
    };
    let model = dreaming_model.unwrap_or(inference.model);
    sqlx::query("UPDATE dream_runs SET status='finished',outcome='failed',ended_at=clock_timestamp() WHERE agent_id=$1 AND status='running'")
        .bind(actor.agent).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO dream_runs (id,agent_id,attempt_id,model) VALUES ($1,$2,$3,$4)")
        .bind(run)
        .bind(actor.agent)
        .bind(actor.attempt)
        .bind(&model)
        .execute(&mut *tx)
        .await?;
    let memories = search(&mut tx, actor.agent, None, DREAM_MEMORIES).await?;
    tx.commit().await?;
    Ok(Some(Dream {
        provider: inference.provider,
        model,
        api_key: inference.api_key,
        memories: memories
            .into_iter()
            .map(|memory| cubebot_protocol::runtime::Memory {
                id: memory.id,
                revision: memory.revision,
                content: memory.content,
            })
            .collect(),
    }))
}

/// Commits one change of this attempt's running run. `false` once the run has ended.
pub(crate) async fn commit_dream(
    pool: &PgPool,
    actor: &RuntimeActor,
    run: Uuid,
    change: &MemoryChange,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let open: Option<Uuid> = sqlx::query_scalar("SELECT id FROM dream_runs WHERE id=$1 AND agent_id=$2 AND attempt_id=$3 AND status='running' FOR UPDATE")
        .bind(run).bind(actor.agent).bind(actor.attempt).fetch_optional(&mut *tx).await?;
    if open.is_none() {
        return Ok(false);
    }
    commit_change(&mut tx, actor.agent, Author::Dream(run), change).await?;
    tx.commit().await?;
    Ok(true)
}

/// Ends this attempt's running run. `false` when it already ended.
pub(crate) async fn finish_dream(
    pool: &PgPool,
    actor: &RuntimeActor,
    run: Uuid,
    end: &DreamEnd,
) -> Result<bool, sqlx::Error> {
    let outcome = match end.outcome {
        DreamOutcome::Completed => "completed",
        DreamOutcome::Yielded => "yielded",
        DreamOutcome::Failed => "failed",
    };
    let ended = sqlx::query("UPDATE dream_runs SET status='finished',ended_at=clock_timestamp(),report=$5,outcome=CASE WHEN $4='completed' AND NOT EXISTS(SELECT 1 FROM memory_revisions WHERE dream_run_id=$1) THEN 'no_op' ELSE $4 END WHERE id=$1 AND agent_id=$2 AND attempt_id=$3 AND status='running'")
        .bind(run).bind(actor.agent).bind(actor.attempt).bind(outcome).bind(&end.report).execute(pool).await?;
    Ok(ended.rows_affected() == 1)
}

/// A run whose sandbox lost its credential cannot report; it failed.
pub(crate) async fn recover_dreams(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE dream_runs d SET status='finished',outcome='failed',ended_at=clock_timestamp() WHERE status='running' AND NOT EXISTS(SELECT 1 FROM runtime_session_access s WHERE s.attempt_id=d.attempt_id AND s.recovery)")
        .execute(pool).await?;
    Ok(())
}

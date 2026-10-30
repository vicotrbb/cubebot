//! Owns persisted memory revisions and source links. People edit through the console; the agent's
//! staged changes commit with the turn that made them. Every write names the revision it read, so
//! a stale write never overwrites a newer one. Skills are not implemented yet.
mod dreams;
use crate::{
    agents::AgentId,
    identity::{Actor, IdentityError},
};
use cubebot_protocol::runtime::{MEMORY_BYTES, MemoryChange};
pub use dreams::{DreamRevision, DreamRun};
pub(crate) use dreams::{commit_dream, finish_dream, recover_dreams, start_dream};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// Search results an agent receives per query.
pub const AGENT_RESULTS: i64 = 10;
/// shortcut: the console sees the 200 most relevant or recent memories; page once agents hold more.
const CONSOLE_RESULTS: i64 = 200;

#[derive(Clone)]
pub struct Knowledge {
    pool: PgPool,
}

#[derive(Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Memory {
    pub id: Uuid,
    pub revision: i64,
    pub content: String,
    /// The turn whose completion wrote the current revision; `None` for a human edit or dream.
    pub source_turn: Option<Uuid>,
    /// The dream run that wrote the current revision.
    pub source_dream: Option<Uuid>,
    pub edited_by_human: bool,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateMemory {
    pub content: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviseMemory {
    pub content: String,
    pub expected_revision: i64,
}

/// Who wrote a revision: a person, the agent through the turn it completed, or a dream run.
#[derive(Clone, Copy, Debug)]
enum Author {
    Human(Uuid),
    Turn(Uuid),
    Dream(Uuid),
}

impl Knowledge {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Most relevant first for a query, most recent first without one.
    pub async fn search(
        &self,
        actor: &Actor,
        agent: AgentId,
        query: Option<&str>,
    ) -> Result<Vec<Memory>, IdentityError> {
        let mut tx = self.pool.begin().await?;
        authorize(&mut tx, actor, agent, "memory.read").await?;
        let found = search(&mut tx, agent, query, CONSOLE_RESULTS).await?;
        tx.commit().await?;
        Ok(found)
    }

    pub async fn create(
        &self,
        actor: &Actor,
        agent: AgentId,
        input: &CreateMemory,
    ) -> Result<Memory, IdentityError> {
        self.edit(actor, agent, None, None, Some(&input.content))
            .await
    }

    pub async fn revise(
        &self,
        actor: &Actor,
        agent: AgentId,
        memory: Uuid,
        input: &ReviseMemory,
    ) -> Result<Memory, IdentityError> {
        self.edit(
            actor,
            agent,
            Some(memory),
            Some(input.expected_revision),
            Some(&input.content),
        )
        .await
    }

    /// Hides the memory from retrieval; its revisions remain.
    pub async fn delete(
        &self,
        actor: &Actor,
        agent: AgentId,
        memory: Uuid,
        expected_revision: i64,
    ) -> Result<(), IdentityError> {
        self.edit(actor, agent, Some(memory), Some(expected_revision), None)
            .await
            .map(drop)
    }

    async fn edit(
        &self,
        actor: &Actor,
        agent: AgentId,
        memory: Option<Uuid>,
        expected: Option<i64>,
        content: Option<&str>,
    ) -> Result<Memory, IdentityError> {
        if content.is_some_and(|content| !valid(content)) {
            return Err(IdentityError::InvalidInput);
        }
        let mut tx = self.pool.begin().await?;
        authorize(&mut tx, actor, agent, "memory.edit").await?;
        let id = write(
            &mut tx,
            agent,
            memory,
            expected,
            content,
            Author::Human(actor.id),
        )
        .await?
        .ok_or(IdentityError::Conflict)?;
        let written = sqlx::query_as(&format!("{SELECT} WHERE m.id=$1"))
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(written)
    }

    /// Ownership comes from the authenticated runtime credential, never from the request.
    pub async fn search_for_agent(
        &self,
        agent: AgentId,
        query: &str,
    ) -> Result<Vec<Memory>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let found = search(&mut tx, agent, Some(query), AGENT_RESULTS).await?;
        tx.commit().await?;
        Ok(found)
    }
}

/// Commits a completed turn's staged changes in its settling transaction. A change against a
/// revision that is no longer current is dropped: whatever replaced it, often a human edit, wins.
pub async fn commit_turn(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    turn: Uuid,
    changes: &[MemoryChange],
) -> Result<(), sqlx::Error> {
    for change in changes {
        commit_change(tx, agent, Author::Turn(turn), change).await?;
    }
    Ok(())
}

async fn commit_change(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    author: Author,
    change: &MemoryChange,
) -> Result<(), sqlx::Error> {
    let (memory, expected, content) = parts(change);
    let reason = match content {
        Some(content) if unwanted(tx, agent, memory.is_none(), content).await? => {
            "duplicate or human-rejected"
        }
        _ => match write(tx, agent, memory, expected, content, author).await? {
            Some(_) => return Ok(()),
            None => "stale",
        },
    };
    tracing::info!(?author, ?memory, reason, "memory change dropped");
    Ok(())
}

/// The memory a change targets, the revision it read, and its content; `None` content removes.
fn parts(change: &MemoryChange) -> (Option<Uuid>, Option<i64>, Option<&str>) {
    match change {
        MemoryChange::Remember { content } => (None, None, Some(content)),
        MemoryChange::Revise {
            memory,
            expected_revision,
            content,
        } => (Some(*memory), Some(*expected_revision), Some(content)),
        MemoryChange::Forget {
            memory,
            expected_revision,
        } => (Some(*memory), Some(*expected_revision), None),
    }
}

/// Content a person replaced or removed is not brought back verbatim by the agent (ADR 0026),
/// and a new memory the agent already holds verbatim is not saved twice.
/// shortcut: exact text only; paraphrases pass, and the scan is unindexed per agent. Compare
/// normalized or embedded text once duplicates or slow commits show up.
async fn unwanted(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    new: bool,
    content: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT ($3 AND EXISTS(SELECT 1 FROM memories WHERE agent_id=$1 AND NOT deleted AND content=$2)) OR EXISTS(SELECT 1 FROM memory_revisions r JOIN memories m ON m.id=r.memory_id AND m.agent_id=$1 JOIN memory_revisions n ON n.memory_id=r.memory_id AND n.revision=r.revision+1 AND n.human_id IS NOT NULL AND (n.deleted OR n.content<>r.content) WHERE r.content=$2)")
        .bind(agent).bind(content).bind(new).fetch_one(&mut **tx).await
}

pub fn valid_change(change: &MemoryChange) -> bool {
    match change {
        MemoryChange::Remember { content } | MemoryChange::Revise { content, .. } => valid(content),
        MemoryChange::Forget { .. } => true,
    }
}

pub fn valid(content: &str) -> bool {
    !content.trim().is_empty() && content.len() <= MEMORY_BYTES && !content.contains('\0')
}

async fn authorize(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    agent: AgentId,
    capability: &str,
) -> Result<(), IdentityError> {
    let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agents g JOIN human_accounts a ON a.id=$1 AND a.active WHERE g.id=$2 AND (a.role='administrator' OR EXISTS(SELECT 1 FROM effective_human_grants p WHERE p.actor_id=a.id AND p.scope_kind='agent' AND p.scope_id=g.id AND p.capability=$3)))")
        .bind(actor.id).bind(agent).bind(capability).fetch_one(&mut **tx).await?;
    if !allowed {
        return Err(IdentityError::NotFound);
    }
    Ok(())
}

/// Creates a memory, or revises or removes (`content` of `None`) the current revision
/// `expected` of an agent's live memory. `None` when that revision is not current.
async fn write(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    memory: Option<Uuid>,
    expected: Option<i64>,
    content: Option<&str>,
    author: Author,
) -> Result<Option<Uuid>, sqlx::Error> {
    let (human, turn, dream) = match author {
        Author::Human(id) => (Some(id), None, None),
        Author::Turn(id) => (None, Some(id), None),
        Author::Dream(id) => (None, None, Some(id)),
    };
    let changed = match memory {
        None => {
            "INSERT INTO memories (agent_id,content) VALUES ($1,$4) RETURNING id,revision,content,deleted"
        }
        Some(_) => {
            "UPDATE memories SET content=COALESCE($4,content),deleted=$4 IS NULL,revision=revision+1,updated_at=clock_timestamp() WHERE id=$2 AND agent_id=$1 AND revision=$3 AND NOT deleted RETURNING id,revision,content,deleted"
        }
    };
    sqlx::query_scalar(&format!("WITH m AS ({changed}) INSERT INTO memory_revisions (memory_id,revision,content,deleted,human_id,turn_id,dream_run_id) SELECT id,revision,content,deleted,$5,$6,$7 FROM m RETURNING memory_id"))
        .bind(agent).bind(memory).bind(expected).bind(content).bind(human).bind(turn).bind(dream)
        .fetch_optional(&mut **tx).await
}

const SELECT: &str = "SELECT m.id,m.revision,m.content,r.turn_id AS source_turn,r.dream_run_id AS source_dream,r.human_id IS NOT NULL AS edited_by_human FROM memories m JOIN memory_revisions r ON r.memory_id=m.id AND r.revision=m.revision";

/// Any query word matches, ranked by how well; stop words alone match nothing.
async fn search(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    query: Option<&str>,
    limit: i64,
) -> Result<Vec<Memory>, sqlx::Error> {
    let query = query.map(str::trim).filter(|query| !query.is_empty());
    // Normalized lexemes, quoted with tsquery's own doubling, joined by OR.
    let words = r#"(SELECT (SELECT string_agg('''' || replace(replace(l,'\','\\'),'''','''''') || '''',' | ') FROM unnest(tsvector_to_array(to_tsvector('english',$2))) l)::tsquery AS q) s"#;
    sqlx::query_as(&format!("{SELECT}, {words} WHERE m.agent_id=$1 AND NOT m.deleted AND ($2::text IS NULL OR m.search @@ s.q) ORDER BY CASE WHEN $2::text IS NULL THEN 0 ELSE ts_rank(m.search,s.q) END DESC, m.updated_at DESC LIMIT $3"))
        .bind(agent).bind(query).bind(limit).fetch_all(&mut **tx).await
}

//! Durable ownership before external effects. No Kubernetes calls occur inside transactions.
mod anchor;
mod objects;
mod preparation;
mod recovery;
use super::Error;
use crate::agents::AgentId;
pub use objects::{ResourceObject, ResourceSpec};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(transparent)]
#[sqlx(transparent)]
pub struct AttemptId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum ResourceKind {
    Pod,
    Workspace,
}

impl ResourceKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pod => "pod",
            Self::Workspace => "workspace",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum Phase {
    Planned,
    Creating,
    Bound,
    Stopped,
}

pub struct Attempt {
    pub id: AttemptId,
    pub agent: AgentId,
    pub kind: ResourceKind,
    pub epoch: i64,
    pub target_generation: i64,
    pub definition_revision: i64,
    pub phase: Phase,
    pub uid: Option<String>,
    pub blocked_reason: Option<String>,
    pub object: ResourceObject,
    pub intent: ResourceObject,
    pub prepared: bool,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: AttemptId,
    agent_id: AgentId,
    kind: ResourceKind,
    epoch: i64,
    target_generation: i64,
    definition_revision: i64,
    phase: Phase,
    uid: Option<String>,
    blocked_reason: Option<String>,
    manifest: sqlx::types::Json<serde_json::Value>,
    source_manifest: sqlx::types::Json<serde_json::Value>,
    prepared: bool,
}
impl Row {
    fn attempt(self) -> Result<Attempt, Error> {
        Ok(Attempt {
            id: self.id,
            agent: self.agent_id,
            kind: self.kind,
            epoch: self.epoch,
            target_generation: self.target_generation,
            definition_revision: self.definition_revision,
            phase: self.phase,
            uid: self.uid,
            blocked_reason: self.blocked_reason,
            object: ResourceObject::decode(self.kind, self.manifest.0)?,
            intent: ResourceObject::decode(self.kind, self.source_manifest.0)?,
            prepared: self.prepared,
        })
    }
}

#[derive(Clone)]
pub struct Registry {
    pool: PgPool,
}
impl Registry {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn attempt(&self, id: AttemptId) -> Result<Attempt, Error> {
        let row:Row = sqlx::query_as("SELECT id, agent_id, kind, epoch, target_generation, definition_revision, phase, uid, blocked_reason, manifest, source_manifest, prepared FROM agent_resources WHERE id = $1")
            .bind(id).fetch_optional(&self.pool).await?.ok_or(Error::Conflict)?;
        row.attempt()
    }

    pub async fn reserve(
        &self,
        agent: AgentId,
        expected_revision: i64,
        spec: ResourceSpec,
    ) -> Result<Attempt, Error> {
        let mut transaction = self.transaction().await?;
        let generation =
            lock_agent(&mut transaction, agent, expected_revision, spec.kind()).await?;
        if let Some(attempt) = existing(&mut transaction, agent, generation, &spec).await? {
            return Ok(attempt);
        }
        let binding = anchor::load(&mut transaction).await?;
        let id = insert_attempt(
            &mut transaction,
            &binding,
            agent,
            expected_revision,
            generation,
            spec,
        )
        .await?;
        transaction.commit().await?;
        self.attempt(id).await
    }

    pub async fn begin_create(&self, id: AttemptId) -> Result<(), Error> {
        let mut transaction = self.transaction().await?;
        let (state,generation):(String,i64) = sqlx::query_as("SELECT a.desired_state,a.restart_generation FROM agents a JOIN agent_resources r ON r.agent_id = a.id WHERE r.id = $1 FOR UPDATE OF a")
            .bind(id).fetch_optional(&mut *transaction).await?.ok_or(Error::Conflict)?;
        let changed = sqlx::query("UPDATE agent_resources SET phase = 'creating', updated_at = clock_timestamp() WHERE id = $1 AND $2 <> 'retired' AND (kind = 'workspace' OR ($2 = 'running' AND target_generation = $3)) AND prepared AND phase IN ('planned','creating')")
            .bind(id).bind(state).bind(generation).execute(&mut *transaction).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Conflict);
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn bind_object(&self, id: AttemptId, observed: &ResourceObject) -> Result<(), Error> {
        let mut transaction = self.transaction().await?;
        let row:Row = sqlx::query_as("SELECT id,agent_id,kind,epoch,target_generation,definition_revision,phase,uid,blocked_reason,manifest,source_manifest,prepared FROM agent_resources WHERE id = $1 FOR UPDATE")
            .bind(id).fetch_optional(&mut *transaction).await?.ok_or(Error::Conflict)?;
        let expected = row.attempt()?;
        if !matches!(expected.phase, Phase::Creating | Phase::Bound) {
            return Err(Error::Conflict);
        }
        objects::verify(&expected, observed)?;
        let uid = objects::uid(observed.metadata())?;
        let version = observed
            .metadata()
            .resource_version
            .as_deref()
            .ok_or(Error::OwnershipMismatch)?;
        sqlx::query("UPDATE agent_resources SET uid = $2, resource_version = $3, phase = 'bound', blocked_reason = NULL, updated_at = clock_timestamp() WHERE id = $1")
            .bind(id).bind(uid).bind(version).execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(())
    }
}

impl Registry {
    async fn transaction(&self) -> Result<Transaction<'_, Postgres>, Error> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
            .execute(&mut *transaction)
            .await?;
        Ok(transaction)
    }
}

async fn lock_agent(
    transaction: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    expected: i64,
    kind: ResourceKind,
) -> Result<i64, Error> {
    let (revision, generation, state): (i64, i64, String) = sqlx::query_as(
        "SELECT revision,restart_generation,desired_state FROM agents WHERE id = $1 FOR UPDATE",
    )
    .bind(agent)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(Error::Conflict)?;
    if revision != expected
        || state == "retired"
        || (kind == ResourceKind::Pod && state != "running")
    {
        return Err(Error::Conflict);
    }
    Ok(generation)
}
async fn existing(
    transaction: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    generation: i64,
    spec: &ResourceSpec,
) -> Result<Option<Attempt>, Error> {
    let current:Option<Row> = sqlx::query_as("SELECT id,agent_id,kind,epoch,target_generation,definition_revision,phase,uid,blocked_reason,manifest,source_manifest,prepared FROM agent_resources WHERE agent_id = $1 AND kind = $2 AND phase <> 'stopped'")
        .bind(agent).bind(spec.kind().as_str()).fetch_optional(&mut **transaction).await?;
    let Some(current) = current else {
        return Ok(None);
    };
    let attempt = current.attempt()?;
    if (spec.kind() == ResourceKind::Pod && attempt.target_generation != generation)
        || !spec.matches(&attempt.intent)
    {
        if attempt.kind == ResourceKind::Pod && attempt.phase == Phase::Planned {
            sqlx::query("UPDATE agent_resources SET phase = 'stopped', termination_proof = '{\"kind\":\"not_dispatched\"}'::jsonb, updated_at = clock_timestamp() WHERE id = $1 AND phase = 'planned'")
                .bind(attempt.id).execute(&mut **transaction).await?;
            return Ok(None);
        }
        return Err(Error::Conflict);
    }
    Ok(Some(attempt))
}
async fn insert_attempt(
    transaction: &mut Transaction<'_, Postgres>,
    binding: &anchor::Binding,
    agent: AgentId,
    revision: i64,
    generation: i64,
    spec: ResourceSpec,
) -> Result<AttemptId, Error> {
    let kind = spec.kind();
    let epoch: i64 = if kind == ResourceKind::Workspace {
        0
    } else {
        sqlx::query_scalar("SELECT coalesce(max(epoch),0) + 1 FROM agent_resources WHERE agent_id = $1 AND kind = 'pod'").bind(agent).fetch_one(&mut **transaction).await?
    };
    let id = AttemptId(Uuid::new_v4());
    let object = spec.object(binding, agent, id, epoch)?;
    let metadata = object.metadata();
    let manifest = serde_json::to_value(&object).map_err(|_| Error::InvalidConfiguration)?;
    sqlx::query("INSERT INTO agent_resources (id,agent_id,kind,epoch,target_generation,definition_revision,namespace,name,manifest,source_manifest) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$9)")
        .bind(id).bind(agent).bind(kind.as_str()).bind(epoch).bind(generation).bind(revision).bind(metadata.namespace.as_deref()).bind(metadata.name.as_deref()).bind(sqlx::types::Json(manifest)).execute(&mut **transaction).await?;
    Ok(id)
}

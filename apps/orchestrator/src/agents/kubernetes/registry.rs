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
    #[tracing::instrument(skip_all, fields(attempt = %id.0))]
    pub async fn attempt(&self, id: AttemptId) -> Result<Attempt, Error> {
        let row = sqlx::query_as!(
            Row,
            r#"
            SELECT id AS "id: AttemptId", agent_id AS "agent_id: AgentId", kind AS "kind: ResourceKind",
                   epoch, target_generation, definition_revision, phase AS "phase: Phase", uid,
                   blocked_reason,
                   manifest AS "manifest: sqlx::types::Json<serde_json::Value>",
                   source_manifest AS "source_manifest: sqlx::types::Json<serde_json::Value>",
                   prepared
            FROM agent_resources
            WHERE id = $1
            "#,
            id.0,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(Error::Conflict)?;
        row.attempt()
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub(super) async fn current(
        &self,
        agent: AgentId,
        kind: ResourceKind,
    ) -> Result<Option<Attempt>, Error> {
        let row = sqlx::query_as!(
            Row,
            r#"
            SELECT id AS "id: AttemptId", agent_id AS "agent_id: AgentId", kind AS "kind: ResourceKind",
                   epoch, target_generation, definition_revision, phase AS "phase: Phase", uid,
                   blocked_reason,
                   manifest AS "manifest: sqlx::types::Json<serde_json::Value>",
                   source_manifest AS "source_manifest: sqlx::types::Json<serde_json::Value>",
                   prepared
            FROM agent_resources
            WHERE agent_id = $1
              AND kind = $2
              AND phase <> 'stopped'
            "#,
            agent.0,
            kind.as_str(),
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(Row::attempt).transpose()
    }

    #[tracing::instrument(skip_all, fields(attempt = %id.0))]
    pub(super) async fn block(
        &self,
        id: AttemptId,
        phase: Phase,
        reason: &str,
    ) -> Result<(), Error> {
        sqlx::query!(
            "
            UPDATE agent_resources
            SET blocked_reason = $3,
                updated_at = clock_timestamp()
            WHERE id = $1
              AND phase = $2
            ",
            id.0,
            phase as Phase,
            reason,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
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

    #[tracing::instrument(skip_all, fields(attempt = %id.0))]
    pub async fn begin_create(&self, id: AttemptId) -> Result<(), Error> {
        let mut transaction = self.transaction().await?;
        let intent = sqlx::query!(
            "
            SELECT a.desired_state, a.restart_generation
            FROM agents a
            JOIN agent_resources r ON r.agent_id = a.id
            WHERE r.id = $1
            FOR UPDATE OF a
            ",
            id.0,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(Error::Conflict)?;
        let changed = sqlx::query!(
            "
            UPDATE agent_resources
            SET phase = 'creating',
                blocked_reason = NULL,
                updated_at = clock_timestamp()
            WHERE id = $1
              AND $2 <> 'retired'
              AND (kind = 'workspace' OR ($2 = 'running' AND target_generation = $3))
              AND prepared
              AND phase = 'planned'
            ",
            id.0,
            intent.desired_state,
            intent.restart_generation,
        )
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if changed != 1 {
            return Err(Error::Conflict);
        }
        transaction.commit().await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(attempt = %id.0))]
    pub async fn bind_object(&self, id: AttemptId, observed: &ResourceObject) -> Result<(), Error> {
        let mut transaction = self.transaction().await?;
        lock_attempt_agent(&mut transaction, id).await?;
        let row = sqlx::query_as!(
            Row,
            r#"
            SELECT id AS "id: AttemptId", agent_id AS "agent_id: AgentId", kind AS "kind: ResourceKind",
                   epoch, target_generation, definition_revision, phase AS "phase: Phase", uid,
                   blocked_reason,
                   manifest AS "manifest: sqlx::types::Json<serde_json::Value>",
                   source_manifest AS "source_manifest: sqlx::types::Json<serde_json::Value>",
                   prepared
            FROM agent_resources
            WHERE id = $1
            FOR UPDATE
            "#,
            id.0,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(Error::Conflict)?;
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
        let observation = super::observation::Observation::of(observed);
        sqlx::query!(
            "
            UPDATE agent_resources
            SET uid = $2,
                resource_version = $3,
                phase = 'bound',
                blocked_reason = CASE WHEN $7 THEN 'runtime_recovery_required' ELSE NULL END,
                observed_at = transaction_timestamp(),
                kubernetes_phase = $4,
                kubernetes_reason = $8,
                kubernetes_ready = $5,
                deleting = $6,
                updated_at = clock_timestamp()
            WHERE id = $1
            ",
            id.0,
            uid,
            version,
            observation.phase,
            observation.ready,
            observation.deleting,
            observation.recovery,
            observation.reason,
        )
        .execute(&mut *transaction)
        .await?;
        if observation.recovery {
            latch_drain(&mut transaction, id).await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}

impl Registry {
    async fn transaction(&self) -> Result<Transaction<'_, Postgres>, Error> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query!("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
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
    let intent = sqlx::query!(
        "
        SELECT revision, restart_generation, desired_state
        FROM agents
        WHERE id = $1
        FOR UPDATE
        ",
        agent.0,
    )
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(Error::Conflict)?;
    if intent.revision != expected
        || intent.desired_state == "retired"
        || (kind == ResourceKind::Pod && intent.desired_state != "running")
    {
        return Err(Error::Conflict);
    }
    Ok(intent.restart_generation)
}
async fn existing(
    transaction: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    generation: i64,
    spec: &ResourceSpec,
) -> Result<Option<Attempt>, Error> {
    let current = sqlx::query_as!(
        Row,
        r#"
        SELECT id AS "id: AttemptId", agent_id AS "agent_id: AgentId", kind AS "kind: ResourceKind",
               epoch, target_generation, definition_revision, phase AS "phase: Phase", uid,
               blocked_reason,
               manifest AS "manifest: sqlx::types::Json<serde_json::Value>",
               source_manifest AS "source_manifest: sqlx::types::Json<serde_json::Value>",
               prepared
        FROM agent_resources
        WHERE agent_id = $1
          AND kind = $2
          AND phase <> 'stopped'
        "#,
        agent.0,
        spec.kind().as_str(),
    )
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(current) = current else {
        return Ok(None);
    };
    let attempt = current.attempt()?;
    if (spec.kind() == ResourceKind::Pod && attempt.target_generation != generation)
        || !spec.matches(&attempt.intent)
    {
        if attempt.kind == ResourceKind::Pod && attempt.phase == Phase::Planned {
            sqlx::query!(
                r#"
                UPDATE agent_resources
                SET phase = 'stopped',
                    termination_proof = '{"kind":"not_dispatched"}'::jsonb,
                    updated_at = clock_timestamp()
                WHERE id = $1
                  AND phase = 'planned'
                "#,
                attempt.id.0,
            )
            .execute(&mut **transaction)
            .await?;
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
        sqlx::query_scalar!(
            r#"
            SELECT coalesce(max(epoch), 0) + 1 AS "epoch!"
            FROM agent_resources
            WHERE agent_id = $1
              AND kind = 'pod'
            "#,
            agent.0,
        )
        .fetch_one(&mut **transaction)
        .await?
    };
    let id = AttemptId(Uuid::new_v4());
    let object = spec.object(binding, agent, id, epoch)?;
    let metadata = object.metadata();
    let manifest =
        serde_json::to_value(&object).map_err(|error| Error::Configuration(error.into()))?;
    sqlx::query!(
        "
        INSERT INTO agent_resources
            (id, agent_id, kind, epoch, target_generation, definition_revision, namespace, name,
             manifest, source_manifest)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)
        ",
        id.0,
        agent.0,
        kind.as_str(),
        epoch,
        generation,
        revision,
        metadata.namespace.as_deref(),
        metadata.name.as_deref(),
        manifest,
    )
    .execute(&mut **transaction)
    .await?;
    Ok(id)
}

async fn lock_attempt_agent(
    transaction: &mut Transaction<'_, Postgres>,
    id: AttemptId,
) -> Result<(), Error> {
    sqlx::query_scalar!(
        "
        SELECT a.id
        FROM agents a
        JOIN agent_resources r ON r.agent_id = a.id
        WHERE r.id = $1
        FOR UPDATE OF a
        ",
        id.0,
    )
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(Error::Conflict)?;
    Ok(())
}
async fn latch_drain(
    transaction: &mut Transaction<'_, Postgres>,
    id: AttemptId,
) -> Result<(), Error> {
    sqlx::query!(
        "
        INSERT INTO agent_runtime_drains (attempt_id)
        VALUES ($1)
        ON CONFLICT (attempt_id) DO NOTHING
        ",
        id.0,
    )
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

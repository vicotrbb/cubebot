//! Single active process. Losing PostgreSQL connectivity never permits blind takeover.
mod development;
mod proof;
use crate::agents::kubernetes::{Error, Kubernetes};
use sqlx::{Connection, PgConnection, PgPool};
use std::time::Duration;
use uuid::Uuid;

const LOCK: i64 = 0x4355_4245_424f_5401;

#[derive(Clone)]
pub struct PodIdentity {
    pub namespace: String,
    pub name: String,
    pub uid: String,
}
struct Row {
    instance_id: Uuid,
    generation: i64,
    namespace: String,
    pod_name: String,
    pod_uid: String,
    container_id: String,
    released: bool,
}
pub struct Owner {
    // Detached so dropping this guard closes the session instead of pooling its advisory lock.
    connection: Option<PgConnection>,
    instance: Uuid,
    generation: i64,
    table: &'static str,
}
impl Owner {
    #[tracing::instrument(skip_all, fields(namespace = %identity.namespace, pod = %identity.name))]
    pub async fn acquire(
        pool: &PgPool,
        api: &Kubernetes,
        identity: PodIdentity,
    ) -> Result<Self, Error> {
        tokio::time::timeout(Duration::from_secs(30), Self::claim(pool, api, identity))
            .await
            .map_err(|_| Error::DeadlineExceeded)?
    }
    async fn claim(pool: &PgPool, api: &Kubernetes, identity: PodIdentity) -> Result<Self, Error> {
        let mut connection = locked_connection(pool).await?;
        let previous = previous_owner(&mut connection, &identity).await?;
        let pod = api
            .platform_pod(&identity.name)
            .await?
            .ok_or(Error::OwnershipMismatch)?;
        let current = proof::current(pod, &identity)?;
        let evidence = prove_previous(api, &identity, &current, previous.as_ref()).await?;
        let instance = Uuid::new_v4();
        let expected_instance = previous
            .as_ref()
            .map_or(Uuid::nil(), |previous| previous.instance_id);
        let expected_generation = previous.as_ref().map_or(0, |previous| previous.generation);
        let generation = sqlx::query_scalar!(
            r#"
            INSERT INTO orchestrator_owner
                (singleton, instance_id, generation, namespace, pod_name, pod_uid, container_id, previous_termination_proof)
            VALUES (true, $1, 1, $2, $3, $4, $5, $6)
            ON CONFLICT (singleton) DO UPDATE SET
                instance_id = EXCLUDED.instance_id,
                generation = orchestrator_owner.generation + 1,
                pod_name = EXCLUDED.pod_name,
                pod_uid = EXCLUDED.pod_uid,
                container_id = EXCLUDED.container_id,
                acquired_at = clock_timestamp(),
                released_at = NULL,
                previous_termination_proof = EXCLUDED.previous_termination_proof
            WHERE orchestrator_owner.instance_id = $7
                AND orchestrator_owner.generation = $8
                AND orchestrator_owner.namespace = EXCLUDED.namespace
            RETURNING generation AS "generation!"
            "#,
            instance,
            &identity.namespace,
            &identity.name,
            &identity.uid,
            &current.container_id,
            evidence,
            expected_instance,
            expected_generation,
        )
        .fetch_optional(&mut connection)
        .await?
        .ok_or(Error::Conflict)?;
        Ok(Self {
            connection: Some(connection),
            instance,
            generation,
            table: "orchestrator_owner",
        })
    }
    pub fn instance(&self) -> Uuid {
        self.instance
    }
    pub fn generation(&self) -> i64 {
        self.generation
    }

    /// Any failure is fatal for this guard; callers must stop serving and issuing work.
    #[tracing::instrument(skip_all, fields(instance = %self.instance, generation = self.generation))]
    pub async fn check(&mut self) -> Result<(), Error> {
        let result = tokio::time::timeout(Duration::from_secs(5), self.check_session())
            .await
            .map_err(|_| Error::DeadlineExceeded)
            .and_then(|result| result);
        if result.is_err() {
            self.connection = None;
        }
        result
    }
    async fn check_session(&mut self) -> Result<(), Error> {
        let connection = self.connection.as_mut().ok_or(Error::Conflict)?;
        // Runtime SQL: the owner table is chosen per guard (production or development owner).
        let owned: bool = sqlx::query_scalar(&format!("SELECT EXISTS(SELECT 1 FROM {} WHERE singleton AND instance_id = $1 AND generation = $2 AND released_at IS NULL)",self.table))
            .bind(self.instance).bind(self.generation).fetch_one(connection).await?;
        if !owned {
            return Err(Error::Conflict);
        }
        Ok(())
    }

    /// Consume only after HTTP handlers and controller operations have fully stopped.
    /// Drop/crash deliberately does not release the durable claim.
    #[tracing::instrument(skip_all, fields(instance = %self.instance, generation = self.generation))]
    pub async fn release(self) -> Result<(), Error> {
        tokio::time::timeout(Duration::from_secs(10), self.release_session())
            .await
            .map_err(|_| Error::DeadlineExceeded)?
    }
    async fn release_session(mut self) -> Result<(), Error> {
        let mut connection = self.connection.take().ok_or(Error::Conflict)?;
        // Runtime SQL: the owner table is chosen per guard (production or development owner).
        let changed = sqlx::query(&format!("UPDATE {} SET released_at = clock_timestamp() WHERE singleton AND instance_id = $1 AND generation = $2 AND released_at IS NULL",self.table))
            .bind(self.instance).bind(self.generation).execute(&mut connection).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Conflict);
        }
        sqlx::query!("SELECT pg_advisory_unlock($1)", LOCK)
            .fetch_all(&mut connection)
            .await?;
        connection.close().await?;
        Ok(())
    }
}
async fn prove_previous(
    api: &Kubernetes,
    identity: &PodIdentity,
    current: &proof::Current,
    previous: Option<&Row>,
) -> Result<Option<serde_json::Value>, Error> {
    let Some(previous) = previous else {
        return Ok(None);
    };
    if previous.namespace != identity.namespace {
        return Err(Error::OwnershipMismatch);
    }
    if previous.released {
        return Ok(None);
    }
    if previous.pod_uid == identity.uid && previous.pod_name == identity.name {
        return proof::restart(&current.pod, previous).map(Some);
    }
    let old = api
        .platform_pod(&previous.pod_name)
        .await?
        .ok_or(Error::TerminationUnproven)?;
    proof::replacement(&old, previous).map(Some)
}

/// The durable claim to replace, after refusing an active development owner or another namespace.
async fn previous_owner(
    connection: &mut PgConnection,
    identity: &PodIdentity,
) -> Result<Option<Row>, Error> {
    let development_active = sqlx::query_scalar!(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM orchestrator_development_owner
            WHERE released_at IS NULL
        ) AS "active!"
        "#,
    )
    .fetch_one(&mut *connection)
    .await?;
    if development_active {
        return Err(Error::TerminationUnproven);
    }
    let selected = sqlx::query_scalar!(
        r#"
        SELECT namespace AS "namespace!"
        FROM installation_kubernetes
        WHERE singleton
        "#,
    )
    .fetch_optional(&mut *connection)
    .await?;
    if selected.is_some_and(|namespace| namespace != identity.namespace) {
        return Err(Error::OwnershipMismatch);
    }
    Ok(sqlx::query_as!(
        Row,
        r#"
        SELECT
            instance_id AS "instance_id!",
            generation AS "generation!",
            namespace AS "namespace!",
            pod_name AS "pod_name!",
            pod_uid AS "pod_uid!",
            container_id AS "container_id!",
            released_at IS NOT NULL AS "released!"
        FROM orchestrator_owner
        WHERE singleton
        "#,
    )
    .fetch_optional(&mut *connection)
    .await?)
}

async fn locked_connection(pool: &PgPool) -> Result<PgConnection, Error> {
    let mut connection = pool.acquire().await?.detach();
    let locked = sqlx::query_scalar!(
        r#"
        SELECT pg_try_advisory_lock($1) AS "locked!"
        "#,
        LOCK,
    )
    .fetch_one(&mut connection)
    .await?;
    if !locked {
        return Err(Error::Busy);
    }
    Ok(connection)
}

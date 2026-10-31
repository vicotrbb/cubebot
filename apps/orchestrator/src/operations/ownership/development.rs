use super::{Error, Owner, locked_connection};
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;
impl Owner {
    #[tracing::instrument(skip_all)]
    pub async fn database_only(pool: &PgPool) -> Result<Self, Error> {
        tokio::time::timeout(Duration::from_secs(30), Self::claim_development(pool))
            .await
            .map_err(|_| Error::DeadlineExceeded)?
    }
    async fn claim_development(pool: &PgPool) -> Result<Self, Error> {
        let mut connection = locked_connection(pool).await?;
        let established = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1
                FROM orchestrator_owner
            ) AS "established!"
            "#,
        )
        .fetch_one(&mut connection)
        .await?;
        if established {
            return Err(Error::InvalidConfiguration);
        }
        let instance = Uuid::new_v4();
        let generation = sqlx::query_scalar!(
            r#"
            INSERT INTO orchestrator_development_owner (singleton, instance_id, generation)
            VALUES (true, $1, 1)
            ON CONFLICT (singleton) DO UPDATE SET
                instance_id = EXCLUDED.instance_id,
                generation = orchestrator_development_owner.generation + 1,
                released_at = NULL
            WHERE orchestrator_development_owner.released_at IS NOT NULL
            RETURNING generation AS "generation!"
            "#,
            instance,
        )
        .fetch_optional(&mut connection)
        .await?
        .ok_or(Error::TerminationUnproven)?;
        Ok(Self {
            connection: Some(connection),
            instance,
            generation,
            table: "orchestrator_development_owner",
        })
    }
}

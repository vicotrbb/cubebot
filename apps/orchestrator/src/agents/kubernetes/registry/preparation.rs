//! Retain a bounded, verified server-defaulted manifest before the dispatch boundary.
mod defaults;
use super::{AttemptId, Error, Phase, Registry, ResourceKind, ResourceObject, Row};
use crate::agents::AgentId;

impl Registry {
    #[tracing::instrument(skip_all, fields(attempt = %id.0))]
    pub async fn record_prepared(
        &self,
        id: AttemptId,
        preview: &ResourceObject,
    ) -> Result<(), Error> {
        let value =
            serde_json::to_value(preview).map_err(|error| Error::Configuration(error.into()))?;
        if value.to_string().len() > 262144 {
            return Err(Error::InvalidConfiguration);
        }
        let mut transaction = self.transaction().await?;
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
        if expected.phase != Phase::Planned {
            return Err(Error::Conflict);
        }
        defaults::verify(&expected.intent, preview)?;
        if expected.prepared
            && serde_json::to_value(&expected.object)
                .map_err(|error| Error::Configuration(error.into()))?
                != value
        {
            return Err(Error::Conflict);
        }
        sqlx::query!(
            "
            UPDATE agent_resources
            SET manifest = $2,
                prepared = true,
                updated_at = clock_timestamp()
            WHERE id = $1
            ",
            id.0,
            value,
        )
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }
}

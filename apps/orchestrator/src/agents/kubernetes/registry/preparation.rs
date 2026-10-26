//! Retain a bounded, verified server-defaulted manifest before the dispatch boundary.
mod defaults;
use super::{AttemptId, Error, Phase, Registry, ResourceObject, Row};

impl Registry {
    pub async fn record_prepared(
        &self,
        id: AttemptId,
        preview: &ResourceObject,
    ) -> Result<(), Error> {
        let value = serde_json::to_value(preview).map_err(|_| Error::InvalidConfiguration)?;
        if value.to_string().len() > 262144 {
            return Err(Error::InvalidConfiguration);
        }
        let mut transaction = self.transaction().await?;
        let row: Row = sqlx::query_as("SELECT id,agent_id,kind,epoch,target_generation,definition_revision,phase,uid,blocked_reason,manifest,source_manifest,prepared FROM agent_resources WHERE id = $1 FOR UPDATE")
            .bind(id).fetch_optional(&mut *transaction).await?.ok_or(Error::Conflict)?;
        let expected = row.attempt()?;
        if expected.phase != Phase::Planned {
            return Err(Error::Conflict);
        }
        defaults::verify(&expected.intent, preview)?;
        if expected.prepared
            && serde_json::to_value(&expected.object).map_err(|_| Error::InvalidConfiguration)?
                != value
        {
            return Err(Error::Conflict);
        }
        sqlx::query("UPDATE agent_resources SET manifest = $2, prepared = true, updated_at = clock_timestamp() WHERE id = $1")
            .bind(id).bind(sqlx::types::Json(value)).execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(())
    }
}

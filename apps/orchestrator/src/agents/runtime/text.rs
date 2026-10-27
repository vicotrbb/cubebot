use super::{Error, RuntimeAccess, RuntimeIdentity};
use crate::{connections::Credentials, work::Work};
use cubebot_protocol::chat::InferenceGrant;

impl RuntimeIdentity {
    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.connections = Some(crate::connections::Connections::new(
            self.pool.clone(),
            credentials,
        ));
        self
    }

    pub async fn claim_text(&self, token: &str) -> Result<Option<InferenceGrant>, Error> {
        let actor = self.authenticate(token, RuntimeAccess::Active).await?;
        let mut tx = self.lock_runtime(&actor).await?;
        self.recheck(token, &actor, &mut tx).await?;
        let active: bool =
            sqlx::query_scalar("SELECT active FROM runtime_session_access WHERE attempt_id=$1")
                .bind(actor.attempt)
                .fetch_one(&mut *tx)
                .await?;
        if !active {
            return Err(Error::Unauthorized);
        }
        let connections = self.connections.as_ref();
        let grant = Work::new(self.pool.clone())
            .claim(&mut tx, &actor, connections, self.terminal_enabled)
            .await?;
        tx.commit().await?;
        Ok(grant)
    }
}

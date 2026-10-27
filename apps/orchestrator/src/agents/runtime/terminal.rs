//! Current authority is checked at admission and on every in-flight poll.
use super::{Error, RuntimeAccess, RuntimeIdentity};
use cubebot_protocol::terminal::{Authorization, Permit};

impl RuntimeIdentity {
    pub fn with_terminal(mut self, enabled: bool) -> Self {
        self.terminal_enabled = enabled;
        self
    }
    pub async fn authorize_terminal(
        &self,
        token: &str,
        input: &Authorization,
    ) -> Result<Permit, Error> {
        if !self.terminal_enabled {
            return Err(Error::Unauthorized);
        }
        input
            .invocation
            .validate()
            .map_err(|_| Error::InvalidConfiguration)?;
        let wire = cubebot_protocol::terminal::Request::Execute {
            invocation: input.invocation.clone(),
            not_after_millis: u64::MAX,
        };
        if serde_json::to_vec(&wire)
            .map_err(|_| Error::InvalidConfiguration)?
            .len()
            > cubebot_protocol::terminal::REQUEST_LIMIT
        {
            return Err(Error::InvalidConfiguration);
        }
        let actor = self.authenticate(token, RuntimeAccess::Active).await?;
        let mut tx = self.lock_runtime(&actor).await?;
        self.recheck(token, &actor, &mut tx).await?;
        let milliseconds = crate::work::terminal::authorize(&mut tx, &actor, input).await?;
        self.recheck(token, &actor, &mut tx).await?;
        tx.commit().await?;
        Ok(Permit { milliseconds })
    }
}

impl RuntimeIdentity {
    pub async fn pending_terminal(&self, token: &str) -> Result<Vec<Authorization>, Error> {
        if !self.terminal_enabled {
            return Ok(Vec::new());
        }
        let actor = self.authenticate(token, RuntimeAccess::Recovery).await?;
        let rows: Vec<(uuid::Uuid, serde_json::Value)> = sqlx::query_as("SELECT i.turn_id,i.request FROM terminal_invocations i JOIN chat_turns t ON t.id=i.turn_id WHERE t.conversation_id=$1 AND i.outcome IS NULL ORDER BY i.created_at,i.id LIMIT 8")
            .bind(actor.agent).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|(turn, request)| {
                Ok(Authorization {
                    turn,
                    invocation: serde_json::from_value(request)
                        .map_err(|_| Error::InvalidConfiguration)?,
                })
            })
            .collect()
    }
}

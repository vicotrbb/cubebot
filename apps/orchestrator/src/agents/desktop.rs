//! Desktop session records (ADRs 0039 and 0085): the viewer, start, end and each mode change
//! they made. Frames pass through the orchestrator and are never stored.
use super::{AgentId, Agents};
use crate::identity::IdentityError;
use cubebot_protocol::desktop::Mode;
use uuid::Uuid;

impl Agents {
    /// The caller has authorized `viewer` to watch the agent's desktop.
    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub async fn start_desktop(&self, agent: AgentId, viewer: Uuid) -> Result<Uuid, IdentityError> {
        Ok(sqlx::query_scalar!(
            "
            INSERT INTO desktop_sessions (agent_id, viewer_id)
            VALUES ($1, $2)
            RETURNING id
            ",
            agent.0,
            viewer,
        )
        .fetch_one(&self.pool)
        .await?)
    }

    #[tracing::instrument(skip_all, fields(session = %session))]
    pub async fn record_desktop_mode(
        &self,
        session: Uuid,
        mode: Mode,
    ) -> Result<(), IdentityError> {
        // The protocol's serialized names.
        let mode = match mode {
            Mode::Agent => "agent",
            Mode::Watch => "watch",
            Mode::Human => "human",
        };
        sqlx::query!(
            "
            UPDATE desktop_sessions
            SET mode_changes = mode_changes
                || jsonb_build_object('mode', $2::text, 'at', clock_timestamp())
            WHERE id = $1
              AND ended_at IS NULL
            ",
            session,
            mode,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(session = %session))]
    pub async fn end_desktop(&self, session: Uuid) -> Result<(), IdentityError> {
        sqlx::query!(
            "
            UPDATE desktop_sessions
            SET ended_at = clock_timestamp()
            WHERE id = $1
              AND ended_at IS NULL
            ",
            session,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

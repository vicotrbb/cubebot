use super::{MeetingAgentMode, Meetings};
use crate::identity::{Actor, IdentityError, administrator};
use serde::{Deserialize, Serialize};

/// The installation's Meetings tab. A meeting copies these when it is created.
#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeetingSettings {
    /// A new link holds guests in a lobby until the host admits them.
    pub lobby_default: bool,
    /// Tell everyone in a meeting that agents listen and a transcript is kept.
    pub consent_notice: bool,
    /// How an invited agent takes part unless the invitation says otherwise.
    pub agent_mode: MeetingAgentMode,
}

impl Meetings {
    #[tracing::instrument(skip_all)]
    pub async fn settings(&self) -> Result<MeetingSettings, IdentityError> {
        Ok(sqlx::query_as!(
            MeetingSettings,
            r#"
            SELECT lobby_default, consent_notice, agent_mode AS "agent_mode: MeetingAgentMode"
            FROM meeting_settings
            "#,
        )
        .fetch_one(&self.pool)
        .await?)
    }

    /// Administrators only.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn save_settings(
        &self,
        actor: &Actor,
        input: &MeetingSettings,
    ) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        administrator(&mut *tx, actor.id).await?;
        sqlx::query!(
            "UPDATE meeting_settings SET lobby_default=$1, consent_notice=$2, agent_mode=$3",
            input.lobby_default,
            input.consent_notice,
            input.agent_mode.as_str(),
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

use super::{AgentMode, Meetings};
use crate::identity::{Actor, IdentityError};
use serde::{Deserialize, Serialize};

#[derive(Serialize, utoipa::ToSchema)]
pub struct MeetingSettings {
    pub agents_join: bool,
    pub agent_screens: bool,
    pub guests: bool,
    pub max_participants: i32,
    pub agent_mode: AgentMode,
    pub public_origin: Option<String>,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveMeetingSettings {
    pub agents_join: bool,
    pub agent_screens: bool,
    pub guests: bool,
    pub max_participants: i32,
    pub agent_mode: AgentMode,
}

impl Meetings {
    #[tracing::instrument(skip_all)]
    pub async fn settings(&self) -> Result<MeetingSettings, IdentityError> {
        let row = sqlx::query!(
            r#"
            SELECT agents_join, agent_screens, guests, max_participants,
                agent_mode AS "agent_mode: AgentMode"
            FROM meeting_settings
            "#,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(MeetingSettings {
            agents_join: row.agents_join,
            agent_screens: row.agent_screens,
            guests: row.guests,
            max_participants: row.max_participants,
            agent_mode: row.agent_mode,
            public_origin: self.origin.clone(),
        })
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn save_settings(
        &self,
        actor: &Actor,
        input: &SaveMeetingSettings,
    ) -> Result<(), IdentityError> {
        if !(2..=24).contains(&input.max_participants) {
            return Err(IdentityError::InvalidInput);
        }
        let mut tx = self.pool.begin().await?;
        let administrator = sqlx::query_scalar!(
            r#"SELECT role='administrator' AS "administrator!" FROM human_accounts WHERE id=$1 AND active FOR SHARE"#,
            actor.id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if administrator != Some(true) {
            return Err(IdentityError::Forbidden);
        }
        sqlx::query!(
            "
            UPDATE meeting_settings
            SET agents_join=$1, agent_screens=$2, guests=$3, max_participants=$4, agent_mode=$5
            ",
            input.agents_join,
            input.agent_screens,
            input.guests,
            input.max_participants,
            input.agent_mode.as_str(),
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

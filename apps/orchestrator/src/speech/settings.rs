use super::{Provider, Voice};
use crate::{
    agents::AgentId,
    connections::{Connections, key_valid},
    identity::{Actor, IdentityError},
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub stt: Provider,
    pub tts: Provider,
    pub voice: Voice,
}

/// The installation's speech settings; the key itself is never returned.
#[derive(Serialize, utoipa::ToSchema)]
pub struct SpeechSettings {
    pub stt_provider: Provider,
    pub tts_provider: Provider,
    pub voice: Voice,
    pub elevenlabs_configured: bool,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveSpeech {
    pub stt_provider: Provider,
    pub tts_provider: Provider,
    pub voice: Voice,
    /// `None` keeps the stored ElevenLabs key.
    #[serde(default)]
    pub elevenlabs_key: Option<String>,
    /// Removes the stored ElevenLabs key.
    #[serde(default)]
    pub forget_elevenlabs_key: bool,
}

/// An agent's overrides; `None` inherits the installation's setting.
#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSpeech {
    pub stt_provider: Option<Provider>,
    pub tts_provider: Option<Provider>,
    pub voice: Option<Voice>,
}

#[tracing::instrument(skip_all)]
pub async fn settings(pool: &PgPool) -> Result<SpeechSettings, IdentityError> {
    Ok(sqlx::query_as!(
        SpeechSettings,
        r#"
        SELECT stt_provider AS "stt_provider: Provider", tts_provider AS "tts_provider: Provider",
            voice AS "voice: Voice", elevenlabs_credential IS NOT NULL AS "elevenlabs_configured!"
        FROM speech_settings
        "#,
    )
    .fetch_one(pool)
    .await?)
}

#[tracing::instrument(skip_all, fields(actor = %actor.id))]
pub async fn save(
    pool: &PgPool,
    connections: Option<&Connections>,
    actor: &Actor,
    input: &SaveSpeech,
) -> Result<(), IdentityError> {
    if !input.elevenlabs_key.as_deref().is_none_or(key_valid) {
        return Err(IdentityError::InvalidInput);
    }
    let mut tx = pool.begin().await?;
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
        "UPDATE speech_settings SET stt_provider=$1, tts_provider=$2, voice=$3",
        input.stt_provider.as_str(),
        input.tts_provider.as_str(),
        input.voice.as_str(),
    )
    .execute(&mut *tx)
    .await?;
    if input.forget_elevenlabs_key || input.elevenlabs_key.is_some() {
        let connections = connections.ok_or(IdentityError::CredentialUnavailable(None))?;
        let key = input.elevenlabs_key.as_deref();
        connections.store_speech_key(&mut tx, key).await?;
    }
    tx.commit().await?;
    Ok(())
}

#[tracing::instrument(skip_all, fields(agent = %agent.0))]
pub async fn save_agent(
    pool: &PgPool,
    agent: AgentId,
    input: &AgentSpeech,
) -> Result<(), IdentityError> {
    let changed = sqlx::query!(
        "UPDATE agents SET stt_provider=$2, tts_provider=$3, voice=$4 WHERE id=$1",
        agent.0,
        input.stt_provider.map(Provider::as_str),
        input.tts_provider.map(Provider::as_str),
        input.voice.map(Voice::as_str),
    )
    .execute(pool)
    .await?;
    if changed.rows_affected() == 0 {
        return Err(IdentityError::NotFound);
    }
    Ok(())
}

#[tracing::instrument(skip_all, fields(agent = %agent.0))]
pub async fn route(pool: &PgPool, agent: AgentId) -> Result<Route, IdentityError> {
    let row = sqlx::query!(
        r#"
        SELECT COALESCE(a.stt_provider, s.stt_provider) AS "stt!: Provider",
            COALESCE(a.tts_provider, s.tts_provider) AS "tts!: Provider",
            COALESCE(a.voice, s.voice) AS "voice!: Voice"
        FROM agents a
        CROSS JOIN speech_settings s
        WHERE a.id=$1
        "#,
        agent.0,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(IdentityError::NotFound)?;
    Ok(Route {
        stt: row.stt,
        tts: row.tts,
        voice: row.voice,
    })
}

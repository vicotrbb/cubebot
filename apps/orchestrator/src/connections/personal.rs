//! People's own provider keys. An agent whose chosen provider its owner brought a key for uses
//! that key, and lists models with it, before any installation key.

use super::{Connections, Inference};
use crate::{agents::AgentId, identity::IdentityError};
use cubebot_protocol::runtime::ProviderKind;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PersonalProvider {
    pub provider: ProviderKind,
    pub context_window: Option<i64>,
    pub updated_at: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SavePersonalProvider {
    pub api_key: String,
    #[serde(default)]
    pub context_window: Option<i64>,
}

fn context(owner: Uuid, provider: ProviderKind) -> Vec<u8> {
    format!("cubebot.personal-provider:{owner}:{}", provider.as_str()).into_bytes()
}

impl Connections {
    #[tracing::instrument(skip_all, fields(owner = %owner))]
    pub async fn personal_providers(
        &self,
        owner: Uuid,
    ) -> Result<Vec<PersonalProvider>, IdentityError> {
        let rows = sqlx::query!(
            r#"SELECT provider, context_window, (to_jsonb(updated_at) #>> '{}') AS "updated_at!" FROM personal_providers WHERE owner = $1 ORDER BY provider"#,
            owner,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                Some(PersonalProvider {
                    provider: ProviderKind::parse(&row.provider)?,
                    context_window: row.context_window,
                    updated_at: row.updated_at,
                })
            })
            .collect())
    }

    /// Plans sign in through the installation; only pay-as-you-go keys are personal here.
    #[tracing::instrument(skip_all, fields(owner = %owner, provider = provider.as_str()))]
    pub async fn save_personal_provider(
        &self,
        owner: Uuid,
        provider: ProviderKind,
        input: &SavePersonalProvider,
    ) -> Result<(), IdentityError> {
        let key = input.api_key.trim();
        let window_ok = input
            .context_window
            .is_none_or(|window| (1024..=100_000_000).contains(&window));
        if provider.subscription()
            || key.is_empty()
            || key.len() > 4096
            || key.chars().any(char::is_control)
            || !window_ok
        {
            return Err(IdentityError::InvalidInput);
        }
        let sealed = self
            .credentials
            .encrypt(key.as_bytes(), &context(owner, provider))
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
        sqlx::query!(
            r#"
            INSERT INTO personal_providers (owner, provider, credential, context_window)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (owner, provider) DO UPDATE
            SET credential = EXCLUDED.credential, context_window = EXCLUDED.context_window, updated_at = clock_timestamp()
            "#,
            owner,
            provider.as_str(),
            sealed,
            input.context_window,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(owner = %owner, provider = provider.as_str()))]
    pub async fn forget_personal_provider(
        &self,
        owner: Uuid,
        provider: ProviderKind,
    ) -> Result<(), IdentityError> {
        sqlx::query_scalar!(
            "DELETE FROM personal_providers WHERE owner = $1 AND provider = $2 RETURNING owner",
            owner,
            provider.as_str(),
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::NotFound)?;
        Ok(())
    }

    /// The agent's owner's key for `provider`, while it may serve (`usable_personal_providers`).
    pub(super) async fn owner_key(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        agent: AgentId,
        provider: ProviderKind,
    ) -> Result<Option<(String, Option<i64>)>, IdentityError> {
        let row = sqlx::query!(
            r#"
            SELECT p.owner AS "owner!", p.credential AS "credential!", p.context_window
            FROM agents g
            JOIN usable_personal_providers p ON p.owner = g.created_by AND p.provider = $2
            WHERE g.id = $1
            "#,
            agent.0,
            provider.as_str(),
        )
        .fetch_optional(&mut **tx)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let secret = self
            .credentials
            .decrypt(&row.credential, &context(row.owner, provider))
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
        let key = String::from_utf8(secret).map_err(|error| {
            IdentityError::CredentialUnavailable(Some(error.utf8_error().into()))
        })?;
        Ok(Some((key, row.context_window)))
    }

    /// An agent whose chosen model is on a provider its owner keyed runs on that key.
    pub(super) async fn owner_inference(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        agent: AgentId,
    ) -> Result<Option<Inference>, IdentityError> {
        let Some(chosen) = super::agent_model::chosen_model(tx, agent).await? else {
            return Ok(None);
        };
        let Some((api_key, window)) = self.owner_key(tx, agent, chosen.provider).await? else {
            return Ok(None);
        };
        Ok(Some(Inference {
            provider: chosen.provider,
            model: chosen.model,
            api_key,
            api_key_expires_at: None,
            context_window: chosen
                .context_window
                .or(window)
                .and_then(|tokens| u64::try_from(tokens).ok()),
            effort: None,
        }))
    }
}

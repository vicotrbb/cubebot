use super::{Connections, administrator, key_valid};
use crate::identity::{Actor, IdentityError};
use cubebot_protocol::runtime::ProviderKind;
use cubebot_providers::{Anthropic, Gemini, Grok, OpenAi, OpenRouter, Provider};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ListModels {
    pub provider: ProviderKind,
    /// The key to list with; without one, the saved key when it belongs to `provider`.
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ModelChoice {
    pub id: String,
    /// Reported by the provider; OpenAI reports none.
    pub context_window: Option<u64>,
    /// Whether it can call tools; a model that cannot only talks.
    pub tools: bool,
}

pub enum ModelsError {
    Identity(IdentityError),
    Provider(cubebot_providers::Error),
}

impl From<IdentityError> for ModelsError {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error)
    }
}

pub(crate) fn configured<P>(
    new: fn() -> Result<P, cubebot_providers::Error>,
    loopback: fn(&str) -> Result<P, cubebot_providers::Error>,
) -> Result<P, cubebot_providers::Error> {
    match std::env::var("CUBEBOT_TEST_PROVIDER_ORIGIN") {
        Ok(origin) => loopback(&origin),
        Err(std::env::VarError::NotPresent) => new(),
        Err(error) => Err(cubebot_providers::Error::Configuration(Some(error.into()))),
    }
}

impl Connections {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn models(
        &self,
        actor: &Actor,
        input: &ListModels,
    ) -> Result<Vec<ModelChoice>, ModelsError> {
        let key = self.listing_key(actor, input).await?;
        let models = match input.provider {
            ProviderKind::Openai => {
                configured(OpenAi::new, OpenAi::for_loopback_tests)
                    .map_err(ModelsError::Provider)?
                    .models(&key)
                    .await
            }
            ProviderKind::Anthropic => {
                configured(Anthropic::new, Anthropic::for_loopback_tests)
                    .map_err(ModelsError::Provider)?
                    .models(&key)
                    .await
            }
            ProviderKind::Openrouter => {
                configured(OpenRouter::new, OpenRouter::for_loopback_tests)
                    .map_err(ModelsError::Provider)?
                    .models(&key)
                    .await
            }
            ProviderKind::Gemini => {
                configured(Gemini::new, Gemini::for_loopback_tests)
                    .map_err(ModelsError::Provider)?
                    .models(&key)
                    .await
            }
            ProviderKind::Grok => {
                configured(Grok::new, Grok::for_loopback_tests)
                    .map_err(ModelsError::Provider)?
                    .models(&key)
                    .await
            }
        }
        .map_err(ModelsError::Provider)?;
        Ok(models
            .into_iter()
            .map(|model| ModelChoice {
                id: model.id,
                context_window: model.context_window,
                tools: model.tools,
            })
            .collect())
    }

    async fn listing_key(
        &self,
        actor: &Actor,
        input: &ListModels,
    ) -> Result<String, IdentityError> {
        let mut tx = self.pool.begin().await?;
        administrator(&mut tx, actor).await?;
        if let Some(key) = &input.api_key {
            return if key_valid(key) {
                Ok(key.clone())
            } else {
                Err(IdentityError::InvalidInput)
            };
        }
        let saved = self
            .saved_key(&mut tx, input.provider)
            .await
            .map_err(|error| match error {
                IdentityError::InvalidInput => IdentityError::NotFound,
                error => error,
            })?;
        tx.commit().await?;
        Ok(saved)
    }
}

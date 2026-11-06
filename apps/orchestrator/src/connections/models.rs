use super::{Connections, For, administrator, key_valid};
use crate::identity::{Actor, IdentityError};
use cubebot_protocol::runtime::ProviderKind;
use cubebot_providers::{Anthropic, Gemini, Grok, OpenAi, OpenRouter, Provider};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ListModels {
    pub provider: ProviderKind,
    /// The key to list with; without one, the saved key when it belongs to `provider`, or for a
    /// subscription, the caller's own signed-in plan.
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

async fn listed(
    provider: ProviderKind,
    key: &str,
) -> Result<Vec<cubebot_providers::Model>, cubebot_providers::Error> {
    match provider {
        ProviderKind::Openai => {
            list(configured(OpenAi::new, OpenAi::for_loopback_tests), key).await
        }
        ProviderKind::Anthropic => {
            list(
                configured(Anthropic::new, Anthropic::for_loopback_tests),
                key,
            )
            .await
        }
        ProviderKind::Openrouter => {
            list(
                configured(OpenRouter::new, OpenRouter::for_loopback_tests),
                key,
            )
            .await
        }
        ProviderKind::Gemini => {
            list(configured(Gemini::new, Gemini::for_loopback_tests), key).await
        }
        ProviderKind::Grok => list(configured(Grok::new, Grok::for_loopback_tests), key).await,
        ProviderKind::Chatgpt => {
            list(
                configured(OpenAi::chatgpt, OpenAi::chatgpt_for_loopback_tests),
                key,
            )
            .await
        }
        ProviderKind::Supergrok => {
            list(
                configured(OpenAi::supergrok, OpenAi::supergrok_for_loopback_tests),
                key,
            )
            .await
        }
        ProviderKind::Minimax | ProviderKind::MinimaxPlan => {
            list(
                configured(Anthropic::minimax, Anthropic::minimax_for_loopback_tests),
                key,
            )
            .await
        }
        ProviderKind::MinimaxCn | ProviderKind::MinimaxPlanCn => {
            list(
                configured(Anthropic::minimax_cn, Anthropic::minimax_for_loopback_tests),
                key,
            )
            .await
        }
    }
}

async fn list<P: Provider>(
    provider: Result<P, cubebot_providers::Error>,
    key: &str,
) -> Result<Vec<cubebot_providers::Model>, cubebot_providers::Error> {
    provider?.models(key).await
}

impl Connections {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn models(
        &self,
        actor: &Actor,
        input: &ListModels,
    ) -> Result<Vec<ModelChoice>, ModelsError> {
        let key = self.listing_key(actor, input).await?;
        let models = listed(input.provider, &key)
            .await
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
        if input.provider.subscription() {
            if input.api_key.is_some() {
                return Err(IdentityError::InvalidInput);
            }
            self.refresh_due(input.provider).await?;
            let (token, _) = self
                .subscription_token(&mut tx, input.provider, For::Person(actor.id))
                .await
                .map_err(|_| IdentityError::NotFound)?;
            tx.commit().await?;
            return Ok(token);
        }
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

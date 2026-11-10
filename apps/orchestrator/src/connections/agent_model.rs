use super::{Connections, For, Inference, ModelChoice, ModelsError, models::listed};
use crate::{agents::AgentId, identity::IdentityError};
use cubebot_protocol::runtime::ProviderKind;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelRef {
    pub provider: ProviderKind,
    pub model: String,
    #[serde(default)]
    pub context_window: Option<i64>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct AgentModel {
    pub chosen: Option<ModelRef>,
    pub default: Option<ModelRef>,
    pub providers: Vec<ProviderKind>,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAgentModel {
    pub chosen: Option<ModelRef>,
}

impl ModelRef {
    fn valid(&self) -> bool {
        let model = self.model.trim();
        !model.is_empty()
            && model.len() <= 200
            && !model.chars().any(char::is_control)
            && self
                .context_window
                .is_none_or(|window| (1024..=100_000_000).contains(&window))
    }
}

impl Connections {
    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub async fn agent_model(&self, agent: AgentId) -> Result<AgentModel, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let model = described(&mut tx, agent).await?;
        tx.commit().await?;
        Ok(model)
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub async fn set_agent_model(
        &self,
        agent: AgentId,
        input: &SetAgentModel,
    ) -> Result<AgentModel, IdentityError> {
        let mut tx = self.pool.begin().await?;
        if let Some(chosen) = &input.chosen
            && (!chosen.valid() || !choosable(&mut tx).await?.contains(&chosen.provider))
        {
            return Err(IdentityError::InvalidInput);
        }
        let chosen = input.chosen.as_ref();
        let updated = sqlx::query!(
            "UPDATE agents SET model_provider=$2, model=$3, model_context_window=$4 WHERE id=$1",
            agent.0,
            chosen.map(|chosen| chosen.provider.as_str()),
            chosen.map(|chosen| chosen.model.trim()),
            chosen.and_then(|chosen| chosen.context_window),
        )
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(IdentityError::NotFound);
        }
        let model = described(&mut tx, agent).await?;
        tx.commit().await?;
        Ok(model)
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0, provider = provider.as_str()))]
    pub async fn agent_models(
        &self,
        agent: AgentId,
        provider: ProviderKind,
    ) -> Result<Vec<ModelChoice>, ModelsError> {
        let mut tx = self.pool.begin().await.map_err(IdentityError::from)?;
        let key = self.listing_material(&mut tx, agent, provider).await?;
        tx.commit().await.map_err(IdentityError::from)?;
        let models = listed(provider, &key)
            .await
            .map_err(ModelsError::Provider)?;
        Ok(models.into_iter().map(ModelChoice::from).collect())
    }

    async fn listing_material(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        agent: AgentId,
        provider: ProviderKind,
    ) -> Result<String, IdentityError> {
        if !choosable(tx).await?.contains(&provider) {
            return Err(IdentityError::NotFound);
        }
        if let Some((key, _)) = self.saved_material(tx, provider).await? {
            return Ok(key);
        }
        Ok(self.connection(tx, For::Agent(agent)).await?.api_key)
    }

    pub(super) async fn chosen(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        agent: AgentId,
        base: Inference,
    ) -> Result<Inference, IdentityError> {
        let Some(chosen) = chosen_model(tx, agent).await? else {
            return Ok(base);
        };
        if chosen.provider == base.provider {
            return Ok(Inference {
                model: chosen.model,
                context_window: window(chosen.context_window).or(base.context_window),
                ..base
            });
        }
        let Some((api_key, saved_window)) = self.saved_material(tx, chosen.provider).await? else {
            return Ok(base);
        };
        Ok(Inference {
            provider: chosen.provider,
            model: chosen.model,
            api_key,
            api_key_expires_at: None,
            context_window: window(chosen.context_window.or(saved_window)),
        })
    }
}

fn window(tokens: Option<i64>) -> Option<u64> {
    tokens.and_then(|tokens| u64::try_from(tokens).ok())
}

async fn chosen_model(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
) -> Result<Option<ModelRef>, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT model_provider, model, model_context_window FROM agents WHERE id=$1",
        agent.0,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.and_then(|row| {
        Some(ModelRef {
            provider: ProviderKind::parse(&row.model_provider?)?,
            model: row.model?,
            context_window: row.model_context_window,
        })
    }))
}

async fn described(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
) -> Result<AgentModel, IdentityError> {
    let exists = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM agents WHERE id=$1) AS "exists!""#,
        agent.0
    )
    .fetch_one(&mut **tx)
    .await?;
    if !exists {
        return Err(IdentityError::NotFound);
    }
    let default = sqlx::query!("SELECT provider, model, context_window FROM provider_connection")
        .fetch_optional(&mut **tx)
        .await?
        .and_then(|row| {
            Some(ModelRef {
                provider: ProviderKind::parse(&row.provider)?,
                model: row.model,
                context_window: row.context_window,
            })
        });
    Ok(AgentModel {
        chosen: chosen_model(tx, agent).await?,
        default,
        providers: choosable(tx).await?,
    })
}

async fn choosable(tx: &mut Transaction<'_, Postgres>) -> Result<Vec<ProviderKind>, sqlx::Error> {
    let names = sqlx::query_scalar!(
        r#"
        SELECT provider AS "provider!" FROM (
            SELECT provider, 0 AS position FROM provider_connection
            UNION ALL
            SELECT provider, 1 FROM provider_saved WHERE provider NOT IN (SELECT provider FROM provider_connection)
        ) providers
        ORDER BY position, provider
        "#
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(names
        .iter()
        .filter_map(|name| ProviderKind::parse(name))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(model: &str, context_window: Option<i64>) -> ModelRef {
        ModelRef {
            provider: ProviderKind::Anthropic,
            model: model.into(),
            context_window,
        }
    }

    #[test]
    fn a_choice_names_one_model_on_one_line_with_a_sane_window() {
        assert!(choice("claude-sonnet-5-5", None).valid());
        assert!(choice("claude-sonnet-5-5", Some(200_000)).valid());
        assert!(!choice("  ", None).valid());
        assert!(!choice("two\nlines", None).valid());
        assert!(!choice(&"m".repeat(201), None).valid());
        assert!(!choice("claude-sonnet-5-5", Some(10)).valid());
    }
}

use super::{
    Connections, For, Inference, ModelChoice, ModelsError, effort_offered, given, models::listed,
};
use crate::{agents::AgentId, identity::IdentityError};
use cubebot_protocol::runtime::{Effort, ProviderKind};
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
    pub effort: Option<Effort>,
    pub provider_efforts: Vec<ProviderEffort>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ProviderEffort {
    pub provider: ProviderKind,
    pub effort: Option<Effort>,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAgentModel {
    pub chosen: Option<ModelRef>,
    #[serde(
        default,
        deserialize_with = "given",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<Effort>)]
    pub effort: Option<Option<Effort>>,
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
        let providers = choosable(&mut tx, agent).await?;
        if let Some(chosen) = &input.chosen
            && (!chosen.valid() || !providers.contains(&chosen.provider))
        {
            return Err(IdentityError::InvalidInput);
        }
        let chosen = input.chosen.as_ref();
        let provider = chosen
            .map(|chosen| chosen.provider)
            .or(providers.first().copied());
        if let Some(Some(effort)) = input.effort
            && !provider.is_some_and(|p| effort_offered(p, Some(effort)))
        {
            return Err(IdentityError::InvalidInput);
        }
        let updated = sqlx::query!(
            "UPDATE agents SET model_provider=$2, model=$3, model_context_window=$4,
                model_effort=CASE WHEN $6 THEN $5 ELSE model_effort END WHERE id=$1",
            agent.0,
            chosen.map(|chosen| chosen.provider.as_str()),
            chosen.map(|chosen| chosen.model.trim()),
            chosen.and_then(|chosen| chosen.context_window),
            input.effort.flatten().map(Effort::as_str),
            input.effort.is_some(),
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
        if !choosable(tx, agent).await?.contains(&provider) {
            return Err(IdentityError::NotFound);
        }
        if let Some((key, _)) = self.owner_key(tx, agent, provider).await? {
            return Ok(key);
        }
        if let Some((key, ..)) = self.saved_material(tx, provider).await? {
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
        let inference = self.chosen_inference(tx, agent, base).await?;
        self.with_effort(tx, agent, inference).await
    }

    /// The agent's own effort, else the provider's, when the provider offers it.
    pub(super) async fn with_effort(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        agent: AgentId,
        inference: Inference,
    ) -> Result<Inference, IdentityError> {
        let own = own_effort(tx, agent).await?;
        let effort = own.or(inference.effort);
        Ok(Inference {
            effort: effort.filter(|_| effort_offered(inference.provider, effort)),
            ..inference
        })
    }

    async fn chosen_inference(
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
        let Some((api_key, saved_window, effort)) =
            self.saved_material(tx, chosen.provider).await?
        else {
            return Ok(base);
        };
        Ok(Inference {
            provider: chosen.provider,
            model: chosen.model,
            api_key,
            api_key_expires_at: None,
            context_window: window(chosen.context_window.or(saved_window)),
            effort,
        })
    }
}

async fn own_effort(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
) -> Result<Option<Effort>, sqlx::Error> {
    let effort = sqlx::query_scalar!("SELECT model_effort FROM agents WHERE id=$1", agent.0)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(effort.flatten().as_deref().and_then(Effort::parse))
}

async fn provider_efforts(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<Vec<ProviderEffort>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT provider AS "provider!", reasoning_effort FROM provider_connection
        UNION ALL
        SELECT provider, reasoning_effort FROM provider_saved
            WHERE provider NOT IN (SELECT provider FROM provider_connection)
        "#
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(ProviderEffort {
                provider: ProviderKind::parse(&row.provider)?,
                effort: row.reasoning_effort.as_deref().and_then(Effort::parse),
            })
        })
        .collect())
}

fn window(tokens: Option<i64>) -> Option<u64> {
    tokens.and_then(|tokens| u64::try_from(tokens).ok())
}

pub(super) async fn chosen_model(
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
        providers: choosable(tx, agent).await?,
        effort: own_effort(tx, agent).await?,
        provider_efforts: provider_efforts(tx).await?,
    })
}

/// The installation's providers, then the agent's owner's own ones.
async fn choosable(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
) -> Result<Vec<ProviderKind>, sqlx::Error> {
    let names = sqlx::query_scalar!(
        r#"
        SELECT provider AS "provider!" FROM (
            SELECT provider, 0 AS position FROM provider_connection
            UNION ALL
            SELECT provider, 1 FROM provider_saved WHERE provider NOT IN (SELECT provider FROM provider_connection)
            UNION ALL
            SELECT p.provider, 2 FROM usable_personal_providers p JOIN agents g ON g.created_by = p.owner AND g.id = $1
            WHERE p.provider NOT IN (SELECT provider FROM provider_connection UNION SELECT provider FROM provider_saved)
        ) providers
        ORDER BY position, provider
        "#,
        agent.0,
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

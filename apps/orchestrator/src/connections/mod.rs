//! Installation-managed inference configuration and encrypted credential storage.
mod credentials;
pub use credentials::{CredentialError, Credentials};

use crate::identity::{Actor, IdentityError};
use cubebot_protocol::runtime::ProviderKind;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Clone)]
pub struct Connections {
    pool: PgPool,
    credentials: Credentials,
}

#[derive(Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigureProvider {
    pub request_id: Uuid,
    pub expected_revision: i64,
    #[serde(default = "openai")]
    pub provider: ProviderKind,
    pub model: String,
    /// Model window in tokens. Required for OpenAI, which cannot report it; Anthropic
    /// reports its own when this is absent.
    #[serde(default)]
    pub context_window: Option<i64>,
    pub api_key: String,
    #[serde(default)]
    pub access: Option<ProviderAccess>,
}
fn openai() -> ProviderKind {
    ProviderKind::Openai
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ProviderSummary {
    pub provider: ProviderKind,
    pub model: String,
    pub context_window: Option<i64>,
    pub revision: i64,
    #[schema(value_type = ProviderAccess)]
    pub access: sqlx::types::Json<ProviderAccess>,
}
type Access = sqlx::types::Json<ProviderAccess>;
type SummaryRow = (String, String, Option<i64>, i64, Access);
fn summary(
    (provider, model, context_window, revision, access): SummaryRow,
) -> Result<ProviderSummary, IdentityError> {
    Ok(ProviderSummary {
        provider: ProviderKind::parse(&provider).ok_or(IdentityError::InvalidInput)?,
        model,
        context_window,
        revision,
        access,
    })
}

/// What a runtime needs for one turn. The key is never logged.
pub(crate) struct Inference {
    pub provider: ProviderKind,
    pub model: String,
    pub api_key: String,
    pub context_window: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProviderAccess {
    #[default]
    AllAgents,
    ExplicitGrants {
        agents: Vec<Uuid>,
    },
}

impl Connections {
    pub fn new(pool: PgPool, credentials: Credentials) -> Self {
        Self { pool, credentials }
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn inspect(&self, actor: &Actor) -> Result<Option<ProviderSummary>, IdentityError> {
        let mut transaction = self.pool.begin().await?;
        administrator(&mut transaction, actor).await?;
        let row = sqlx::query!(
            r#"
            SELECT
                provider AS "provider!",
                model AS "model!",
                context_window AS "context_window?",
                revision AS "revision!",
                access AS "access!: Access"
            FROM provider_connection
            "#,
        )
        .fetch_optional(&mut *transaction)
        .await?;
        transaction.commit().await?;
        row.map(|row| {
            summary((
                row.provider,
                row.model,
                row.context_window,
                row.revision,
                row.access,
            ))
        })
        .transpose()
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id))]
    pub async fn configure(
        &self,
        actor: &Actor,
        input: &ConfigureProvider,
    ) -> Result<ProviderSummary, IdentityError> {
        validate(input)?;
        let digest = Sha256::digest(
            serde_json::to_vec(input).map_err(|error| IdentityError::Invalid(error.into()))?,
        )
        .to_vec();
        let mut transaction = self.pool.begin().await?;
        administrator(&mut transaction, actor).await?;
        sqlx::query!(
            "
            SELECT singleton
            FROM installation
            FOR UPDATE
            ",
        )
        .fetch_all(&mut *transaction)
        .await?;
        if let Some(previous) = previous(&mut transaction, actor, input.request_id, &digest).await?
        {
            return Ok(previous);
        }
        let revision = self
            .next_revision(&mut transaction, input.expected_revision)
            .await?;
        let access = selected_access(&mut transaction, input.access.as_ref()).await?;
        let context = credential_context(&mut transaction, revision).await?;
        let encrypted = self
            .credentials
            .encrypt(input.api_key.as_bytes(), &context)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
        sqlx::query!(
            "
            INSERT INTO provider_connection
                (model, credential, key_version, revision, updated_by, access, provider, context_window)
            VALUES ($1, $2, 1, $3, $4, $5, $6, $7)
            ON CONFLICT (singleton) DO UPDATE SET
                model = EXCLUDED.model,
                credential = EXCLUDED.credential,
                revision = EXCLUDED.revision,
                updated_by = EXCLUDED.updated_by,
                updated_at = clock_timestamp(),
                access = EXCLUDED.access,
                provider = EXCLUDED.provider,
                context_window = EXCLUDED.context_window
            ",
            &input.model,
            encrypted,
            revision,
            actor.id,
            sqlx::types::Json(&access) as _,
            input.provider.as_str(),
            input.context_window,
        )
        .execute(&mut *transaction)
        .await?;
        sqlx::query!(
            "
            INSERT INTO provider_commands
                (actor_id, request_id, input_digest, model, revision, access, provider, context_window)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ",
            actor.id,
            input.request_id,
            digest,
            &input.model,
            revision,
            sqlx::types::Json(&access) as _,
            input.provider.as_str(),
            input.context_window,
        )
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(ProviderSummary {
            provider: input.provider,
            model: input.model.clone(),
            context_window: input.context_window,
            revision,
            access: sqlx::types::Json(access),
        })
    }
    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub(crate) async fn inference(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        agent: crate::agents::AgentId,
    ) -> Result<Inference, IdentityError> {
        self.connection(tx, Some(agent)).await
    }

    /// The installation's connection for the orchestrator's own duties, such as room
    /// coordination, regardless of which agents may use it.
    #[tracing::instrument(skip_all)]
    pub(crate) async fn coordinator(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Inference, IdentityError> {
        self.connection(tx, None).await
    }

    async fn connection(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        agent: Option<crate::agents::AgentId>,
    ) -> Result<Inference, IdentityError> {
        let row = sqlx::query!(
            r#"
            SELECT
                provider AS "provider!",
                model AS "model!",
                credential AS "credential!",
                revision AS "revision!",
                context_window AS "context_window?"
            FROM provider_connection
            WHERE $1::uuid IS NULL
                OR access->>'mode' = 'all_agents'
                OR (access->>'mode' = 'explicit_grants' AND access->'agents' @> jsonb_build_array($1::uuid))
            "#,
            agent.map(|agent| agent.0),
        )
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(IdentityError::CredentialUnavailable(None))?;
        let context = credential_context(tx, row.revision).await?;
        let secret = self
            .credentials
            .decrypt(&row.credential, &context)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
        // Only the UTF-8 position survives; FromUtf8Error would keep the decrypted key bytes.
        let api_key = String::from_utf8(secret).map_err(|error| {
            IdentityError::CredentialUnavailable(Some(error.utf8_error().into()))
        })?;
        Ok(Inference {
            provider: ProviderKind::parse(&row.provider)
                .ok_or(IdentityError::CredentialUnavailable(None))?,
            model: row.model,
            api_key,
            context_window: row.context_window.map(|window| window as u64),
        })
    }

    async fn next_revision(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        expected: i64,
    ) -> Result<i64, IdentityError> {
        let current = sqlx::query!(
            r#"
            SELECT revision AS "revision!", credential AS "credential!"
            FROM provider_connection
            "#,
        )
        .fetch_optional(&mut **transaction)
        .await?;
        let revision = match current {
            Some(current) => {
                let context = credential_context(transaction, current.revision).await?;
                self.credentials
                    .decrypt(&current.credential, &context)
                    .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
                current.revision
            }
            None => 0,
        };
        if revision != expected {
            return Err(IdentityError::Conflict);
        }
        revision.checked_add(1).ok_or(IdentityError::InvalidInput)
    }
}

async fn administrator(
    transaction: &mut Transaction<'_, Postgres>,
    actor: &Actor,
) -> Result<(), IdentityError> {
    let allowed = sqlx::query_scalar!(
        r#"
        SELECT role = 'administrator' AS "administrator!"
        FROM human_accounts
        WHERE id = $1 AND active
        FOR SHARE
        "#,
        actor.id,
    )
    .fetch_optional(&mut **transaction)
    .await?;
    if allowed != Some(true) {
        return Err(IdentityError::Forbidden);
    }
    Ok(())
}

async fn credential_context(
    transaction: &mut Transaction<'_, Postgres>,
    revision: i64,
) -> Result<Vec<u8>, IdentityError> {
    let installation = sqlx::query_scalar!(
        r#"
        SELECT id AS "id!"
        FROM installation
        "#,
    )
    .fetch_one(&mut **transaction)
    .await?;
    Ok(format!("cubebot:{installation}:provider:openai:key:1:revision:{revision}").into_bytes())
}

fn validate(input: &ConfigureProvider) -> Result<(), IdentityError> {
    if input.request_id.is_nil()
        || input.expected_revision < 0
        || input.model.is_empty()
        || input.model.len() > 128
        || input.api_key.is_empty()
        || input.api_key.len() > 4096
        || !input
            .context_window
            .map_or(input.provider == ProviderKind::Anthropic, |window| {
                (1024..=100_000_000).contains(&window)
            })
    {
        return Err(IdentityError::InvalidInput);
    }
    if !input
        .model
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"-._:/".contains(&byte))
        || !input.api_key.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(IdentityError::InvalidInput);
    }
    Ok(())
}

async fn current_access(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<ProviderAccess, IdentityError> {
    let access = sqlx::query_scalar!(
        r#"
        SELECT access AS "access!: Access"
        FROM provider_connection
        "#,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(access.map(|access| access.0).unwrap_or_default())
}
async fn validate_access(
    tx: &mut Transaction<'_, Postgres>,
    access: &ProviderAccess,
) -> Result<(), IdentityError> {
    if let ProviderAccess::ExplicitGrants { agents } = access {
        if agents.len() > 100 {
            return Err(IdentityError::InvalidInput);
        }
        let count = sqlx::query_scalar!(
            r#"
            SELECT count(*) AS "count!"
            FROM agents
            WHERE id = ANY($1)
            "#,
            agents,
        )
        .fetch_one(&mut **tx)
        .await?;
        if count != agents.len() as i64 {
            return Err(IdentityError::InvalidInput);
        }
    }
    Ok(())
}

async fn selected_access(
    tx: &mut Transaction<'_, Postgres>,
    selected: Option<&ProviderAccess>,
) -> Result<ProviderAccess, IdentityError> {
    let access = match selected {
        Some(selected) => selected.clone(),
        None => current_access(tx).await?,
    };
    validate_access(tx, &access).await?;
    Ok(access)
}

async fn previous(
    transaction: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    request: Uuid,
    digest: &[u8],
) -> Result<Option<ProviderSummary>, IdentityError> {
    // The command's input digest, then its summary row.
    let previous = sqlx::query!(
        r#"
        SELECT
            input_digest AS "input_digest!",
            provider AS "provider!",
            model AS "model!",
            context_window AS "context_window?",
            revision AS "revision!",
            access AS "access!: Access"
        FROM provider_commands
        WHERE actor_id = $1 AND request_id = $2
        "#,
        actor.id,
        request,
    )
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(previous) = previous else {
        return Ok(None);
    };
    if previous.input_digest != digest {
        return Err(IdentityError::Conflict);
    }
    summary((
        previous.provider,
        previous.model,
        previous.context_window,
        previous.revision,
        previous.access,
    ))
    .map(Some)
}

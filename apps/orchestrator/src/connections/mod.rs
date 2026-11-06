mod chatgpt;
mod credentials;
mod mcp;
mod models;
mod native;
mod subscriptions;
mod supergrok;
mod vault;
pub use chatgpt::ChatgptSignIn;
pub use credentials::{CredentialError, Credentials};
pub use mcp::{AgentMcpServer, McpServerSummary, McpTransport, SaveMcpServer};
pub(crate) use models::configured;
pub use models::{ListModels, ModelChoice, ModelsError};
pub use native::{
    AgentConnection, Catalog, ConnectError, ConnectionActivity, ConnectionEvent, ConnectionSummary,
    EventRoutine, FinishSignIn, SaveConnection, SignInStarted, StartSignIn, UpdateConnection,
    callback as callback_address,
};
pub(crate) use subscriptions::For;
pub use subscriptions::{SignInError, SubscriptionSummary, keep_fresh};
pub use supergrok::SupergrokSignIn;
pub(crate) use vault::same_site;
pub use vault::{SaveVaultItem, VaultSummary};

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
    sign_ins: subscriptions::Pending,
    /// Controlled-vendor qualification: subscription OAuth at a loopback origin.
    vendor: Option<reqwest::Url>,
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
    /// `None` keeps the key saved for this provider. A subscription has none: its owner signs in.
    #[serde(default)]
    pub api_key: Option<String>,
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
/// A provider that holds a key without being the one agents use.
#[derive(Serialize, utoipa::ToSchema)]
pub struct SavedProvider {
    pub provider: ProviderKind,
    pub model: String,
    pub context_window: Option<i64>,
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
    /// Unix seconds when a subscription's access token expires.
    pub api_key_expires_at: Option<i64>,
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
        Self {
            pool,
            credentials,
            sign_ins: Default::default(),
            vendor: None,
        }
    }

    /// Sends subscription sign-in and refresh to a literal loopback origin instead of the vendors.
    pub fn with_vendor_for_loopback_tests(mut self, origin: &str) -> Self {
        self.vendor = reqwest::Url::parse(origin)
            .ok()
            .filter(|url| url.host_str() == Some("127.0.0.1"));
        assert!(self.vendor.is_some(), "a loopback origin");
        self
    }

    fn vendor(&self) -> Option<&reqwest::Url> {
        self.vendor.as_ref()
    }

    /// A test controls the vendors, so their pages need not be their own or use HTTPS.
    fn controlled(&self) -> bool {
        self.vendor.is_some() || std::env::var_os("CUBEBOT_TEST_PROVIDER_ORIGIN").is_some()
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
        let encrypted = self
            .credential(&mut transaction, actor, input, revision)
            .await?;
        self.set_aside(&mut transaction, input.provider).await?;
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
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn saved(&self, actor: &Actor) -> Result<Vec<SavedProvider>, IdentityError> {
        let mut transaction = self.pool.begin().await?;
        administrator(&mut transaction, actor).await?;
        let rows = sqlx::query!(
            "
            SELECT provider, model, context_window
            FROM provider_saved
            ORDER BY provider
            ",
        )
        .fetch_all(&mut *transaction)
        .await?;
        transaction.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(SavedProvider {
                    provider: ProviderKind::parse(&row.provider)
                        .ok_or(IdentityError::InvalidInput)?,
                    model: row.model,
                    context_window: row.context_window,
                })
            })
            .collect()
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn forget(&self, actor: &Actor, provider: ProviderKind) -> Result<(), IdentityError> {
        let mut transaction = self.pool.begin().await?;
        administrator(&mut transaction, actor).await?;
        let removed = sqlx::query!(
            "
            DELETE FROM provider_saved
            WHERE provider = $1
            ",
            provider.as_str(),
        )
        .execute(&mut *transaction)
        .await?;
        if removed.rows_affected() == 0 {
            return Err(IdentityError::NotFound);
        }
        transaction.commit().await?;
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    pub async fn key_matches(&self) -> Result<bool, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let provider = sqlx::query!(
            r#"
            SELECT credential AS "credential!", revision AS "revision!"
            FROM provider_connection
            WHERE credential IS NOT NULL
            "#,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let subscription = sqlx::query!(
            "SELECT provider, credential, revision FROM provider_subscriptions ORDER BY provider LIMIT 1"
        )
        .fetch_optional(&mut *tx)
        .await?;
        let (record, context) = if let Some(row) = provider {
            let context = credential_context(&mut tx, row.revision).await?;
            (row.credential, context)
        } else if let Some(row) = subscription {
            let provider = ProviderKind::parse(&row.provider).ok_or(IdentityError::InvalidInput)?;
            let opens = self
                .opened(&mut tx, provider, row.revision, &row.credential)
                .await
                .is_ok();
            tx.commit().await?;
            return Ok(opens);
        } else {
            let item = sqlx::query!(
                "
                SELECT id, ciphertext
                FROM vault_items
                ORDER BY id
                LIMIT 1
                ",
            )
            .fetch_optional(&mut *tx)
            .await?;
            let Some(item) = item else {
                return Ok(true);
            };
            (
                item.ciphertext,
                vault::sealing_context(&mut *tx, "vault", item.id).await?,
            )
        };
        tx.commit().await?;
        Ok(self.credentials.decrypt(&record, &context).is_ok())
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub(crate) async fn inference(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        agent: crate::agents::AgentId,
    ) -> Result<Inference, IdentityError> {
        self.connection(tx, For::Agent(agent)).await
    }

    /// The agent's current material, renewing a due subscription first.
    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub(crate) async fn current(
        &self,
        agent: crate::agents::AgentId,
    ) -> Result<Inference, IdentityError> {
        let active = sqlx::query_scalar!("SELECT provider FROM provider_connection")
            .fetch_optional(&self.pool)
            .await?;
        if let Some(provider) = active
            .as_deref()
            .and_then(ProviderKind::parse)
            .filter(|provider| provider.subscription())
        {
            self.refresh_due(provider).await?;
        }
        let mut tx = self.pool.begin().await?;
        let inference = self.inference(&mut tx, agent).await?;
        tx.commit().await?;
        Ok(inference)
    }

    /// Room routing for a message `person` sent.
    #[tracing::instrument(skip_all)]
    pub(crate) async fn coordinator(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        person: Uuid,
    ) -> Result<Inference, IdentityError> {
        self.connection(tx, For::Person(person)).await
    }

    /// Orchestration of a task `person` owns.
    #[tracing::instrument(skip_all)]
    pub(crate) async fn task_orchestrator(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        person: Uuid,
        provider: Option<ProviderKind>,
        model: Option<&str>,
    ) -> Result<Inference, IdentityError> {
        let (Some(provider), Some(model)) = (provider, model) else {
            return self.coordinator(tx, person).await;
        };
        Ok(Inference {
            provider,
            model: model.to_owned(),
            api_key: self.saved_key(tx, provider).await?,
            api_key_expires_at: None,
            context_window: None,
        })
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0, provider = provider.as_str()))]
    pub(crate) async fn speech_key(
        &self,
        agent: crate::agents::AgentId,
        provider: crate::speech::Provider,
    ) -> Result<String, IdentityError> {
        let mut tx = self.pool.begin().await?;
        if provider == crate::speech::Provider::Elevenlabs {
            let stored = sqlx::query_scalar!("SELECT elevenlabs_credential FROM speech_settings")
                .fetch_one(&mut *tx)
                .await?
                .ok_or(IdentityError::CredentialUnavailable(None))?;
            let context = speech_context(&mut tx).await?;
            let secret = self
                .credentials
                .decrypt(&stored, &context)
                .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
            return String::from_utf8(secret).map_err(|error| {
                IdentityError::CredentialUnavailable(Some(error.utf8_error().into()))
            });
        }
        let active = sqlx::query_scalar!("SELECT provider FROM provider_connection")
            .fetch_optional(&mut *tx)
            .await?;
        if active.as_deref() == Some(ProviderKind::Openai.as_str()) {
            return Ok(self.connection(&mut tx, For::Agent(agent)).await?.api_key);
        }
        self.saved_key(&mut tx, ProviderKind::Openai).await
    }

    pub(crate) async fn store_speech_key(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: Option<&str>,
    ) -> Result<(), IdentityError> {
        let context = speech_context(tx).await?;
        let stored = key
            .map(|key| self.credentials.encrypt(key.as_bytes(), &context))
            .transpose()
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
        sqlx::query!(
            "UPDATE speech_settings SET elevenlabs_credential=$1",
            stored
        )
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// A subscription ignores the access setting: it serves only its owner's work.
    async fn connection(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: For,
    ) -> Result<Inference, IdentityError> {
        let agent = match who {
            For::Agent(agent) => Some(agent.0),
            For::Person(_) | For::Installation => None,
        };
        let row = sqlx::query!(
            r#"
            SELECT
                provider AS "provider!",
                model AS "model!",
                credential AS "credential?",
                revision AS "revision!",
                context_window AS "context_window?"
            FROM provider_connection
            WHERE $1::uuid IS NULL
                OR provider IN ('chatgpt', 'supergrok')
                OR access->>'mode' = 'all_agents'
                OR (access->>'mode' = 'explicit_grants' AND access->'agents' @> jsonb_build_array($1::uuid))
            "#,
            agent,
        )
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(IdentityError::CredentialUnavailable(None))?;
        let provider =
            ProviderKind::parse(&row.provider).ok_or(IdentityError::CredentialUnavailable(None))?;
        let (api_key, api_key_expires_at) = match row.credential {
            Some(credential) => (self.api_key(tx, row.revision, &credential).await?, None),
            None => {
                let (token, expires) = self.subscription_token(tx, provider, who).await?;
                (token, Some(expires))
            }
        };
        Ok(Inference {
            provider,
            model: row.model,
            api_key,
            api_key_expires_at,
            context_window: row.context_window.map(|window| window as u64),
        })
    }

    async fn api_key(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        revision: i64,
        credential: &[u8],
    ) -> Result<String, IdentityError> {
        let context = credential_context(tx, revision).await?;
        let secret = self
            .credentials
            .decrypt(credential, &context)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
        // Only the UTF-8 position survives; FromUtf8Error would keep the decrypted key bytes.
        String::from_utf8(secret)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.utf8_error().into())))
    }

    async fn credential(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        input: &ConfigureProvider,
        revision: i64,
    ) -> Result<Option<Vec<u8>>, IdentityError> {
        if input.provider.subscription() {
            subscriptions::owned(transaction, actor, input.provider).await?;
            return Ok(None);
        }
        let key = match &input.api_key {
            Some(key) => key.clone(),
            None => self.saved_key(transaction, input.provider).await?,
        };
        let context = credential_context(transaction, revision).await?;
        self.credentials
            .encrypt(key.as_bytes(), &context)
            .map(Some)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))
    }

    async fn saved_key(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        provider: ProviderKind,
    ) -> Result<String, IdentityError> {
        let aside = sqlx::query_scalar!(
            "
            SELECT credential
            FROM provider_saved
            WHERE provider = $1
            ",
            provider.as_str(),
        )
        .fetch_optional(&mut **transaction)
        .await?;
        if let Some(credential) = aside {
            let context = saved_context(transaction, provider).await?;
            let secret = self
                .credentials
                .decrypt(&credential, &context)
                .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
            return String::from_utf8(secret).map_err(|error| {
                IdentityError::CredentialUnavailable(Some(error.utf8_error().into()))
            });
        }
        let saved = self
            .connection(transaction, For::Installation)
            .await
            .map_err(|error| match error {
                IdentityError::CredentialUnavailable(None) => IdentityError::InvalidInput,
                error => error,
            })?;
        if saved.provider != provider {
            return Err(IdentityError::InvalidInput);
        }
        Ok(saved.api_key)
    }

    async fn set_aside(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        next: ProviderKind,
    ) -> Result<(), IdentityError> {
        sqlx::query!(
            "
            DELETE FROM provider_saved
            WHERE provider = $1
            ",
            next.as_str(),
        )
        .execute(&mut **transaction)
        .await?;
        let outgoing = match self.connection(transaction, For::Installation).await {
            Ok(outgoing) if outgoing.provider != next => outgoing,
            Ok(_) | Err(IdentityError::CredentialUnavailable(None)) => return Ok(()),
            Err(error) => return Err(error),
        };
        let context = saved_context(transaction, outgoing.provider).await?;
        let credential = self
            .credentials
            .encrypt(outgoing.api_key.as_bytes(), &context)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
        sqlx::query!(
            "
            INSERT INTO provider_saved (provider, model, context_window, credential)
            VALUES ($1, $2, $3, $4)
            ",
            outgoing.provider.as_str(),
            outgoing.model,
            outgoing.context_window.map(|window| window as i64),
            credential,
        )
        .execute(&mut **transaction)
        .await?;
        Ok(())
    }

    async fn next_revision(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        expected: i64,
    ) -> Result<i64, IdentityError> {
        let current = sqlx::query!(
            r#"
            SELECT revision AS "revision!", credential AS "credential?"
            FROM provider_connection
            "#,
        )
        .fetch_optional(&mut **transaction)
        .await?;
        let revision = match current {
            Some(current) => {
                if let Some(credential) = &current.credential {
                    self.api_key(transaction, current.revision, credential)
                        .await?;
                }
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

async fn speech_context(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<Vec<u8>, IdentityError> {
    let installation = sqlx::query_scalar!(
        r#"
        SELECT id AS "id!"
        FROM installation
        "#,
    )
    .fetch_one(&mut **transaction)
    .await?;
    Ok(format!("cubebot:{installation}:speech:elevenlabs").into_bytes())
}

async fn saved_context(
    transaction: &mut Transaction<'_, Postgres>,
    provider: ProviderKind,
) -> Result<Vec<u8>, IdentityError> {
    let installation = sqlx::query_scalar!(
        r#"
        SELECT id AS "id!"
        FROM installation
        "#,
    )
    .fetch_one(&mut **transaction)
    .await?;
    Ok(format!(
        "cubebot:{installation}:provider:{}:saved",
        provider.as_str()
    )
    .into_bytes())
}

fn validate(input: &ConfigureProvider) -> Result<(), IdentityError> {
    if input.provider.subscription() && (input.api_key.is_some() || input.access.is_some()) {
        return Err(IdentityError::InvalidInput);
    }
    if input.request_id.is_nil()
        || input.expected_revision < 0
        || input.model.is_empty()
        || input.model.len() > 128
        || !input.api_key.as_deref().is_none_or(key_valid)
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
        .all(|byte| byte.is_ascii_alphanumeric() || b"-._:/~".contains(&byte))
    {
        return Err(IdentityError::InvalidInput);
    }
    Ok(())
}

pub(crate) fn key_valid(key: &str) -> bool {
    (1..=4096).contains(&key.len()) && key.bytes().all(|byte| byte.is_ascii_graphic())
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

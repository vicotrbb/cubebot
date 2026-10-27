//! Installation-managed inference configuration and encrypted credential storage.
mod credentials;
pub use credentials::{CredentialError, Credentials};

use crate::identity::{Actor, IdentityError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Clone)]
pub struct Connections {
    pool: PgPool,
    credentials: Credentials,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigureProvider {
    pub request_id: Uuid,
    pub expected_revision: i64,
    pub model: String,
    pub api_key: String,
    #[serde(default)]
    pub access: Option<ProviderAccess>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ProviderSummary {
    pub model: String,
    pub revision: i64,
    pub access: sqlx::types::Json<ProviderAccess>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
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

    pub async fn inspect(&self, actor: &Actor) -> Result<Option<ProviderSummary>, IdentityError> {
        let mut transaction = self.pool.begin().await?;
        administrator(&mut transaction, actor).await?;
        let summary = sqlx::query_as("SELECT model, revision, access FROM provider_connection")
            .fetch_optional(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(summary)
    }

    pub async fn configure(
        &self,
        actor: &Actor,
        input: &ConfigureProvider,
    ) -> Result<ProviderSummary, IdentityError> {
        validate(input)?;
        let digest =
            Sha256::digest(serde_json::to_vec(input).map_err(|_| IdentityError::InvalidInput)?)
                .to_vec();
        let mut transaction = self.pool.begin().await?;
        administrator(&mut transaction, actor).await?;
        sqlx::query("SELECT singleton FROM installation FOR UPDATE")
            .execute(&mut *transaction)
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
            .map_err(|_| IdentityError::CredentialUnavailable)?;
        sqlx::query("INSERT INTO provider_connection (model, credential, key_version, revision, updated_by,access) VALUES ($1,$2,1,$3,$4,$5) ON CONFLICT (singleton) DO UPDATE SET model=EXCLUDED.model, credential=EXCLUDED.credential, revision=EXCLUDED.revision, updated_by=EXCLUDED.updated_by, updated_at=clock_timestamp(),access=EXCLUDED.access")
            .bind(&input.model).bind(encrypted).bind(revision).bind(actor.id).bind(sqlx::types::Json(&access)).execute(&mut *transaction).await?;
        sqlx::query("INSERT INTO provider_commands (actor_id,request_id,input_digest,model,revision,access) VALUES ($1,$2,$3,$4,$5,$6)")
            .bind(actor.id).bind(input.request_id).bind(digest).bind(&input.model).bind(revision).bind(sqlx::types::Json(&access)).execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok(ProviderSummary {
            model: input.model.clone(),
            revision,
            access: sqlx::types::Json(access),
        })
    }
    pub(crate) async fn inference(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        agent: crate::agents::AgentId,
    ) -> Result<(String, String, i64), IdentityError> {
        let (model, encrypted, revision): (String, Vec<u8>, i64) =
            sqlx::query_as("SELECT model,credential,revision FROM provider_connection WHERE access->>'mode'='all_agents' OR (access->>'mode'='explicit_grants' AND access->'agents' @> jsonb_build_array($1::uuid))")
                .bind(agent).fetch_optional(&mut **tx)
                .await?
                .ok_or(IdentityError::CredentialUnavailable)?;
        let context = credential_context(tx, revision).await?;
        let secret = self
            .credentials
            .decrypt(&encrypted, &context)
            .map_err(|_| IdentityError::CredentialUnavailable)?;
        let key = String::from_utf8(secret).map_err(|_| IdentityError::CredentialUnavailable)?;
        Ok((model, key, revision))
    }

    async fn next_revision(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        expected: i64,
    ) -> Result<i64, IdentityError> {
        let current: Option<(i64, Vec<u8>)> =
            sqlx::query_as("SELECT revision, credential FROM provider_connection")
                .fetch_optional(&mut **transaction)
                .await?;
        let revision = match current {
            Some((revision, encrypted)) => {
                let context = credential_context(transaction, revision).await?;
                self.credentials
                    .decrypt(&encrypted, &context)
                    .map_err(|_| IdentityError::CredentialUnavailable)?;
                revision
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
    let allowed: Option<bool> = sqlx::query_scalar(
        "SELECT role='administrator' FROM human_accounts WHERE id=$1 AND active FOR SHARE",
    )
    .bind(actor.id)
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
    let installation: Uuid = sqlx::query_scalar("SELECT id FROM installation")
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
    let access: Option<sqlx::types::Json<ProviderAccess>> =
        sqlx::query_scalar("SELECT access FROM provider_connection")
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
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agents WHERE id=ANY($1)")
            .bind(agents)
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
    let previous: Option<(Vec<u8>, String, i64, sqlx::types::Json<ProviderAccess>)> = sqlx::query_as("SELECT input_digest, model, revision, access FROM provider_commands WHERE actor_id=$1 AND request_id=$2")
            .bind(actor.id).bind(request).fetch_optional(&mut **transaction).await?;
    if let Some((old, model, revision, access)) = previous {
        if old != digest {
            return Err(IdentityError::Conflict);
        }
        return Ok(Some(ProviderSummary {
            model,
            revision,
            access,
        }));
    }
    Ok(None)
}

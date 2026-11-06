//! Installation settings administrators change in the console. An unset value keeps the
//! deployment's own, from the Helm chart or `.env`.
use crate::identity::IdentityError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Below these the console would refuse ordinary work.
const MIN_QUOTA: i64 = 64 << 20;
const MIN_UPLOAD: i64 = 1 << 20;

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Shown in the console and to agents; the deployment's name when empty.
    pub name: Option<String>,
    pub storage_quota_bytes: Option<i64>,
    /// The largest file one upload may store.
    pub upload_limit_bytes: Option<i64>,
    pub revision: i64,
}

impl Settings {
    pub async fn load(pool: &PgPool) -> Result<Self, sqlx::Error> {
        sqlx::query_as!(
            Self,
            "SELECT name, storage_quota_bytes, upload_limit_bytes, revision FROM installation_settings",
        )
        .fetch_one(pool)
        .await
    }

    /// 409 when `revision` is not the current one.
    #[tracing::instrument(skip_all)]
    pub async fn save(pool: &PgPool, input: &Self) -> Result<Self, IdentityError> {
        let name = input.name.as_deref().map(str::trim);
        let valid_name = name.is_none_or(|name| {
            (1..=64).contains(&name.chars().count()) && !name.chars().any(char::is_control)
        });
        if !valid_name
            || input
                .storage_quota_bytes
                .is_some_and(|bytes| bytes < MIN_QUOTA)
            || input
                .upload_limit_bytes
                .is_some_and(|bytes| bytes < MIN_UPLOAD)
        {
            return Err(IdentityError::InvalidInput);
        }
        sqlx::query_as!(
            Self,
            "
            UPDATE installation_settings
            SET name=$1, storage_quota_bytes=$2, upload_limit_bytes=$3, revision=revision+1
            WHERE revision=$4
            RETURNING name, storage_quota_bytes, upload_limit_bytes, revision
            ",
            name,
            input.storage_quota_bytes,
            input.upload_limit_bytes,
            input.revision,
        )
        .fetch_optional(pool)
        .await?
        .ok_or(IdentityError::Conflict)
    }

    /// The administrator's name, else `CUBEBOT_INSTALLATION_NAME`, else "Cubebot".
    pub fn name(&self) -> String {
        self.name.clone().unwrap_or_else(deployment_name)
    }

    /// The overrides `Storage::limit` takes.
    pub fn limits(&self) -> (Option<u64>, Option<u64>) {
        let bytes = |value: Option<i64>| value.and_then(|value| u64::try_from(value).ok());
        (
            bytes(self.storage_quota_bytes),
            bytes(self.upload_limit_bytes),
        )
    }
}

pub fn deployment_name() -> String {
    std::env::var("CUBEBOT_INSTALLATION_NAME")
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Cubebot".into())
}

//! Backup settings. A value the deployment sets, in Helm values or `.env`, wins and is read-only in
//! the console; a value saved in the console comes next, then the default.
use super::bucket::Target;
use crate::{connections::Credentials, identity::IdentityError};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

pub const KEEP_DAILY: u32 = 7;
pub const KEEP_WEEKLY: u32 = 4;
pub const BANDWIDTH: u64 = 20 << 20;
pub const CONCURRENCY: usize = 2;
/// The additional data the bucket secret is encrypted with.
const CONTEXT: &[u8] = b"cubebot backup bucket secret";

/// Each setting, absent where it is not set at that level.
// No deny_unknown_fields: SaveBackup flattens these beside its own, which serde would refuse.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Fields {
    /// An S3-compatible API address; AWS S3 in `region` when empty.
    pub endpoint: Option<String>,
    /// us-east-1 when empty; auto for Cloudflare R2.
    pub region: Option<String>,
    pub bucket: Option<String>,
    pub access_key_id: Option<String>,
    /// `daily` or `off`.
    pub schedule: Option<String>,
    /// How many of the newest days keep their newest backup.
    pub keep_daily: Option<i32>,
    /// How many of the newest weeks keep their newest backup.
    pub keep_weekly: Option<i32>,
    /// Bytes a second, shared by every transfer of a backup.
    pub bandwidth_bytes: Option<i64>,
    /// Transfers that run at once.
    pub concurrency: Option<i32>,
}

impl Fields {
    /// What the deployment sets, from `CUBEBOT_BACKUP_*`; the secret is read from a file.
    pub fn deployment() -> (Self, Option<String>) {
        let number = |name| variable(name).and_then(|value| value.parse().ok());
        let fields = Self {
            endpoint: variable("CUBEBOT_BACKUP_ENDPOINT"),
            region: variable("CUBEBOT_BACKUP_REGION"),
            bucket: variable("CUBEBOT_BACKUP_BUCKET"),
            access_key_id: variable("CUBEBOT_BACKUP_ACCESS_KEY_ID"),
            schedule: variable("CUBEBOT_BACKUP_SCHEDULE"),
            keep_daily: number("CUBEBOT_BACKUP_KEEP_DAILY").map(|value: i64| value as i32),
            keep_weekly: number("CUBEBOT_BACKUP_KEEP_WEEKLY").map(|value: i64| value as i32),
            bandwidth_bytes: number("CUBEBOT_BACKUP_BANDWIDTH_BYTES"),
            concurrency: number("CUBEBOT_BACKUP_CONCURRENCY").map(|value: i64| value as i32),
        };
        (fields, secret_file("CUBEBOT_BACKUP_SECRET_ACCESS_KEY_FILE"))
    }

    /// Each of these values, else `other`'s.
    pub fn or(&self, other: &Self) -> Self {
        let pick = |mine: &Option<String>, theirs: &Option<String>| mine.clone().or(theirs.clone());
        Self {
            endpoint: pick(&self.endpoint, &other.endpoint),
            region: pick(&self.region, &other.region),
            bucket: pick(&self.bucket, &other.bucket),
            access_key_id: pick(&self.access_key_id, &other.access_key_id),
            schedule: pick(&self.schedule, &other.schedule),
            keep_daily: self.keep_daily.or(other.keep_daily),
            keep_weekly: self.keep_weekly.or(other.keep_weekly),
            bandwidth_bytes: self.bandwidth_bytes.or(other.bandwidth_bytes),
            concurrency: self.concurrency.or(other.concurrency),
        }
    }

    /// The first value out of range, named for the person who set it.
    pub fn problem(&self) -> Option<&'static str> {
        let endpoint = self.endpoint.as_deref().is_some_and(|endpoint| {
            reqwest::Url::parse(endpoint)
                .ok()
                .filter(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
                .is_none()
        });
        let region = self
            .region
            .as_deref()
            .is_some_and(|region| cubebot_connectors::s3::checked_region(region).is_err());
        let bucket = self.bucket.as_deref().is_some_and(|bucket| {
            !(3..=63).contains(&bucket.len())
                || !bucket.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-')
                })
        });
        let checks = [
            (endpoint, "the endpoint is an http or https address"),
            (
                region,
                "the region is lowercase letters, digits and hyphens",
            ),
            (
                bucket,
                "the bucket is 3 to 63 lowercase letters, digits, dots and hyphens",
            ),
            (
                self.schedule
                    .as_deref()
                    .is_some_and(|schedule| !matches!(schedule, "daily" | "off")),
                "the schedule is daily or off",
            ),
            (
                self.keep_daily
                    .is_some_and(|days| !(1..=365).contains(&days)),
                "keep 1 to 365 daily backups",
            ),
            (
                self.keep_weekly
                    .is_some_and(|weeks| !(0..=520).contains(&weeks)),
                "keep 0 to 520 weekly backups",
            ),
            (
                self.bandwidth_bytes.is_some_and(|bytes| bytes < 1 << 20),
                "the bandwidth is at least 1 MiB a second",
            ),
            (
                self.concurrency
                    .is_some_and(|count| !(1..=16).contains(&count)),
                "run 1 to 16 transfers at once",
            ),
        ];
        checks
            .into_iter()
            .find(|(bad, _)| *bad)
            .map(|(_, problem)| problem)
    }
}

/// A variable's value; an empty one is unset, as Compose passes every `.env` name through.
pub(super) fn variable(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The trimmed contents of the file a variable names; an empty file is unset.
pub(super) fn secret_file(name: &str) -> Option<String> {
    let path = variable(name)?;
    match std::fs::read_to_string(&path) {
        Ok(secret) => Some(secret.trim().to_owned()).filter(|secret| !secret.is_empty()),
        Err(error) => {
            tracing::warn!(?error, variable = name, "backup secret file unreadable");
            None
        }
    }
}

/// The settings in force.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Absent until a bucket, key ID and secret are all set.
    pub target: Option<Target>,
    pub daily: bool,
    pub keep_daily: u32,
    pub keep_weekly: u32,
    pub bandwidth: u64,
    pub concurrency: usize,
}

impl Config {
    pub fn of(fields: &Fields, secret: Option<String>) -> Self {
        let target = match (&fields.bucket, &fields.access_key_id, secret) {
            (Some(bucket), Some(access_key_id), Some(secret_access_key)) => Some(Target {
                endpoint: fields.endpoint.clone(),
                region: fields.region.clone().unwrap_or_else(|| "us-east-1".into()),
                bucket: bucket.clone(),
                access_key_id: access_key_id.clone(),
                secret_access_key,
            }),
            _ => None,
        };
        let positive = |value: Option<i32>, default: u32| {
            value
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(default)
        };
        Self {
            target,
            daily: fields.schedule.as_deref() != Some("off"),
            keep_daily: positive(fields.keep_daily, KEEP_DAILY).max(1),
            keep_weekly: positive(fields.keep_weekly, KEEP_WEEKLY),
            bandwidth: fields
                .bandwidth_bytes
                .and_then(|bytes| u64::try_from(bytes).ok())
                .unwrap_or(BANDWIDTH),
            concurrency: positive(fields.concurrency, CONCURRENCY as u32).clamp(1, 16) as usize,
        }
    }
}

/// The console's saved settings.
pub struct Saved {
    pub fields: Fields,
    pub secret: Option<Vec<u8>>,
    pub revision: i64,
}

impl Saved {
    pub async fn load(pool: &PgPool) -> Result<Self, sqlx::Error> {
        let row = sqlx::query!(
            "
            SELECT endpoint, region, bucket, access_key_id, secret_access_key, schedule, keep_daily,
                keep_weekly, bandwidth_bytes, concurrency, revision
            FROM backup_settings
            "
        )
        .fetch_one(pool)
        .await?;
        Ok(Self {
            fields: Fields {
                endpoint: row.endpoint,
                region: row.region,
                bucket: row.bucket,
                access_key_id: row.access_key_id,
                schedule: row.schedule,
                keep_daily: row.keep_daily,
                keep_weekly: row.keep_weekly,
                bandwidth_bytes: row.bandwidth_bytes,
                concurrency: row.concurrency,
            },
            secret: row.secret_access_key,
            revision: row.revision,
        })
    }

    /// The bucket secret, when one is saved and the key opens it.
    pub fn secret(&self, credentials: Option<&Credentials>) -> Option<String> {
        let opened = credentials?
            .decrypt(self.secret.as_deref()?, CONTEXT)
            .ok()?;
        String::from_utf8(opened).ok()
    }

    /// The settings in force: the deployment's, then these, then the defaults.
    pub fn config(&self, credentials: Option<&Credentials>) -> Config {
        let (deployment, secret) = Fields::deployment();
        Config::of(
            &deployment.or(&self.fields),
            secret.or_else(|| self.secret(credentials)),
        )
    }
}

/// Settings an administrator saves; values the deployment sets are ignored here.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SaveBackup {
    #[serde(flatten)]
    pub fields: Fields,
    /// Replaces the saved secret access key; empty removes it; absent keeps it.
    pub secret_access_key: Option<String>,
    pub revision: i64,
}

/// 400 for a value out of range or a secret without a credential key; 409 for a stale revision.
#[tracing::instrument(skip_all)]
pub async fn save(
    pool: &PgPool,
    credentials: Option<&Credentials>,
    input: &SaveBackup,
) -> Result<Saved, IdentityError> {
    let trim = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let fields = Fields {
        endpoint: trim(&input.fields.endpoint),
        region: trim(&input.fields.region),
        bucket: trim(&input.fields.bucket),
        access_key_id: trim(&input.fields.access_key_id),
        schedule: trim(&input.fields.schedule),
        ..input.fields.clone()
    };
    if fields.problem().is_some() {
        return Err(IdentityError::InvalidInput);
    }
    let secret = match input.secret_access_key.as_deref().map(str::trim) {
        None => None,
        Some("") => Some(None),
        Some(secret) => {
            let credentials = credentials.ok_or(IdentityError::InvalidInput)?;
            let sealed = credentials
                .encrypt(secret.as_bytes(), CONTEXT)
                .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
            Some(Some(sealed))
        }
    };
    let saved = sqlx::query!(
        "
        UPDATE backup_settings
        SET endpoint=$1, region=$2, bucket=$3, access_key_id=$4, schedule=$5, keep_daily=$6,
            keep_weekly=$7, bandwidth_bytes=$8, concurrency=$9,
            secret_access_key=CASE WHEN $10 THEN $11 ELSE secret_access_key END,
            revision=revision+1
        WHERE revision=$12
        RETURNING revision
        ",
        fields.endpoint,
        fields.region,
        fields.bucket,
        fields.access_key_id,
        fields.schedule,
        fields.keep_daily,
        fields.keep_weekly,
        fields.bandwidth_bytes,
        fields.concurrency,
        secret.is_some(),
        secret.flatten(),
        input.revision,
    )
    .fetch_optional(pool)
    .await?;
    saved.ok_or(IdentityError::Conflict)?;
    Ok(Saved::load(pool).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_deployment_wins_and_defaults_fill_the_rest() {
        let deployment = Fields {
            bucket: Some("from-helm".into()),
            ..Fields::default()
        };
        let console = Fields {
            bucket: Some("from-console".into()),
            access_key_id: Some("key".into()),
            keep_daily: Some(3),
            schedule: Some("off".into()),
            ..Fields::default()
        };
        let config = Config::of(&deployment.or(&console), Some("secret".into()));
        let target = config.target.unwrap();
        assert_eq!(target.bucket, "from-helm");
        assert_eq!(target.region, "us-east-1");
        assert_eq!((config.keep_daily, config.keep_weekly), (3, KEEP_WEEKLY));
        assert!(!config.daily);
        assert_eq!(
            Config::of(&console, None).target,
            None,
            "no secret, no bucket"
        );
    }

    #[test]
    fn a_save_reads_its_fields_beside_the_secret_and_revision() {
        let saved: SaveBackup = serde_json::from_value(serde_json::json!({
            "bucket": "b", "keep_daily": 3, "secret_access_key": "s", "revision": 2
        }))
        .unwrap();
        assert_eq!(saved.fields.bucket.as_deref(), Some("b"));
        assert_eq!(saved.fields.keep_daily, Some(3));
        assert_eq!(
            (saved.secret_access_key.as_deref(), saved.revision),
            (Some("s"), 2)
        );
    }

    #[test]
    fn values_out_of_range_are_named() {
        let bad = |fields: Fields| fields.problem();
        assert_eq!(bad(Fields::default()), None);
        assert!(
            bad(Fields {
                endpoint: Some("minio:9000".into()),
                ..Fields::default()
            })
            .is_some()
        );
        assert!(
            bad(Fields {
                bucket: Some("Has Spaces".into()),
                ..Fields::default()
            })
            .is_some()
        );
        assert!(
            bad(Fields {
                schedule: Some("hourly".into()),
                ..Fields::default()
            })
            .is_some()
        );
        assert!(
            bad(Fields {
                concurrency: Some(0),
                ..Fields::default()
            })
            .is_some()
        );
        let fine = Fields {
            endpoint: Some("http://minio.lan:9000".into()),
            region: Some("auto".into()),
            bucket: Some("cubebot-backups".into()),
            ..Fields::default()
        };
        assert_eq!(bad(fine), None);
    }
}

//! Backups to an S3-compatible bucket without pausing anything
//! (docs/adr/packaging.md#backups-without-pauses), and restore into a fresh installation.
//!
//! The bucket holds `blobs/<id>`, each shared-file blob once, and `backups/<id>/`: the database
//! dump, one tar per agent workspace and, written last, `manifest.json`. A backup whose manifest
//! is in the bucket passed verification; one that failed has its manifest removed.
pub mod bucket;
mod capture;
pub mod restore;
mod retention;
pub mod settings;

use crate::{
    connections::Credentials,
    identity::{IdentityError, Source},
    storage::Storage,
};
use axum::body::Bytes;
use bucket::{Bucket, Pace};
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{io::AsyncReadExt, sync::Notify};
use uuid::Uuid;

/// The manifest's own format; a restore refuses one newer than it reads.
pub const SCHEMA: u32 = 1;
/// What the manifest's key check seals with the credential key.
const KEY_CHECK: &[u8] = b"cubebot backup key check";

/// What went wrong, worded for the administrator who reads it in the console.
#[derive(Debug)]
pub struct Failure {
    pub message: String,
    source: Option<Source>,
}

impl Failure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }
    /// The cause's own words follow the message, since administrators read only the message.
    pub fn caused(message: impl Into<String>, source: impl Into<Source>) -> Self {
        let source = source.into();
        Self {
            message: format!("{} ({source})", message.into()),
            source: Some(source),
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_deref().map(|source| source as _)
    }
}

impl From<sqlx::Error> for Failure {
    fn from(error: sqlx::Error) -> Self {
        Self::caused("the database could not be read", error)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub id: String,
    /// The Cubebot release that made it.
    pub version: String,
    /// The newest database migration applied when it was made.
    pub migration: i64,
    pub created_at_unix: i64,
    /// The credential key sealed this; a restore opens it to prove it holds the same key. Absent
    /// when the installation had no key.
    pub key_check: Option<String>,
    pub parts: Vec<Part>,
    pub blobs: Vec<Blob>,
    /// What this backup lacks and why; a backup with any is incomplete.
    pub missing: Vec<Missing>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Part {
    /// `database` or `workspace`.
    pub kind: String,
    pub agent: Option<Uuid>,
    pub key: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Blob {
    pub id: Uuid,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Missing {
    pub part: String,
    pub reason: String,
}

pub fn manifest_key(backup: &str) -> String {
    format!("backups/{backup}/manifest.json")
}
pub fn database_key(backup: &str) -> String {
    format!("backups/{backup}/database.dump")
}
pub fn workspace_key(backup: &str, agent: Uuid) -> String {
    format!("backups/{backup}/workspaces/{agent}.tar")
}
pub fn blob_key(blob: Uuid) -> String {
    format!("blobs/{blob}")
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn unhex(text: &str) -> Option<Vec<u8>> {
    let pairs = text.as_bytes().chunks(2);
    pairs
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect::<Option<Vec<u8>>>()
        .filter(|_| text.len().is_multiple_of(2))
}

/// A reader as a stream of chunks, for uploads.
pub fn chunks<R>(reader: R) -> impl Stream<Item = std::io::Result<Bytes>> + Unpin
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    Box::pin(futures_util::stream::try_unfold(
        reader,
        |mut reader| async move {
            let mut buffer = vec![0; 256 * 1024];
            let read = reader.read(&mut buffer).await?;
            if read == 0 {
                return Ok(None);
            }
            buffer.truncate(read);
            Ok(Some((Bytes::from(buffer), reader)))
        },
    ))
}

pub type Checked = std::pin::Pin<Box<dyn Stream<Item = std::io::Result<Bytes>> + Send>>;

/// `body`, ending in an error instead of its end when its bytes do not hash to `sha256`.
fn checked<S>(body: S, sha256: Vec<u8>) -> Checked
where
    S: Stream<Item = reqwest::Result<Bytes>> + Send + 'static,
{
    use futures_util::StreamExt;
    use sha2::{Digest, Sha256};
    let state = (Box::pin(body), Sha256::new(), sha256, false);
    Box::pin(futures_util::stream::unfold(
        state,
        |(mut body, mut hash, expected, ended)| async move {
            if ended {
                return None;
            }
            let next = match body.next().await {
                Some(Ok(chunk)) => {
                    hash.update(&chunk);
                    return Some((Ok(chunk), (body, hash, expected, false)));
                }
                Some(Err(error)) => std::io::Error::other(error.without_url()),
                None if hash.clone().finalize()[..] == expected[..] => return None,
                None => {
                    std::io::Error::other("the backed-up workspace does not match its checksum")
                }
            };
            Some((Err(next), (body, hash, expected, true)))
        },
    ))
}

/// The backup in progress, which agents' workspace uploads join.
pub(crate) struct Run {
    pub id: String,
    pub bucket: Bucket,
    pub pace: Pace,
    pub permits: tokio::sync::Semaphore,
}

#[derive(Clone)]
pub struct Backups(Arc<Inner>);

struct Inner {
    pool: PgPool,
    database_url: String,
    credentials: Option<Credentials>,
    credential_key: Option<std::path::PathBuf>,
    storage: Option<Storage>,
    /// Where restored agents fetch their workspaces, while any is still to be restored.
    restore: Option<Bucket>,
    wake: Notify,
    requested: Mutex<Option<Option<Uuid>>>,
    run: Mutex<Option<Arc<Run>>>,
}

impl Backups {
    pub fn new(
        pool: PgPool,
        database_url: String,
        credentials: Option<Credentials>,
        storage: Option<Storage>,
    ) -> Self {
        let restore = restore::Source::environment()
            .ok()
            .flatten()
            .and_then(|source| Bucket::new(source.target).ok());
        Self(Arc::new(Inner {
            pool,
            database_url,
            credentials,
            credential_key: std::env::var_os("CUBEBOT_CREDENTIAL_KEY_FILE").map(Into::into),
            storage,
            restore,
            wake: Notify::new(),
            requested: Mutex::new(None),
            run: Mutex::new(None),
        }))
    }

    /// Runs scheduled and requested backups until the process ends.
    pub fn start(&self) {
        let backups = self.clone();
        tokio::spawn(async move { backups.work().await });
    }

    async fn work(self) {
        self.close_interrupted().await;
        loop {
            tokio::select! {
                () = self.0.wake.notified() => {}
                () = tokio::time::sleep(Duration::from_secs(60)) => {}
            }
            let requested = self.lock_requested().take();
            let due = match requested {
                Some(_) => true,
                None => self.due().await,
            };
            if due {
                capture::capture(&self, requested.flatten()).await;
            }
        }
    }

    /// A backup a restart interrupted ended with that process.
    async fn close_interrupted(&self) {
        let interrupted = sqlx::query!(
            "
            UPDATE backups SET status='failed', finished_at=clock_timestamp(),
                error='The orchestrator stopped during this backup.'
            WHERE status='running'
            "
        )
        .execute(&self.0.pool)
        .await;
        if let Err(error) = interrupted {
            tracing::warn!(?error, "interrupted backups could not be closed");
        }
    }

    fn lock_requested(&self) -> std::sync::MutexGuard<'_, Option<Option<Uuid>>> {
        self.0
            .requested
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_run(&self) -> std::sync::MutexGuard<'_, Option<Arc<Run>>> {
        self.0
            .run
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Daily: when nothing started in the last day, or a failed attempt an hour ago.
    async fn due(&self) -> bool {
        let config = match settings::Saved::load(&self.0.pool).await {
            Ok(saved) => saved.config(self.0.credentials.as_ref()),
            Err(error) => {
                tracing::warn!(?error, "backup settings unavailable");
                return false;
            }
        };
        if !config.daily || config.target.is_none() {
            return false;
        }
        let recent = sqlx::query_scalar!(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM backups
                WHERE started_at > clock_timestamp() - CASE WHEN status='failed'
                    THEN interval '1 hour' ELSE interval '23 hours 55 minutes' END
            ) AS "recent!"
            "#
        )
        .fetch_one(&self.0.pool)
        .await;
        matches!(recent, Ok(false))
    }

    /// Starts a backup now; 409 while one runs or is already asked for.
    pub fn request(&self, by: Uuid) -> Result<(), IdentityError> {
        let mut requested = self.lock_requested();
        if self.lock_run().is_some() || requested.is_some() {
            return Err(IdentityError::Conflict);
        }
        *requested = Some(Some(by));
        self.0.wake.notify_one();
        Ok(())
    }

    pub fn running(&self) -> bool {
        self.lock_run().is_some() || self.lock_requested().is_some()
    }

    /// The credential key's bytes, for the one download an administrator may make.
    pub fn credential_key(&self) -> Option<Vec<u8>> {
        std::fs::read(self.0.credential_key.as_ref()?)
            .ok()
            .filter(|key| key.len() == 32)
    }

    /// An agent's workspace for the backup in progress; refused unless that backup asked for it.
    #[tracing::instrument(skip_all, fields(backup, %agent))]
    pub async fn receive_workspace<S, E>(
        &self,
        backup: &str,
        agent: Uuid,
        body: S,
    ) -> Result<(), WorkspaceRefusal>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: Into<Source>,
    {
        let run = self.lock_run().clone().filter(|run| run.id == backup);
        let run = run.ok_or(WorkspaceRefusal::NotAsked)?;
        let claimed = sqlx::query!(
            "
            UPDATE backup_workspaces SET status='uploading'
            WHERE backup_id=$1 AND agent_id=$2 AND status='requested'
            ",
            backup,
            agent,
        )
        .execute(&self.0.pool)
        .await
        .map_err(WorkspaceRefusal::Database)?;
        if claimed.rows_affected() == 0 {
            return Err(WorkspaceRefusal::NotAsked);
        }
        let permit = run.permits.acquire().await;
        let uploaded = run
            .bucket
            .upload(&workspace_key(backup, agent), body, &run.pace)
            .await;
        drop(permit);
        let (bytes, sha256, error) = match &uploaded {
            Ok((bytes, sha256)) => (i64::try_from(*bytes).ok(), Some(sha256.to_vec()), None),
            Err(failure) => (None, None, Some(failure.message.clone())),
        };
        sqlx::query!(
            "
            UPDATE backup_workspaces
            SET status=CASE WHEN $3::text IS NULL THEN 'done' ELSE 'missing' END,
                bytes=$4, sha256=$5, error=$3
            WHERE backup_id=$1 AND agent_id=$2 AND status='uploading'
            ",
            backup,
            agent,
            error,
            bytes,
            sha256,
        )
        .execute(&self.0.pool)
        .await
        .map_err(WorkspaceRefusal::Database)?;
        uploaded.map(drop).map_err(WorkspaceRefusal::Upload)
    }

    /// This agent's backed-up workspace, its size and bytes, while a restore still has to put
    /// it back. Bytes that end up not matching the backup's checksum end the stream in an error.
    pub async fn workspace_restore(&self, agent: Uuid) -> Result<Option<(u64, Checked)>, Failure> {
        let pending = sqlx::query!(
            "SELECT backup_id, bytes, sha256 FROM workspace_restores WHERE agent_id=$1 AND restored_at IS NULL",
            agent
        )
        .fetch_optional(&self.0.pool)
        .await?;
        let Some(pending) = pending else {
            return Ok(None);
        };
        let bucket = self.0.restore.as_ref().ok_or_else(|| {
            Failure::new(
                "The restore source is no longer configured, so this workspace cannot be fetched.",
            )
        })?;
        let response = bucket.get(&workspace_key(&pending.backup_id, agent)).await;
        let response = match response {
            Ok(response) => response,
            Err(failure) => {
                self.restored(agent, Some(failure.message.clone())).await?;
                return Err(failure);
            }
        };
        let bytes = u64::try_from(pending.bytes).unwrap_or_default();
        Ok(Some((
            bytes,
            checked(response.bytes_stream(), pending.sha256),
        )))
    }

    /// Records how the agent's own extraction went; `error` is absent when it succeeded.
    pub async fn restored(&self, agent: Uuid, error: Option<String>) -> Result<(), Failure> {
        sqlx::query!(
            "
            UPDATE workspace_restores
            SET restored_at=CASE WHEN $2::text IS NULL THEN clock_timestamp() END, error=$2
            WHERE agent_id=$1 AND restored_at IS NULL
            ",
            agent,
            error,
        )
        .execute(&self.0.pool)
        .await?;
        Ok(())
    }

    pub fn pool(&self) -> &PgPool {
        &self.0.pool
    }

    pub fn credentials(&self) -> Option<&Credentials> {
        self.0.credentials.as_ref()
    }
}

#[derive(Debug)]
pub enum WorkspaceRefusal {
    /// No backup in progress asked this agent for its workspace, or it already sent it.
    NotAsked,
    Database(sqlx::Error),
    Upload(Failure),
}

/// The newest backups first, as the console lists them.
#[derive(Serialize, utoipa::ToSchema)]
pub struct BackupSummary {
    pub id: String,
    pub started_at_unix: i64,
    pub finished_at_unix: Option<i64>,
    /// `running`, `complete`, `incomplete` (verified, with parts missing), `failed` or `deleted`
    /// (removed by retention).
    pub status: String,
    pub bytes: i64,
    pub missing: Vec<Missing>,
    pub error: Option<String>,
    /// Who started it; absent for a scheduled one.
    pub requested_by: Option<String>,
}

/// An agent workspace a restore put back, or could not.
#[derive(Serialize, utoipa::ToSchema)]
pub struct WorkspaceRestore {
    pub agent: Uuid,
    pub agent_name: String,
    pub backup: String,
    pub restored_at_unix: Option<i64>,
    pub error: Option<String>,
}

impl Backups {
    pub async fn history(&self) -> Result<Vec<BackupSummary>, sqlx::Error> {
        let rows = sqlx::query!(
            r#"
            SELECT b.id, EXTRACT(EPOCH FROM b.started_at)::bigint AS "started!",
                EXTRACT(EPOCH FROM b.finished_at)::bigint AS finished, b.status, b.bytes, b.missing,
                b.error, a.login AS "login?"
            FROM backups b LEFT JOIN human_accounts a ON a.id=b.requested_by
            ORDER BY b.id DESC LIMIT 60
            "#
        )
        .fetch_all(&self.0.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| BackupSummary {
                id: row.id,
                started_at_unix: row.started,
                finished_at_unix: row.finished,
                status: row.status,
                bytes: row.bytes,
                missing: serde_json::from_value(row.missing).unwrap_or_default(),
                error: row.error,
                requested_by: row.login,
            })
            .collect())
    }

    pub async fn workspace_restores(&self) -> Result<Vec<WorkspaceRestore>, sqlx::Error> {
        let rows = sqlx::query!(
            r#"
            SELECT r.agent_id, a.name, r.backup_id, EXTRACT(EPOCH FROM r.restored_at)::bigint AS restored,
                r.error
            FROM workspace_restores r JOIN agents a ON a.id=r.agent_id
            ORDER BY a.name
            "#
        )
        .fetch_all(&self.0.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| WorkspaceRestore {
                agent: row.agent_id,
                agent_name: row.name,
                backup: row.backup_id,
                restored_at_unix: row.restored,
                error: row.error,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_refuses_odd_or_foreign_text() {
        let bytes = [0, 15, 16, 255];
        assert_eq!(hex(&bytes), "000f10ff");
        assert_eq!(unhex("000f10ff").unwrap(), bytes);
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
    }
}

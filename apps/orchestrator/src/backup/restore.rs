//! Restore into a fresh installation, at the orchestrator's first start: when the database is
//! empty and `CUBEBOT_RESTORE_*` names a bucket. It checks the credential key first, then loads
//! the database and the shared files; each agent fetches its own workspace before it takes work.
//!
//! A marker on the shared-file volume says a restore began, so one interrupted after the database
//! loaded resumes rather than starting a half-restored installation.
use super::{
    Blob, Failure, KEY_CHECK, Manifest, Missing, SCHEMA, blob_key,
    bucket::{Bucket, Target},
    database_key, hex, manifest_key,
    settings::{secret_file, variable},
    unhex,
};
use crate::connections::Credentials;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncWriteExt, sync::watch};

const MARKER: &str = "restore.json";
const DUMP: &str = "restore.dump";
/// Blobs fetched at once.
const FETCHES: usize = 4;

/// What the orchestrator serves while it restores: the progress, and liveness and readiness so
/// probes neither restart it nor take it out of its Service; everything else is 503.
pub fn router(progress: watch::Receiver<RestoreProgress>) -> axum::Router {
    use axum::{Json, http::StatusCode, routing::get};
    let current = move || {
        let progress = progress.borrow().clone();
        async move { ([("cache-control", "no-store")], Json(progress)) }
    };
    let alive = || async {
        Json(cubebot_protocol::Liveness {
            status: cubebot_protocol::LivenessStatus::Alive,
        })
    };
    axum::Router::new()
        .route("/api/restore", get(current))
        .route("/api/health/live", get(alive))
        .route("/api/health/ready", get(|| async { StatusCode::OK }))
        .fallback(|| async {
            let body = Json(serde_json::json!({"error": "restoring"}));
            (StatusCode::SERVICE_UNAVAILABLE, body)
        })
}

/// Where to restore from.
#[derive(Clone, Debug)]
pub struct Source {
    pub target: Target,
    /// A backup's id; the newest verified one when absent.
    pub backup: Option<String>,
}

impl Source {
    /// From `CUBEBOT_RESTORE_*`; absent unless `CUBEBOT_RESTORE_BUCKET` is set.
    pub fn environment() -> Result<Option<Self>, Failure> {
        let Some(bucket) = variable("CUBEBOT_RESTORE_BUCKET") else {
            return Ok(None);
        };
        let access_key_id = variable("CUBEBOT_RESTORE_ACCESS_KEY_ID").ok_or_else(|| {
            Failure::new("CUBEBOT_RESTORE_BUCKET needs CUBEBOT_RESTORE_ACCESS_KEY_ID")
        })?;
        let secret_access_key = secret_file("CUBEBOT_RESTORE_SECRET_ACCESS_KEY_FILE").ok_or_else(|| {
            Failure::new("CUBEBOT_RESTORE_BUCKET needs CUBEBOT_RESTORE_SECRET_ACCESS_KEY_FILE, a file holding the secret access key")
        })?;
        Ok(Some(Self {
            target: Target {
                endpoint: variable("CUBEBOT_RESTORE_ENDPOINT"),
                region: variable("CUBEBOT_RESTORE_REGION").unwrap_or_else(|| "us-east-1".into()),
                bucket,
                access_key_id,
                secret_access_key,
            },
            backup: variable("CUBEBOT_RESTORE_BACKUP").filter(|backup| backup != "newest"),
        }))
    }
}

/// What the console shows while a restore runs.
#[derive(Clone, Debug, Default, Serialize, utoipa::ToSchema)]
pub struct RestoreProgress {
    /// `restoring`, or `failed` with `error`.
    pub state: &'static str,
    /// The backup's id, once chosen.
    pub backup: Option<String>,
    pub step: String,
    /// Shared files fetched so far, of `total`.
    pub done: u64,
    pub total: u64,
    pub error: Option<String>,
    /// What the backup itself lacked.
    pub missing: Vec<Missing>,
}

#[derive(Serialize, Deserialize)]
struct Marker {
    backup: String,
}

/// A restore this start must run, before anything else touches the database.
pub struct Plan {
    source: Source,
    url: String,
    files: PathBuf,
    /// Set when an interrupted restore of this backup resumes.
    resume: Option<String>,
}

/// Whether this start restores: an unfinished restore resumes; otherwise only an empty database
/// with a restore source configured is restored into.
pub async fn plan(url: &str) -> Result<Option<Plan>, Failure> {
    let Some(source) = Source::environment()? else {
        return Ok(None);
    };
    let files = variable("CUBEBOT_STORAGE_DIRECTORY")
        .map(PathBuf::from)
        .ok_or_else(|| {
            Failure::new("a restore needs CUBEBOT_STORAGE_DIRECTORY, the shared-file volume")
        })?;
    let resume = match std::fs::read(files.join(MARKER)) {
        Ok(bytes) => Some(
            serde_json::from_slice::<Marker>(&bytes)
                .map_err(|error| {
                    Failure::caused(
                        "the restore marker on the shared-file volume is unreadable",
                        error,
                    )
                })?
                .backup,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(Failure::caused(
                "the restore marker could not be read",
                error,
            ));
        }
    };
    let plan = Plan {
        source,
        url: url.to_owned(),
        files,
        resume,
    };
    if plan.resume.is_none() && !empty(&connect(url).await?).await? {
        tracing::info!(
            "a restore source is set but the database already holds an installation; nothing is restored"
        );
        return Ok(None);
    }
    Ok(Some(plan))
}

async fn connect(url: &str) -> Result<PgPool, Failure> {
    let mut attempts = 0;
    loop {
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(5))
            .connect(url)
            .await;
        match pool {
            Ok(pool) => return Ok(pool),
            // The database may still be starting beside a fresh installation.
            Err(_) if attempts < 60 => {
                attempts += 1;
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(error) => return Err(Failure::caused("the database could not be reached", error)),
        }
    }
}

/// No migration has run, so nothing of an installation is there.
async fn empty(pool: &PgPool) -> Result<bool, Failure> {
    let present = sqlx::query_scalar!(
        r#"SELECT to_regclass('public._sqlx_migrations') IS NOT NULL AS "present!""#
    )
    .fetch_one(pool)
    .await?;
    Ok(!present)
}

impl Plan {
    /// Runs the restore, reporting each step; on failure the last report says why.
    pub async fn run(self, progress: watch::Sender<RestoreProgress>) -> Result<(), Failure> {
        let report = |step: &str| {
            progress.send_modify(|progress| {
                progress.state = "restoring";
                progress.step = step.to_owned();
            });
        };
        report("Reading the backup");
        let outcome = self.restore(&progress, report).await;
        if let Err(failure) = &outcome {
            tracing::error!(error = ?failure, "restore failed");
            progress.send_modify(|progress| {
                progress.state = "failed";
                progress.error = Some(failure.message.clone());
            });
        }
        outcome
    }

    async fn restore(
        &self,
        progress: &watch::Sender<RestoreProgress>,
        report: impl Fn(&str),
    ) -> Result<(), Failure> {
        let bucket = Bucket::new(self.source.target.clone())?;
        let manifest = self.manifest(&bucket).await?;
        let id = manifest.id.clone();
        progress.send_modify(|progress| {
            progress.backup = Some(id.clone());
            progress.missing = manifest.missing.clone();
        });
        compatible(&manifest)?;
        if self.resume.is_none() {
            key_matches(&manifest)?;
            tokio::fs::create_dir_all(&self.files)
                .await
                .map_err(|error| {
                    Failure::caused(format!("{} could not be made", self.files.display()), error)
                })?;
            let marker = serde_json::to_vec(&Marker { backup: id.clone() }).unwrap_or_default();
            write_synced(&self.files.join(MARKER), &marker).await?;
        }
        self.load(&bucket, &manifest, progress, report).await?;
        tracing::info!(backup = %id, "restore finished");
        Ok(())
    }

    /// The database, unless an interrupted restore already loaded it, then the shared files and
    /// the workspaces agents will fetch; last, the marker goes.
    async fn load(
        &self,
        bucket: &Bucket,
        manifest: &Manifest,
        progress: &watch::Sender<RestoreProgress>,
        report: impl Fn(&str),
    ) -> Result<(), Failure> {
        let pool = connect(&self.url).await?;
        if empty(&pool).await? {
            report("Loading the database");
            self.database(bucket, manifest).await?;
        }
        report("Fetching shared files");
        self.blobs(bucket, manifest, progress).await?;
        report("Preparing agent workspaces");
        record(&pool, manifest).await?;
        forget_platform(&pool).await?;
        let _ = tokio::fs::remove_file(self.files.join(DUMP)).await;
        tokio::fs::remove_file(self.files.join(MARKER))
            .await
            .map_err(|error| Failure::caused("the restore marker could not be removed", error))
    }

    /// The chosen backup's manifest: the one a resumed restore began, the one named, or the newest.
    async fn manifest(&self, bucket: &Bucket) -> Result<Manifest, Failure> {
        let id = match (&self.resume, &self.source.backup) {
            (Some(id), _) | (None, Some(id)) => id.clone(),
            (None, None) => newest(bucket).await?,
        };
        let bytes = bucket.bytes(&manifest_key(&id)).await.map_err(|failure| {
            let bucket = bucket.name();
            Failure::new(format!(
                "Backup {id} has no manifest in bucket {bucket}: {failure}"
            ))
        })?;
        let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|error| {
            Failure::caused(format!("the manifest of backup {id} is unreadable"), error)
        })?;
        if manifest.id != id {
            return Err(Failure::new(format!(
                "the manifest of backup {id} names another backup"
            )));
        }
        Ok(manifest)
    }

    /// The dump is fetched and checked in full first, since pg_restore commits as its input ends.
    async fn database(&self, bucket: &Bucket, manifest: &Manifest) -> Result<(), Failure> {
        let part = manifest
            .parts
            .iter()
            .find(|part| part.kind == "database")
            .ok_or_else(|| Failure::new("the backup has no database"))?;
        let path = self.files.join(DUMP);
        let sha256 = download(bucket, &database_key(&manifest.id), &path).await?;
        if sha256 != part.sha256 {
            return Err(Failure::new(
                "The backup's database dump does not match its checksum; it may be damaged.",
            ));
        }
        let output = tokio::process::Command::new("pg_restore")
            .args([
                "--no-owner",
                "--no-privileges",
                "--single-transaction",
                "--exit-on-error",
            ])
            .arg("--dbname")
            .arg(&self.url)
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|error| Failure::caused("pg_restore could not start", error))?;
        if !output.status.success() {
            let said = String::from_utf8_lossy(&output.stderr);
            let said = said.lines().last().unwrap_or("no message").to_owned();
            return Err(Failure::new(format!("pg_restore failed: {said}")));
        }
        Ok(())
    }

    async fn blobs(
        &self,
        bucket: &Bucket,
        manifest: &Manifest,
        progress: &watch::Sender<RestoreProgress>,
    ) -> Result<(), Failure> {
        let wanted: Vec<Blob> = manifest
            .blobs
            .iter()
            .filter(|blob| {
                !manifest
                    .missing
                    .iter()
                    .any(|missing| missing.part == format!("shared file {}", blob.id))
            })
            .cloned()
            .collect();
        progress.send_modify(|progress| progress.total = wanted.len() as u64);
        let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(FETCHES));
        let mut fetching = tokio::task::JoinSet::new();
        for blob in wanted {
            let (bucket, files, permits) = (bucket.clone(), self.files.clone(), permits.clone());
            fetching.spawn(async move {
                let _permit = permits.acquire().await;
                fetch_blob(&bucket, &files, &blob).await
            });
        }
        while let Some(outcome) = fetching.join_next().await {
            outcome.map_err(|error| Failure::caused("a shared-file fetch stopped", error))??;
            progress.send_modify(|progress| progress.done += 1);
        }
        Ok(())
    }
}

async fn fetch_blob(bucket: &Bucket, files: &Path, blob: &Blob) -> Result<(), Failure> {
    if crate::storage::restored_blob(files, blob.id, blob.bytes) {
        return Ok(());
    }
    let sha256 = unhex(&blob.sha256).unwrap_or_default();
    let response = bucket.get(&blob_key(blob.id)).await?;
    let body = response
        .bytes_stream()
        .map(|chunk| chunk.map_err(|error| error.without_url()));
    crate::storage::restore_blob(files, blob.id, &sha256, body)
        .await
        .map_err(|error| {
            Failure::caused(
                format!("shared file {} could not be restored: {error}", blob.id),
                error,
            )
        })
}

/// The newest backup whose manifest is in the bucket, which only verified backups keep.
async fn newest(bucket: &Bucket) -> Result<String, Failure> {
    let keys = bucket.list("backups/").await?;
    keys.iter()
        .filter_map(|(key, _)| key.strip_prefix("backups/")?.strip_suffix("/manifest.json"))
        .max()
        .map(str::to_owned)
        .ok_or_else(|| {
            Failure::new(format!(
                "Bucket {} holds no verified backup.",
                bucket.name()
            ))
        })
}

fn compatible(manifest: &Manifest) -> Result<(), Failure> {
    if manifest.schema > SCHEMA {
        return Err(Failure::new(format!(
            "Backup {} was made by Cubebot {}, whose backups this release cannot read; install that release or a later one.",
            manifest.id, manifest.version
        )));
    }
    let newest = sqlx::migrate!("./migrations")
        .iter()
        .map(|migration| migration.version)
        .max()
        .unwrap_or(0);
    if manifest.migration > newest {
        return Err(Failure::new(format!(
            "Backup {} was made by Cubebot {}, which is newer than this release; install that release or a later one to restore it.",
            manifest.id, manifest.version
        )));
    }
    Ok(())
}

/// The restored records of the backed-up installation's processes and Kubernetes objects name
/// things that do not exist here, and absence never proves they stopped; so the restore retires
/// them, in one transaction: no process owns this installation yet, its anchor is made again,
/// recorded agent pods are stopped as restored and their workspace claims planned anew. Docker
/// sandboxes, recorded under namespace `docker`, already recover from missing containers.
async fn forget_platform(pool: &PgPool) -> Result<(), Failure> {
    let mut transaction = pool.begin().await?;
    sqlx::query!("DELETE FROM orchestrator_owner")
        .execute(&mut *transaction)
        .await?;
    sqlx::query!("DELETE FROM orchestrator_development_owner")
        .execute(&mut *transaction)
        .await?;
    sqlx::query!("DELETE FROM installation_kubernetes")
        .execute(&mut *transaction)
        .await?;
    sqlx::query!(
        r#"
        UPDATE agent_resources
        SET phase = 'stopped', termination_proof = '{"kind":"restored"}'::jsonb,
            blocked_reason = NULL, updated_at = clock_timestamp()
        WHERE kind = 'pod' AND phase <> 'stopped' AND namespace <> 'docker'
        "#
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query!("DELETE FROM agent_resources WHERE kind = 'workspace' AND namespace <> 'docker'")
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

/// Fails fast, before anything is written, when this installation's key is not the backup's.
fn key_matches(manifest: &Manifest) -> Result<(), Failure> {
    let Some(check) = &manifest.key_check else {
        return Ok(());
    };
    let Some(path) = std::env::var_os("CUBEBOT_CREDENTIAL_KEY_FILE") else {
        return Err(Failure::new(
            "The backup's credentials are encrypted, but this installation has no credential key; set CUBEBOT_CREDENTIAL_KEY_FILE to the backed-up installation's key.",
        ));
    };
    let credentials = Credentials::from_file(Path::new(&path)).map_err(|error| {
        Failure::caused(
            "CUBEBOT_CREDENTIAL_KEY_FILE is not a private 32-byte key",
            error,
        )
    })?;
    let sealed = unhex(check).unwrap_or_default();
    match credentials.decrypt(&sealed, KEY_CHECK) {
        Ok(opened) if opened == KEY_CHECK => Ok(()),
        _ => Err(Failure::new(
            "The credential key is not the one the backed-up installation used. Give this installation that key: with Compose, the line cubebot key printed on the old machine; with Helm, its credentials Secret.",
        )),
    }
}

/// Saves an object to `path`, returning its SHA-256 in hex.
async fn download(bucket: &Bucket, key: &str, path: &Path) -> Result<String, Failure> {
    use sha2::{Digest, Sha256};
    let response = bucket.get(key).await?;
    let mut file = tokio::fs::File::create(path).await.map_err(|error| {
        Failure::caused(format!("{} could not be written", path.display()), error)
    })?;
    let mut hash = Sha256::new();
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|error| {
            Failure::caused(format!("{key} could not be read"), error.without_url())
        })?;
        hash.update(&chunk);
        file.write_all(&chunk).await.map_err(|error| {
            Failure::caused(format!("{} could not be written", path.display()), error)
        })?;
    }
    file.sync_all().await.map_err(|error| {
        Failure::caused(format!("{} could not be written", path.display()), error)
    })?;
    Ok(hex(&hash.finalize()))
}

async fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    let written = async {
        let mut file = tokio::fs::File::create(path).await?;
        file.write_all(bytes).await?;
        file.sync_all().await
    };
    written
        .await
        .map_err(|error| Failure::caused(format!("{} could not be written", path.display()), error))
}

/// The restored database still shows this backup running, as it was when dumped; it gets its
/// verified status, and each backed-up workspace waits for its agent.
async fn record(pool: &PgPool, manifest: &Manifest) -> Result<(), Failure> {
    let status = if manifest.missing.is_empty() {
        "complete"
    } else {
        "incomplete"
    };
    sqlx::query!(
        "
        UPDATE backups SET status=$2, missing=$3, finished_at=to_timestamp($4), bytes=$5, error=NULL
        WHERE id=$1 AND status='running'
        ",
        manifest.id,
        status,
        serde_json::to_value(&manifest.missing).unwrap_or_default(),
        manifest.created_at_unix as f64,
        i64::try_from(manifest.parts.iter().map(|part| part.bytes).sum::<u64>())
            .unwrap_or(i64::MAX),
    )
    .execute(pool)
    .await?;
    for part in manifest
        .parts
        .iter()
        .filter(|part| part.kind == "workspace")
    {
        let (Some(agent), Some(sha256)) = (part.agent, unhex(&part.sha256)) else {
            continue;
        };
        sqlx::query!(
            "
            INSERT INTO workspace_restores (agent_id, backup_id, bytes, sha256)
            SELECT $1, $2, $3, $4 WHERE EXISTS (SELECT 1 FROM agents WHERE id=$1)
            ON CONFLICT (agent_id) DO NOTHING
            ",
            agent,
            manifest.id,
            i64::try_from(part.bytes).unwrap_or(i64::MAX),
            sha256,
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(key_check: Option<String>) -> Manifest {
        Manifest {
            schema: SCHEMA,
            id: "20261107T030000Z".into(),
            version: "0.2.4".into(),
            migration: 68,
            created_at_unix: 0,
            key_check,
            parts: Vec::new(),
            blobs: Vec::new(),
            missing: Vec::new(),
        }
    }

    #[test]
    fn backups_from_a_newer_release_are_refused() {
        assert!(compatible(&manifest(None)).is_ok());
        let newer = Manifest {
            migration: 100_000,
            ..manifest(None)
        };
        assert!(
            compatible(&newer)
                .unwrap_err()
                .message
                .contains("newer than this release")
        );
        let format = Manifest {
            schema: SCHEMA + 1,
            ..manifest(None)
        };
        assert!(compatible(&format).is_err());
    }

    #[test]
    fn a_key_check_opens_only_with_the_key_that_sealed_it() {
        let key = Credentials::new([7; 32]);
        let sealed = hex(&key.encrypt(KEY_CHECK, KEY_CHECK).unwrap());
        let opened = key.decrypt(&unhex(&sealed).unwrap(), KEY_CHECK).unwrap();
        assert_eq!(opened, KEY_CHECK);
        let other = Credentials::new([8; 32]);
        assert!(other.decrypt(&unhex(&sealed).unwrap(), KEY_CHECK).is_err());
    }
}

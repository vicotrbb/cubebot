//! One backup: the database from one exported snapshot, the shared-file blobs that snapshot
//! names and the bucket lacks, each agent's workspace, then the manifest, verification and
//! retention. Nothing pauses, so the parts may differ by the minutes between them.
use super::{
    Backups, Blob, Failure, KEY_CHECK, Manifest, Missing, Part, Run, SCHEMA, blob_key,
    bucket::{Bucket, Pace},
    chunks, database_key, hex, manifest_key, retention, settings,
};
use crate::storage::Storage;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, process::Stdio, sync::Arc, time::Duration};
use tokio::io::AsyncReadExt;
use uuid::Uuid;

/// How long an agent may take to start sending its workspace.
const CONNECT: Duration = Duration::from_secs(10 * 60);
/// How long the whole set of workspaces may take.
const WORKSPACES: Duration = Duration::from_secs(12 * 60 * 60);
/// Parts re-read in full to check their SHA-256.
const SPOT_CHECKS: usize = 3;

#[tracing::instrument(skip_all)]
pub(super) async fn capture(backups: &Backups, requested_by: Option<Uuid>) {
    let pool = &backups.0.pool;
    let started = sqlx::query_scalar!(
        r#"
        INSERT INTO backups (id, requested_by)
        VALUES (to_char(clock_timestamp() AT TIME ZONE 'UTC', 'YYYYMMDD"T"HH24MISS"Z"'), $1)
        RETURNING id
        "#,
        requested_by,
    )
    .fetch_one(pool)
    .await;
    let id = match started {
        Ok(id) => id,
        Err(error) => {
            tracing::warn!(?error, "a backup could not start");
            return;
        }
    };
    let outcome = run(backups, &id).await;
    *backups.lock_run() = None;
    let (status, bytes, missing, error) = match &outcome {
        Ok(manifest) => {
            let bytes: u64 = manifest.parts.iter().map(|part| part.bytes).sum();
            let status = if manifest.missing.is_empty() {
                "complete"
            } else {
                "incomplete"
            };
            (
                status,
                bytes,
                serde_json::to_value(&manifest.missing).unwrap_or_default(),
                None,
            )
        }
        Err(failure) => {
            tracing::warn!(error = ?failure, backup = %id, "backup failed");
            (
                "failed",
                0,
                serde_json::json!([]),
                Some(failure.message.clone()),
            )
        }
    };
    let recorded = sqlx::query!(
        "UPDATE backups SET status=$2, bytes=$3, missing=$4, error=$5, finished_at=clock_timestamp() WHERE id=$1",
        id,
        status,
        i64::try_from(bytes).unwrap_or(i64::MAX),
        missing,
        error,
    )
    .execute(pool)
    .await;
    if let Err(error) = recorded {
        tracing::warn!(?error, backup = %id, "backup outcome not recorded");
    }
    tracing::info!(backup = %id, status, bytes, "backup finished");
}

/// The settings in force, refusing invalid deployment values and a missing bucket.
async fn config(backups: &Backups) -> Result<settings::Config, Failure> {
    let (deployment, _) = settings::Fields::deployment();
    if let Some(problem) = deployment.problem() {
        return Err(Failure::new(format!(
            "A CUBEBOT_BACKUP_ setting of the deployment is invalid: {problem}."
        )));
    }
    let saved = settings::Saved::load(&backups.0.pool).await?;
    Ok(saved.config(backups.credentials()))
}

fn key_check(backups: &Backups) -> Result<Option<String>, Failure> {
    let Some(credentials) = backups.credentials() else {
        return Ok(None);
    };
    let sealed = credentials.encrypt(KEY_CHECK, KEY_CHECK).map_err(|error| {
        Failure::caused("the credential key could not seal the key check", error)
    })?;
    Ok(Some(hex(&sealed)))
}

async fn run(backups: &Backups, id: &str) -> Result<Manifest, Failure> {
    let config = config(backups).await?;
    let target = config.target.clone().ok_or_else(|| {
        Failure::new("No bucket is set: give the bucket, its access key ID and secret access key.")
    })?;
    let run = Arc::new(Run {
        id: id.to_owned(),
        bucket: Bucket::new(target)?,
        pace: Pace::new(config.bandwidth),
        permits: tokio::sync::Semaphore::new(config.concurrency),
    });
    *backups.lock_run() = Some(run.clone());
    let (database, blobs, migration) = database(&run, backups).await?;
    let mut missing = upload_blobs(&run, backups.0.storage.as_ref(), &blobs).await?;
    let (workspaces, absent) = workspaces(&run, backups).await?;
    missing.extend(absent);
    let key_check = key_check(backups)?;
    let manifest = Manifest {
        schema: SCHEMA,
        id: id.to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        migration,
        created_at_unix: unix_now(),
        key_check,
        parts: std::iter::once(database).chain(workspaces).collect(),
        blobs,
        missing,
    };
    seal(&run, &manifest).await?;
    if let Err(failure) =
        retention::prune(&run, &backups.0.pool, config.keep_daily, config.keep_weekly).await
    {
        tracing::warn!(error = ?failure, "old backups were not pruned");
    }
    Ok(manifest)
}

/// Writes the manifest last, then verifies; a backup that fails verification loses its manifest,
/// so it is never restored and retention removes its parts.
async fn seal(run: &Run, manifest: &Manifest) -> Result<(), Failure> {
    let written = serde_json::to_vec(manifest)
        .map_err(|error| Failure::caused("the manifest could not be written", error))?;
    let key = manifest_key(&manifest.id);
    run.bucket.put(&key, written.clone()).await?;
    let verified = verify(run, manifest, &written).await;
    if verified.is_err() {
        let _ = run.bucket.delete(&key).await;
    }
    verified
}

fn unix_now() -> i64 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    now.map_or(0, |now| now.as_secs() as i64)
}

/// pg_dump reads one snapshot without blocking writers; the same snapshot lists the blobs, so
/// the dump never names a file this backup leaves out.
async fn database(run: &Run, backups: &Backups) -> Result<(Part, Vec<Blob>, i64), Failure> {
    let mut snapshot = backups.0.pool.begin().await?;
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *snapshot)
        .await?;
    let name = sqlx::query_scalar!(r#"SELECT pg_export_snapshot() AS "name!""#)
        .fetch_one(&mut *snapshot)
        .await?;
    let blobs = sqlx::query!("SELECT id, size, sha256 FROM blobs ORDER BY id")
        .fetch_all(&mut *snapshot)
        .await?
        .into_iter()
        .map(|row| Blob {
            id: row.id,
            bytes: u64::try_from(row.size).unwrap_or_default(),
            sha256: hex(&row.sha256),
        })
        .collect();
    let migration = sqlx::query_scalar!(
        r#"SELECT COALESCE(max(version), 0) AS "version!" FROM _sqlx_migrations"#
    )
    .fetch_one(&mut *snapshot)
    .await?;
    let mut dump = tokio::process::Command::new("pg_dump")
        .args(["--format=custom", "--no-owner", "--no-privileges"])
        .arg(format!("--snapshot={name}"))
        .arg("--dbname")
        .arg(&backups.0.database_url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| Failure::caused("pg_dump could not start", error))?;
    let (Some(output), Some(mut errors)) = (dump.stdout.take(), dump.stderr.take()) else {
        return Err(Failure::new("pg_dump's output was not captured"));
    };
    let complaint = tokio::spawn(async move {
        let mut text = String::new();
        let _ = errors.read_to_string(&mut text).await;
        text
    });
    let key = database_key(&run.id);
    let uploaded = run.bucket.upload(&key, chunks(output), &run.pace).await;
    let status = dump
        .wait()
        .await
        .map_err(|error| Failure::caused("pg_dump did not finish", error))?;
    snapshot.rollback().await?;
    let complaint = complaint.await.unwrap_or_default();
    if !status.success() {
        let said = complaint.lines().last().unwrap_or("no message");
        return Err(Failure::new(format!("pg_dump failed: {said}")));
    }
    let (bytes, sha256) = uploaded?;
    let part = Part {
        kind: "database".into(),
        agent: None,
        key,
        bytes,
        sha256: hex(&sha256),
    };
    Ok((part, blobs, migration))
}

/// Blobs never change once written, so only those the bucket lacks are sent.
async fn upload_blobs(
    run: &Arc<Run>,
    storage: Option<&Storage>,
    blobs: &[Blob],
) -> Result<Vec<Missing>, Failure> {
    let present: HashMap<String, u64> = run.bucket.list("blobs/").await?.into_iter().collect();
    let wanted: Vec<&Blob> = blobs
        .iter()
        .filter(|blob| present.get(&blob_key(blob.id)) != Some(&blob.bytes))
        .collect();
    let storage = storage.cloned();
    // shortcut: one waiting task per blob the bucket lacks; send in batches if an installation's
    // first backup ever faces millions of files.
    let mut sending = tokio::task::JoinSet::new();
    for blob in wanted.into_iter().cloned() {
        let (run, storage) = (run.clone(), storage.clone());
        sending.spawn(async move {
            let sent = upload_blob(&run, storage.as_ref(), &blob).await;
            sent.err().map(|failure| Missing {
                part: format!("shared file {}", blob.id),
                reason: failure.message,
            })
        });
    }
    let sent: Vec<Option<Missing>> = sending.join_all().await;
    Ok(sent.into_iter().flatten().collect())
}

async fn upload_blob(run: &Run, storage: Option<&Storage>, blob: &Blob) -> Result<(), Failure> {
    let storage =
        storage.ok_or_else(|| Failure::new("this installation has no shared-file volume"))?;
    let size = i64::try_from(blob.bytes).unwrap_or(i64::MAX);
    let file = storage.read(blob.id, size).await.map_err(|error| {
        Failure::caused("it could not be read from the shared-file volume", error)
    })?;
    let key = blob_key(blob.id);
    let _permit = run.permits.acquire().await;
    let (_, sha256) = run.bucket.upload(&key, chunks(file), &run.pace).await?;
    if hex(&sha256) != blob.sha256 {
        let _ = run.bucket.delete(&key).await;
        return Err(Failure::new(
            "its bytes on the volume no longer match their checksum",
        ));
    }
    Ok(())
}

/// Asks every agent that is not stopped or retired for its workspace, which each sends on its
/// own at low priority, and waits for them all.
async fn workspaces(run: &Run, backups: &Backups) -> Result<(Vec<Part>, Vec<Missing>), Failure> {
    let pool = &backups.0.pool;
    sqlx::query!(
        "
        INSERT INTO backup_workspaces (backup_id, agent_id, status, error)
        SELECT $1, id,
            CASE WHEN desired_state='stopped' THEN 'missing' ELSE 'requested' END,
            CASE WHEN desired_state='stopped' THEN 'The agent is stopped, so it could not send its workspace.' END
        FROM agents WHERE desired_state <> 'retired'
        ",
        run.id,
    )
    .execute(pool)
    .await?;
    let started = tokio::time::Instant::now();
    loop {
        let late = started.elapsed() > CONNECT;
        let over = started.elapsed() > WORKSPACES;
        let waiting = sqlx::query_scalar!(
            r#"
            WITH late AS (
                UPDATE backup_workspaces SET status='missing',
                    error=CASE WHEN status='requested'
                        THEN 'The agent did not start sending its workspace within 10 minutes; it may be offline.'
                        ELSE 'The workspace did not finish within 12 hours.' END
                WHERE backup_id=$1 AND ((status='requested' AND $2) OR (status='uploading' AND $3))
                RETURNING agent_id
            )
            SELECT count(*) AS "waiting!" FROM backup_workspaces
            WHERE backup_id=$1 AND status IN ('requested', 'uploading')
                AND agent_id NOT IN (SELECT agent_id FROM late)
            "#,
            run.id,
            late,
            over,
        )
        .fetch_one(pool)
        .await?;
        if waiting == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let rows = sqlx::query!(
        r#"
        SELECT w.agent_id, a.name, w.status, w.bytes, w.sha256, w.error
        FROM backup_workspaces w JOIN agents a ON a.id=w.agent_id
        WHERE w.backup_id=$1 ORDER BY a.name
        "#,
        run.id,
    )
    .fetch_all(pool)
    .await?;
    let (mut parts, mut missing) = (Vec::new(), Vec::new());
    for row in rows {
        match (row.status.as_str(), row.bytes, row.sha256) {
            ("done", Some(bytes), Some(sha256)) => parts.push(Part {
                kind: "workspace".into(),
                agent: Some(row.agent_id),
                key: super::workspace_key(&run.id, row.agent_id),
                bytes: u64::try_from(bytes).unwrap_or_default(),
                sha256: hex(&sha256),
            }),
            _ => missing.push(Missing {
                part: format!("{}'s workspace", row.name),
                reason: row.error.unwrap_or_else(|| "It was not received.".into()),
            }),
        }
    }
    Ok((parts, missing))
}

/// Reads the manifest back, checks every part and blob is in the bucket at its size, and re-reads
/// a few in full to compare their SHA-256.
async fn verify(run: &Run, manifest: &Manifest, written: &[u8]) -> Result<(), Failure> {
    let read = run.bucket.bytes(&manifest_key(&manifest.id)).await?;
    if read.as_ref() != written {
        return Err(Failure::new(
            "Verification failed: the manifest read back differs from the one written.",
        ));
    }
    let mut listed: HashMap<String, u64> = run
        .bucket
        .list(&format!("backups/{}/", manifest.id))
        .await?
        .into_iter()
        .collect();
    listed.extend(run.bucket.list("blobs/").await?);
    let mut checked: Vec<(String, u64, &str)> = manifest
        .parts
        .iter()
        .map(|part| (part.key.clone(), part.bytes, part.sha256.as_str()))
        .collect();
    checked.extend(
        manifest
            .blobs
            .iter()
            .map(|blob| (blob_key(blob.id), blob.bytes, blob.sha256.as_str())),
    );
    let missing = manifest
        .missing
        .iter()
        .map(|missing| missing.part.as_str())
        .collect::<Vec<_>>();
    let absent: Vec<&str> = checked
        .iter()
        .filter(|(key, bytes, _)| listed.get(key) != Some(bytes))
        .filter(|(key, _, _)| !blob_reported(key, &missing))
        .map(|(key, _, _)| key.as_str())
        .collect();
    if let Some(first) = absent.first() {
        return Err(Failure::new(format!(
            "Verification failed: {} parts are absent from the bucket or have the wrong size, such as {first}.",
            absent.len()
        )));
    }
    for (key, _, sha256) in sample(&checked) {
        let response = run.bucket.get(key).await?;
        let mut body = response.bytes_stream();
        let mut hash = Sha256::new();
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|error| {
                Failure::caused(format!("{key} could not be read back"), error.without_url())
            })?;
            run.pace.take(chunk.len()).await;
            hash.update(&chunk);
        }
        if hex(&hash.finalize()) != *sha256 {
            return Err(Failure::new(format!(
                "Verification failed: {key} read back with a different checksum."
            )));
        }
    }
    Ok(())
}

/// A blob the backup already reports missing is not looked for again.
fn blob_reported(key: &str, missing: &[&str]) -> bool {
    key.strip_prefix("blobs/").is_some_and(|id| {
        missing
            .iter()
            .any(|part| part.strip_prefix("shared file ") == Some(id))
    })
}

fn sample<T>(items: &[T]) -> Vec<&T> {
    use rand_core::RngCore;
    let mut chosen: Vec<&T> = Vec::new();
    let mut indices: Vec<usize> = (0..items.len()).collect();
    while chosen.len() < SPOT_CHECKS && !indices.is_empty() {
        let at = (rand_core::OsRng.next_u64() % indices.len() as u64) as usize;
        chosen.push(&items[indices.swap_remove(at)]);
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sample_takes_distinct_items_up_to_its_size() {
        let items = [1, 2, 3, 4, 5];
        let mut chosen: Vec<i32> = sample(&items).into_iter().copied().collect();
        chosen.sort_unstable();
        chosen.dedup();
        assert_eq!(chosen.len(), SPOT_CHECKS);
        assert_eq!(sample(&[7]).len(), 1);
        assert!(sample::<i32>(&[]).is_empty());
    }

    #[test]
    fn only_blobs_already_reported_missing_are_excused() {
        let id = Uuid::new_v4().to_string();
        let part = format!("shared file {id}");
        assert!(blob_reported(&format!("blobs/{id}"), &[part.as_str()]));
        assert!(!blob_reported(
            &format!("blobs/{}", Uuid::new_v4()),
            &[part.as_str()]
        ));
        assert!(!blob_reported("backups/x/database.dump", &[part.as_str()]));
    }
}

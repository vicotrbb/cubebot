//! Shared files (docs/adr/memory.md#orchestrator-managed-file-storage-on-a-persistent-volume and docs/adr/memory.md#native-cubebot-storage-api): bytes on an exclusively owned volume, metadata in
//! PostgreSQL. A person or an agent owns a folder tree; a share with a person, a team or an
//! agent grants read or write on a file and everything under it. People also reach an agent's
//! files through `workspace.read` and `workspace.write` on it, and administrators read every
//! file (docs/adr/identity.md#administrator-content-access). Callers authenticate; this module authorizes every operation.
//!
//! An upload is staged, synchronized and hard linked into place before its rows commit, so a row
//! never names missing bytes; a crash in between leaves an unreferenced blob that [`Storage::start`]
//! removes before anything is served. The client retries with the same request id.
mod volume;

use crate::{agents::AgentId, identity::Source};
use axum::body::Bytes;
use cubebot_protocol::runtime::{FileAccess, FileEntry, FileShare, ShareKind, SharePermission};
use futures_util::Stream;
use sqlx::PgPool;
use std::{
    collections::HashSet,
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;
use volume::{Published, Volume};

/// Concurrent uploads and downloads; another is refused with a retry rather than queued.
const TRANSFERS: usize = 16;
const DEFAULT_QUOTA: u64 = 10 << 30;

#[derive(Clone, Copy, Debug)]
pub enum Principal {
    Person(Uuid),
    Agent(AgentId),
}

impl Principal {
    pub(crate) fn parts(self) -> (Option<Uuid>, Option<Uuid>) {
        match self {
            Self::Person(id) => (Some(id), None),
            Self::Agent(id) => (None, Some(id.0)),
        }
    }
}

#[derive(Debug)]
pub enum StorageError {
    /// Why the request was refused, for the caller to read.
    Invalid(&'static str),
    Forbidden,
    NotFound,
    /// The name is taken in that folder.
    Conflict,
    Busy,
    /// The installation quota is used up.
    Full,
    /// The request body failed before it was complete; nothing was stored.
    Incomplete(Source),
    /// A recorded blob's bytes are missing or the wrong size.
    Integrity(Uuid, Source),
    /// No storage volume is configured.
    Unavailable,
    Io(io::Error),
    Database(sqlx::Error),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(reason) => formatter.write_str(reason),
            Self::Forbidden => formatter.write_str("forbidden"),
            Self::NotFound => formatter.write_str("not found"),
            Self::Conflict => formatter.write_str("name taken"),
            Self::Busy => formatter.write_str("storage busy"),
            Self::Full => formatter.write_str("storage quota exhausted"),
            Self::Incomplete(_) => formatter.write_str("upload incomplete"),
            Self::Integrity(blob, _) => write!(formatter, "blob {blob} failed its integrity check"),
            Self::Unavailable | Self::Io(_) | Self::Database(_) => {
                formatter.write_str("storage unavailable")
            }
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Incomplete(error) | Self::Integrity(_, error) => Some(error.as_ref()),
            Self::Io(error) => Some(error),
            Self::Database(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for StorageError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<sqlx::Error> for StorageError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

/// A unique name taken is a conflict and a vanished parent a missing file.
fn constrained(error: sqlx::Error) -> StorageError {
    match error.as_database_error() {
        Some(database) if database.is_unique_violation() => StorageError::Conflict,
        Some(database) if database.is_foreign_key_violation() => StorageError::NotFound,
        _ => StorageError::Database(error),
    }
}

pub struct Upload {
    pub parent: Option<Uuid>,
    pub name: String,
    pub content_type: Option<String>,
    /// The caller's id for this upload, so a retry after a lost response finds the same file.
    pub request: Uuid,
}

struct Row {
    id: Uuid,
    parent_id: Option<Uuid>,
    name: String,
    blob_id: Option<Uuid>,
    content_type: Option<String>,
    size: Option<i64>,
    sha256: Option<Vec<u8>>,
    owner_account_id: Option<Uuid>,
    owner_agent_id: Option<Uuid>,
    updated_at: String,
}

impl Row {
    fn entry(self, access: FileAccess) -> FileEntry {
        FileEntry {
            id: self.id,
            parent: self.parent_id,
            name: self.name,
            folder: self.blob_id.is_none(),
            size: self.size,
            content_type: self.content_type,
            sha256: self
                .sha256
                .map(|digest| digest.iter().map(|byte| format!("{byte:02x}")).collect()),
            owner_user: self.owner_account_id,
            owner_agent: self.owner_agent_id,
            access,
            updated_at: self.updated_at,
        }
    }
}

type Owner = (Option<Uuid>, Option<Uuid>);

#[derive(Clone)]
pub struct Storage(Arc<Inner>);

struct Inner {
    pool: PgPool,
    volume: Volume,
    transfers: Arc<Semaphore>,
    quota: u64,
    /// Stored and staged bytes. One active orchestrator owns the volume (docs/adr/agents.md#one-active-orchestrator-with-automatic-restart).
    used: Mutex<u64>,
}

impl Storage {
    /// `None` without `CUBEBOT_STORAGE_DIRECTORY`, an absolute path on the volume.
    /// `CUBEBOT_STORAGE_QUOTA_BYTES` caps stored and staged bytes, 10 GiB by default.
    pub async fn environment(pool: PgPool) -> Result<Option<Self>, Source> {
        let Some(directory) = std::env::var_os("CUBEBOT_STORAGE_DIRECTORY").map(PathBuf::from)
        else {
            return Ok(None);
        };
        if !directory.is_absolute() {
            return Err("CUBEBOT_STORAGE_DIRECTORY must be an absolute path".into());
        }
        let quota = match std::env::var("CUBEBOT_STORAGE_QUOTA_BYTES") {
            Ok(quota) => quota.parse()?,
            Err(std::env::VarError::NotPresent) => DEFAULT_QUOTA,
            Err(error) => return Err(error.into()),
        };
        Ok(Some(Self::start(pool, directory, quota).await?))
    }

    /// Reconciles the volume with the recorded blobs; call it before serving.
    #[tracing::instrument(skip_all)]
    pub async fn start(pool: PgPool, directory: PathBuf, quota: u64) -> Result<Self, StorageError> {
        // shortcut: holds every blob id in memory once; scan in pages past millions of files.
        let blobs = sqlx::query!(
            "
            SELECT id, size
            FROM blobs
            "
        )
        .fetch_all(&pool)
        .await?;
        let used = blobs.iter().map(|blob| blob.size as u64).sum();
        let recorded: HashSet<Uuid> = blobs.iter().map(|blob| blob.id).collect();
        let (volume, missing) = tokio::task::spawn_blocking(move || {
            let volume = Volume::new(directory)?;
            let missing = volume.reconcile(&recorded)?;
            Ok::<_, io::Error>((volume, missing))
        })
        .await
        .map_err(io::Error::other)??;
        for blob in missing {
            tracing::error!(blob = %blob, "a recorded blob has no bytes on the storage volume");
        }
        Ok(Self(Arc::new(Inner {
            pool,
            volume,
            transfers: Arc::new(Semaphore::new(TRANSFERS)),
            quota,
            used: Mutex::new(used),
        })))
    }

    fn reserve(&self, bytes: u64) -> bool {
        let mut used = self
            .0
            .used
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let admitted = used.saturating_add(bytes) <= self.0.quota;
        if admitted {
            *used += bytes;
        }
        admitted
    }

    fn release(&self, bytes: u64) {
        let mut used = self
            .0
            .used
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *used = used.saturating_sub(bytes);
    }

    fn transfer(&self) -> Result<OwnedSemaphorePermit, StorageError> {
        let transfers = self.0.transfers.clone();
        transfers
            .try_acquire_owned()
            .map_err(|_| StorageError::Busy)
    }

    /// What `who` may do with `file`, through ownership, an agent grant, administration or a
    /// share on it or a folder above it.
    async fn access(&self, file: Uuid, who: Principal) -> Result<FileAccess, StorageError> {
        let (account, agent) = who.parts();
        let row = sqlx::query!(
            r#"
            WITH RECURSIVE chain AS (
                SELECT id, parent_id FROM files WHERE id = $1
                UNION ALL
                SELECT f.id, f.parent_id FROM files f JOIN chain c ON f.id = c.parent_id
            )
            SELECT EXISTS (SELECT 1 FROM chain) AS "exists!", GREATEST(
                (SELECT 3 FROM files WHERE id = $1
                    AND (owner_account_id = $2 OR owner_agent_id = $3)),
                (SELECT MAX(CASE g.capability WHEN 'workspace.write' THEN 3 ELSE 1 END)
                    FROM files f JOIN effective_human_grants g ON g.scope_kind = 'agent'
                        AND g.scope_id = f.owner_agent_id AND g.actor_id = $2
                        AND g.capability IN ('workspace.read', 'workspace.write')
                    WHERE f.id = $1),
                (SELECT 1 FROM human_accounts
                    WHERE id = $2 AND active AND role = 'administrator'),
                (SELECT MAX(CASE s.permission WHEN 'write' THEN 2 ELSE 1 END)
                    FROM chain c JOIN file_shares s ON s.file_id = c.id
                    WHERE s.account_id = $2 OR s.agent_id = $3 OR s.team_id IN (
                        SELECT m.team_id FROM human_team_members m
                        JOIN human_teams t ON t.id = m.team_id AND t.active
                        WHERE m.account_id = $2))
            ) AS level
            "#,
            file,
            account,
            agent,
        )
        .fetch_one(&self.0.pool)
        .await?;
        match (row.exists, row.level) {
            (false, _) => Err(StorageError::NotFound),
            (_, Some(3)) => Ok(FileAccess::Manage),
            (_, Some(2)) => Ok(FileAccess::Write),
            (_, Some(1)) => Ok(FileAccess::Read),
            _ => Err(StorageError::Forbidden),
        }
    }

    async fn require(
        &self,
        file: Uuid,
        who: Principal,
        needed: FileAccess,
    ) -> Result<FileAccess, StorageError> {
        let access = self.access(file, who).await?;
        if access < needed {
            return Err(StorageError::Forbidden);
        }
        Ok(access)
    }

    /// Folders first, then by name.
    async fn rows(&self, ids: &[Uuid]) -> Result<Vec<Row>, StorageError> {
        Ok(sqlx::query_as!(
            Row,
            r#"
            SELECT f.id, f.parent_id, f.name, f.blob_id, f.content_type,
                b.size AS "size?", b.sha256 AS "sha256?", f.owner_account_id, f.owner_agent_id,
                (to_jsonb(f.updated_at) #>> '{}') AS "updated_at!"
            FROM files f LEFT JOIN blobs b ON b.id = f.blob_id
            WHERE f.id = ANY($1)
            ORDER BY f.blob_id IS NOT NULL, f.name
            "#,
            ids,
        )
        .fetch_all(&self.0.pool)
        .await?)
    }

    async fn row(&self, id: Uuid) -> Result<Row, StorageError> {
        self.rows(&[id]).await?.pop().ok_or(StorageError::NotFound)
    }

    async fn entry(&self, id: Uuid, who: Principal) -> Result<FileEntry, StorageError> {
        let access = self.access(id, who).await?;
        Ok(self.row(id).await?.entry(access))
    }

    /// A folder's contents, or `who`'s own top level for `None`.
    #[tracing::instrument(skip_all)]
    pub async fn list(
        &self,
        who: Principal,
        parent: Option<Uuid>,
    ) -> Result<Vec<FileEntry>, StorageError> {
        // shortcut: no paging; add a cursor once folders hold thousands of entries.
        let (access, ids) = match parent {
            Some(parent) => {
                let access = self.require(parent, who, FileAccess::Read).await?;
                let ids = sqlx::query_scalar!(
                    "
                    SELECT id
                    FROM files
                    WHERE parent_id=$1
                    ",
                    parent,
                )
                .fetch_all(&self.0.pool)
                .await?;
                (access, ids)
            }
            None => {
                let (account, agent) = who.parts();
                let ids = sqlx::query_scalar!(
                    "
                    SELECT id
                    FROM files
                    WHERE parent_id IS NULL AND (owner_account_id=$1 OR owner_agent_id=$2)
                    ",
                    account,
                    agent,
                )
                .fetch_all(&self.0.pool)
                .await?;
                (FileAccess::Manage, ids)
            }
        };
        let rows = self.rows(&ids).await?;
        Ok(rows.into_iter().map(|row| row.entry(access)).collect())
    }

    /// Files and folders shared with `who` directly or through a team, that it does not own.
    #[tracing::instrument(skip_all)]
    pub async fn shared(&self, who: Principal) -> Result<Vec<FileEntry>, StorageError> {
        let (account, agent) = who.parts();
        let ids = sqlx::query_scalar!(
            "
            SELECT DISTINCT s.file_id
            FROM file_shares s JOIN files f ON f.id = s.file_id
            WHERE (s.account_id = $1 OR s.agent_id = $2 OR s.team_id IN (
                    SELECT m.team_id FROM human_team_members m
                    JOIN human_teams t ON t.id = m.team_id AND t.active
                    WHERE m.account_id = $1))
                AND (f.owner_account_id = $1) IS NOT TRUE AND (f.owner_agent_id = $2) IS NOT TRUE
            ",
            account,
            agent,
        )
        .fetch_all(&self.0.pool)
        .await?;
        let mut entries = Vec::new();
        // shortcut: one access query per shared item; fold it into the listing if lists grow.
        for row in self.rows(&ids).await? {
            let access = self.access(row.id, who).await?;
            entries.push(row.entry(access));
        }
        Ok(entries)
    }

    /// The owner of what goes into `parent`, which `who` may write; `who` at its top level.
    async fn destination(
        &self,
        who: Principal,
        parent: Option<Uuid>,
    ) -> Result<Owner, StorageError> {
        let Some(parent) = parent else {
            return Ok(who.parts());
        };
        self.require(parent, who, FileAccess::Write).await?;
        let row = self.row(parent).await?;
        if row.blob_id.is_some() {
            return Err(StorageError::Invalid("the parent is a file, not a folder"));
        }
        Ok((row.owner_account_id, row.owner_agent_id))
    }

    #[tracing::instrument(skip_all)]
    pub async fn create_folder(
        &self,
        who: Principal,
        parent: Option<Uuid>,
        name: &str,
    ) -> Result<FileEntry, StorageError> {
        named(name)?;
        let (account, agent) = self.destination(who, parent).await?;
        let id = Uuid::new_v4();
        sqlx::query!(
            "
            INSERT INTO files (id, parent_id, owner_account_id, owner_agent_id, name)
            VALUES ($1, $2, $3, $4, $5)
            ",
            id,
            parent,
            account,
            agent,
            name,
        )
        .execute(&self.0.pool)
        .await
        .map_err(constrained)?;
        self.entry(id, who).await
    }

    /// Streams `body` onto the volume and records it, returning it and whether this call made
    /// it: an earlier upload with the same request id returns its file without reading `body`.
    #[tracing::instrument(skip_all, fields(request = %upload.request))]
    pub async fn upload<S, E>(
        &self,
        who: Principal,
        upload: &Upload,
        body: S,
    ) -> Result<(FileEntry, bool), StorageError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: Into<Source>,
    {
        named(&upload.name)?;
        let existing = sqlx::query_scalar!(
            "
            SELECT id
            FROM files
            WHERE request_id=$1
            ",
            upload.request,
        )
        .fetch_optional(&self.0.pool)
        .await?;
        if let Some(existing) = existing {
            return Ok((self.entry(existing, who).await?, false));
        }
        let _transfer = self.transfer()?;
        let owner = self.destination(who, upload.parent).await?;
        let id = Uuid::new_v4();
        let mut reserved = 0;
        let received = self
            .0
            .volume
            .receive(id, body, |bytes| {
                let admitted = self.reserve(bytes);
                if admitted {
                    reserved += bytes;
                }
                admitted
            })
            .await;
        let recorded = match received {
            Ok(published) => self.record(id, owner, upload, &published).await,
            Err(error) => Err(error),
        };
        if let Err(error) = recorded {
            self.0.volume.remove(id).await;
            self.release(reserved);
            return Err(error);
        }
        Ok((self.entry(id, who).await?, true))
    }

    /// Commits a published blob and its file; the blob shares the file's id.
    async fn record(
        &self,
        id: Uuid,
        (account, agent): Owner,
        upload: &Upload,
        published: &Published,
    ) -> Result<(), StorageError> {
        let mut tx = self.0.pool.begin().await?;
        sqlx::query!(
            "
            INSERT INTO blobs (id, size, sha256)
            VALUES ($1, $2, $3)
            ",
            id,
            published.size as i64,
            &published.sha256[..],
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "
            INSERT INTO files (id, parent_id, owner_account_id, owner_agent_id, name, blob_id,
                content_type, request_id)
            VALUES ($1, $2, $3, $4, $5, $1, $6, $7)
            ",
            id,
            upload.parent,
            account,
            agent,
            upload.name,
            media_type(upload.content_type.as_deref()),
            upload.request,
        )
        .execute(&mut *tx)
        .await
        .map_err(constrained)?;
        tx.commit().await?;
        Ok(())
    }

    /// A file's bytes for `who`, holding a transfer slot until the returned permit drops.
    #[tracing::instrument(skip_all, fields(file = %file))]
    pub async fn open(
        &self,
        who: Principal,
        file: Uuid,
    ) -> Result<(FileEntry, tokio::fs::File, OwnedSemaphorePermit), StorageError> {
        let access = self.require(file, who, FileAccess::Read).await?;
        let row = self.row(file).await?;
        let (Some(blob), Some(size)) = (row.blob_id, row.size) else {
            return Err(StorageError::Invalid("a folder has no content"));
        };
        let transfer = self.transfer()?;
        let bytes = self.0.volume.open(blob, size).await?;
        Ok((row.entry(access), bytes, transfer))
    }

    /// Renames and moves a file or folder among its owner's folders.
    #[tracing::instrument(skip_all, fields(file = %file))]
    pub async fn place(
        &self,
        who: Principal,
        file: Uuid,
        parent: Option<Uuid>,
        name: &str,
    ) -> Result<FileEntry, StorageError> {
        named(name)?;
        self.require(file, who, FileAccess::Write).await?;
        let row = self.row(file).await?;
        // No parent is the owner's top level, which may not be `who`'s.
        let owner = (row.owner_account_id, row.owner_agent_id);
        if parent.is_some() && self.destination(who, parent).await? != owner {
            return Err(StorageError::Invalid(
                "a file moves only among its owner's folders",
            ));
        }
        let placed = sqlx::query!(
            "
            WITH RECURSIVE chain AS (
                SELECT id, parent_id FROM files WHERE id = $2
                UNION ALL
                SELECT f.id, f.parent_id FROM files f JOIN chain c ON f.id = c.parent_id
            )
            UPDATE files
            SET parent_id=$2, name=$3, updated_at=clock_timestamp()
            WHERE id=$1 AND NOT EXISTS (SELECT 1 FROM chain WHERE id = $1)
            ",
            file,
            parent,
            name,
        )
        .execute(&self.0.pool)
        .await
        .map_err(constrained)?;
        if placed.rows_affected() == 0 {
            return Err(StorageError::Invalid("a folder cannot move into itself"));
        }
        self.entry(file, who).await
    }

    /// Keeps a stream as a new blob no file names, for another table to refer to.
    #[tracing::instrument(skip_all)]
    pub async fn keep<S, E>(&self, body: S) -> Result<crate::knowledge::KeptBlob, StorageError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: Into<Source>,
    {
        let _transfer = self.transfer()?;
        let id = Uuid::new_v4();
        let mut reserved = 0;
        let received = self
            .0
            .volume
            .receive(id, body, |bytes| {
                let admitted = self.reserve(bytes);
                if admitted {
                    reserved += bytes;
                }
                admitted
            })
            .await;
        let recorded = match received {
            Ok(published) => sqlx::query!(
                "
                INSERT INTO blobs (id, size, sha256)
                VALUES ($1, $2, $3)
                ",
                id,
                published.size as i64,
                &published.sha256[..],
            )
            .execute(&self.0.pool)
            .await
            .map(|_| published.size as i64)
            .map_err(StorageError::from),
            Err(error) => Err(error),
        };
        match recorded {
            Ok(size) => Ok(crate::knowledge::KeptBlob { blob: id, size }),
            Err(error) => {
                self.0.volume.remove(id).await;
                self.release(reserved);
                Err(error)
            }
        }
    }

    /// A kept blob's bytes, holding a transfer slot until they are served.
    pub async fn read(
        &self,
        blob: Uuid,
        size: i64,
    ) -> Result<(tokio::fs::File, OwnedSemaphorePermit), StorageError> {
        let transfer = self.transfer()?;
        Ok((self.0.volume.open(blob, size).await?, transfer))
    }

    /// Deletes kept blobs nothing refers to any more.
    pub async fn discard(&self, blobs: &[Uuid]) -> Result<(), StorageError> {
        let gone = sqlx::query!(
            "
            DELETE FROM blobs
            WHERE id = ANY($1)
            RETURNING id, size
            ",
            blobs,
        )
        .fetch_all(&self.0.pool)
        .await?;
        for blob in gone {
            self.0.volume.remove(blob.id).await;
            self.release(blob.size as u64);
        }
        Ok(())
    }

    /// Deletes a file, or a folder with everything under it, then unlinks the bytes.
    #[tracing::instrument(skip_all, fields(file = %file))]
    pub async fn delete(&self, who: Principal, file: Uuid) -> Result<(), StorageError> {
        self.require(file, who, FileAccess::Write).await?;
        let blobs = sqlx::query!(
            "
            WITH RECURSIVE tree AS (
                SELECT id FROM files WHERE id = $1
                UNION ALL
                SELECT f.id FROM files f JOIN tree t ON f.parent_id = t.id
            ), gone AS (
                DELETE FROM files WHERE id IN (SELECT id FROM tree) RETURNING blob_id
            )
            DELETE FROM blobs WHERE id IN (SELECT blob_id FROM gone)
            RETURNING id, size
            ",
            file,
        )
        .fetch_all(&self.0.pool)
        .await?;
        for blob in blobs {
            self.0.volume.remove(blob.id).await;
            self.release(blob.size as u64);
        }
        Ok(())
    }

    /// The file's own shares, for whoever manages it.
    #[tracing::instrument(skip_all, fields(file = %file))]
    pub async fn shares(&self, who: Principal, file: Uuid) -> Result<Vec<FileShare>, StorageError> {
        self.require(file, who, FileAccess::Manage).await?;
        let rows = sqlx::query!(
            r#"
            SELECT s.permission, a.login AS "login?", t.name AS "team?", g.name AS "agent?"
            FROM file_shares s
            LEFT JOIN human_accounts a ON a.id = s.account_id
            LEFT JOIN human_teams t ON t.id = s.team_id
            LEFT JOIN agents g ON g.id = s.agent_id
            WHERE s.file_id = $1
            ORDER BY a.login, t.name, g.name
            "#,
            file,
        )
        .fetch_all(&self.0.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let (kind, name) = match (row.login, row.team, row.agent) {
                    (Some(login), _, _) => (ShareKind::User, login),
                    (_, Some(team), _) => (ShareKind::Team, team),
                    (_, _, agent) => (ShareKind::Agent, agent?),
                };
                let permission = match row.permission.as_str() {
                    "write" => SharePermission::Write,
                    _ => SharePermission::Read,
                };
                Some(FileShare {
                    kind,
                    name,
                    permission,
                })
            })
            .collect())
    }

    /// Adds or changes `shares`, first removing the file's others when `replace`.
    #[tracing::instrument(skip_all, fields(file = %file))]
    pub async fn share(
        &self,
        who: Principal,
        file: Uuid,
        shares: &[FileShare],
        replace: bool,
    ) -> Result<Vec<FileShare>, StorageError> {
        self.require(file, who, FileAccess::Manage).await?;
        if shares.len() > 100 {
            return Err(StorageError::Invalid("a file has at most 100 shares"));
        }
        let mut tx = self.0.pool.begin().await?;
        if replace {
            sqlx::query!(
                "
                DELETE FROM file_shares
                WHERE file_id=$1
                ",
                file,
            )
            .execute(&mut *tx)
            .await?;
        }
        for share in shares {
            let (account, team, agent) = recipient(&mut tx, share).await?;
            let permission = match share.permission {
                SharePermission::Read => "read",
                SharePermission::Write => "write",
            };
            sqlx::query!(
                "
                INSERT INTO file_shares (file_id, account_id, team_id, agent_id, permission)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (file_id, account_id, team_id, agent_id)
                DO UPDATE SET permission = EXCLUDED.permission
                ",
                file,
                account,
                team,
                agent,
                permission,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        self.shares(who, file).await
    }
}

/// The one active person, team or agent a share names.
async fn recipient(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    share: &FileShare,
) -> Result<(Option<Uuid>, Option<Uuid>, Option<Uuid>), StorageError> {
    let name = &share.name;
    let ids = match share.kind {
        ShareKind::User => {
            sqlx::query_scalar!(
                "
                SELECT id
                FROM human_accounts
                WHERE login=lower($1) AND active
                ",
                name,
            )
            .fetch_all(&mut **tx)
            .await?
        }
        ShareKind::Team => {
            sqlx::query_scalar!(
                "
                SELECT id
                FROM human_teams
                WHERE name=$1 AND active
                ",
                name,
            )
            .fetch_all(&mut **tx)
            .await?
        }
        ShareKind::Agent => {
            sqlx::query_scalar!(
                "
                SELECT id
                FROM agents
                WHERE name=$1 AND desired_state <> 'retired'
                ",
                name,
            )
            .fetch_all(&mut **tx)
            .await?
        }
    };
    let [id] = ids[..] else {
        return Err(StorageError::Invalid(
            "a share names one active person by login (not their name), one team or one agent",
        ));
    };
    Ok(match share.kind {
        ShareKind::User => (Some(id), None, None),
        ShareKind::Team => (None, Some(id), None),
        ShareKind::Agent => (None, None, Some(id)),
    })
}

pub(crate) fn named(name: &str) -> Result<(), StorageError> {
    let valid = (1..=255).contains(&name.len())
        && !matches!(name, "." | "..")
        && name.trim() == name
        && !name.contains('/')
        && !name.chars().any(char::is_control);
    if !valid {
        return Err(StorageError::Invalid(
            "a name is 1 to 255 bytes without slashes, control characters or surrounding spaces",
        ));
    }
    Ok(())
}

/// The bare `type/subtype` of a declared content type; anything else is served as octets.
pub(crate) fn media_type(declared: Option<&str>) -> Option<String> {
    let essence = declared?.split(';').next()?.trim().to_ascii_lowercase();
    let token = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&byte))
    };
    let (kind, subtype) = essence.split_once('/')?;
    (essence.len() <= 127 && token(kind) && token(subtype)).then_some(essence)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_never_become_paths() {
        assert!(named("report.pdf").is_ok());
        assert!(named("relatório final.txt").is_ok());
        for bad in [
            "",
            ".",
            "..",
            "a/b",
            "../x",
            " lead",
            "nul\0",
            "line\nbreak",
        ] {
            assert!(named(bad).is_err(), "{bad:?}");
        }
        assert!(named(&"x".repeat(256)).is_err());
    }

    #[test]
    fn only_a_plain_media_type_is_kept() {
        let kept = |declared| media_type(Some(declared));
        assert_eq!(
            kept("Text/HTML; charset=utf-8").as_deref(),
            Some("text/html")
        );
        assert_eq!(kept("application/pdf").as_deref(), Some("application/pdf"));
        assert_eq!(kept("text/html\r\nx-injected: 1"), None);
        assert_eq!(kept("nonsense"), None);
        assert_eq!(media_type(None), None);
    }
}

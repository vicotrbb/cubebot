//! Bytes on the storage volume. `staging/` holds uploads being received and `blobs/` immutable
//! published bytes named by their row's id. Both are on one filesystem, so publication is a hard
//! link, which never replaces an existing blob, followed by synchronizing the directory.
use super::StorageError;
use axum::body::Bytes;
use futures_util::{Stream, StreamExt};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

pub(super) struct Volume {
    root: PathBuf,
}

pub(super) struct Published {
    pub size: u64,
    pub sha256: [u8; 32],
}

impl Volume {
    pub fn new(root: PathBuf) -> io::Result<Self> {
        std::fs::create_dir_all(root.join("staging"))?;
        std::fs::create_dir_all(root.join("blobs"))?;
        Ok(Self { root })
    }

    fn staging(&self, id: Uuid) -> PathBuf {
        self.root.join("staging").join(id.to_string())
    }

    // shortcut: one flat directory of blobs; fan out into prefix directories once a volume
    // holds enough files for directory lookups to show in transfer latency.
    fn blob(&self, id: Uuid) -> PathBuf {
        self.root.join("blobs").join(id.to_string())
    }

    /// Streams `body` into staging, then publishes it as blob `id` once its data is on disk.
    /// `reserve` admits each chunk against the quota. Nothing named `id` remains on failure.
    pub async fn receive<S, E>(
        &self,
        id: Uuid,
        body: S,
        reserve: impl FnMut(u64) -> bool,
    ) -> Result<Published, StorageError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: Into<crate::identity::Source>,
    {
        let staging = self.staging(id);
        let published = self.publish(&staging, id, body, reserve).await;
        // The staging name goes either way; a published blob keeps its own link.
        let _ = tokio::fs::remove_file(&staging).await;
        if published.is_err() {
            self.remove(id).await;
        }
        published
    }

    async fn publish<S, E>(
        &self,
        staging: &Path,
        id: Uuid,
        mut body: S,
        mut reserve: impl FnMut(u64) -> bool,
    ) -> Result<Published, StorageError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: Into<crate::identity::Source>,
    {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(staging)
            .await?;
        let (mut hash, mut size) = (Sha256::new(), 0);
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|error| StorageError::Incomplete(error.into()))?;
            if !reserve(chunk.len() as u64) {
                drain(&mut body).await;
                return Err(StorageError::Full);
            }
            hash.update(&chunk);
            size += chunk.len() as u64;
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        file.sync_all().await?;
        tokio::fs::hard_link(staging, self.blob(id)).await?;
        sync_directory(self.root.join("blobs")).await?;
        Ok(Published {
            size,
            sha256: hash.finalize().into(),
        })
    }

    /// A ready blob's bytes; missing or resized bytes are an integrity fault, never empty content.
    pub async fn open(&self, id: Uuid, size: i64) -> Result<tokio::fs::File, StorageError> {
        let opened = self.sized(id, size).await;
        if let Err(StorageError::Integrity(_, error)) = &opened {
            tracing::error!(?error, blob = %id, "shared file integrity fault");
        }
        opened
    }

    async fn sized(&self, id: Uuid, size: i64) -> Result<tokio::fs::File, StorageError> {
        let file = tokio::fs::File::open(self.blob(id)).await;
        let file = file.map_err(|error| StorageError::Integrity(id, error.into()))?;
        let length = file.metadata().await?.len();
        if i64::try_from(length).ok() != Some(size) {
            let error = format!("{length} bytes on the volume, {size} recorded");
            return Err(StorageError::Integrity(id, error.into()));
        }
        Ok(file)
    }

    /// Unlinks a blob whose row is gone; a failure leaves it for startup reconciliation.
    pub async fn remove(&self, id: Uuid) {
        match tokio::fs::remove_file(self.blob(id)).await {
            Err(error) if error.kind() != io::ErrorKind::NotFound => {
                tracing::warn!(?error, blob = %id, "unreferenced blob left for reconciliation");
            }
            _ => {}
        }
    }

    /// Before serving: drops every staged upload and every blob without a row, which a crash
    /// between publication and commit leaves, and returns the recorded blobs with no bytes.
    pub fn reconcile(&self, recorded: &HashSet<Uuid>) -> io::Result<Vec<Uuid>> {
        for entry in std::fs::read_dir(self.root.join("staging"))? {
            std::fs::remove_file(entry?.path())?;
        }
        let mut present = HashSet::new();
        for entry in std::fs::read_dir(self.root.join("blobs"))? {
            let path = entry?.path();
            let id = path
                .file_name()
                .and_then(|name| name.to_str()?.parse().ok());
            match id {
                Some(id) if recorded.contains(&id) => {
                    present.insert(id);
                }
                Some(_) => std::fs::remove_file(&path)?,
                None => tracing::warn!(path = %path.display(), "unexpected file in blobs"),
            }
        }
        Ok(recorded.difference(&present).copied().collect())
    }
}

// shortcut: a refused upload is read out up to 64 MiB or 30 seconds; serve HTTP/2, whose
// streams end without closing the connection, if larger uploads must still see their 507.
const DRAIN: u64 = 64 * 1024 * 1024;

/// Reads out the rest of a refused upload, since an HTTP/1 server that answers before the body
/// ends closes the connection, and a client still sending then sees a reset, not the answer.
async fn drain<S, E>(body: &mut S)
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
{
    let mut left = DRAIN;
    let reading = async {
        while let Some(Ok(chunk)) = body.next().await {
            let Some(rest) = left.checked_sub(chunk.len() as u64) else {
                return;
            };
            left = rest;
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(30), reading).await;
}

async fn sync_directory(path: PathBuf) -> io::Result<()> {
    tokio::task::spawn_blocking(move || std::fs::File::open(path)?.sync_all())
        .await
        .map_err(io::Error::other)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_refused_upload_is_read_out_and_leaves_nothing() {
        let root = std::env::temp_dir().join(format!("cubebot-volume-{}", Uuid::new_v4()));
        let volume = Volume::new(root.clone()).unwrap();
        let chunks = (0..3).map(|_| Ok::<_, io::Error>(Bytes::from_static(b"bytes")));
        let mut body = futures_util::stream::iter(chunks);
        let refused = volume.receive(Uuid::new_v4(), &mut body, |_| false).await;
        assert!(matches!(refused, Err(StorageError::Full)));
        assert!(
            body.next().await.is_none(),
            "the rest of the body was left unread"
        );
        assert_eq!(std::fs::read_dir(root.join("staging")).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(root.join("blobs")).unwrap().count(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}

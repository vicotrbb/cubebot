mod capture;
mod plan;
mod render;

pub(crate) use capture::Capture;

use crate::identity::{Actor, IdentityError};
use crate::meetings::live::wire::Stream;
use crate::storage::{Storage, StorageError};
use capture::Recorder;
use render::{Job, Tools};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;
use tokio::sync::{Notify, mpsc};
use uuid::Uuid;

pub(super) const MARKS: &str = "marks.jsonl";
const QUEUE: usize = 4096;
const DIRECTORY: &str = "recordings";
const IDLE: Duration = Duration::from_secs(60);
const TICK: Duration = Duration::from_secs(2);
const ERROR_CHARS: usize = 300;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mark", rename_all = "snake_case")]
pub(crate) enum Mark {
    Name {
        at: u64,
        slot: u16,
        name: String,
    },
    Voice {
        at: u64,
        slot: u16,
        rate: u32,
        file: String,
        offset: u64,
    },
    Loud {
        at: u64,
        slot: u16,
    },
    Video {
        at: u64,
        slot: u16,
        stream: Stream,
        file: String,
    },
    Desktop {
        at: u64,
        slot: u16,
        file: String,
    },
    Stop {
        at: u64,
        file: String,
    },
    End {
        at: u64,
    },
}

impl Mark {
    fn at(&self) -> u64 {
        match self {
            Self::Name { at, .. }
            | Self::Voice { at, .. }
            | Self::Loud { at, .. }
            | Self::Video { at, .. }
            | Self::Desktop { at, .. }
            | Self::Stop { at, .. }
            | Self::End { at } => *at,
        }
    }
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = MeetingRecording)]
pub struct Recording {
    pub id: Uuid,
    pub state: String,
    pub progress: i16,
    pub started_by: String,
    pub started_at: String,
    pub stopped_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub file: Option<Uuid>,
    pub size: Option<i64>,
    pub error: Option<String>,
}

#[derive(Clone, Default)]
pub struct Recordings(Option<Arc<Inner>>);

struct Inner {
    pool: PgPool,
    storage: Storage,
    tools: Tools,
    wake: Notify,
}

impl Recordings {
    pub fn new(pool: PgPool, storage: Option<Storage>) -> Self {
        let inner = storage.and_then(|storage| {
            Tools::detect().map(|tools| Inner {
                pool,
                storage,
                tools,
                wake: Notify::new(),
            })
        });
        Self(inner.map(Arc::new))
    }

    pub fn available(&self) -> bool {
        self.0.is_some()
    }

    pub(crate) fn start(&self, meeting: Uuid, by: Uuid) -> Option<(Uuid, mpsc::Sender<Capture>)> {
        let inner = self.0.clone()?;
        let id = Uuid::new_v4();
        let (sender, receiver) = mpsc::channel(QUEUE);
        tokio::spawn(record(inner, id, meeting, by, receiver));
        Some((id, sender))
    }

    pub(crate) fn stopped(&self, id: Uuid, by: Uuid) {
        let Some(inner) = self.0.clone() else {
            return;
        };
        tokio::spawn(async move {
            let saved = sqlx::query!(
                "UPDATE meeting_recordings SET stopped_by=$2 WHERE id=$1",
                id,
                by,
            )
            .execute(&inner.pool)
            .await;
            if let Err(error) = saved {
                tracing::warn!(?error, recording = %id, "who stopped the recording is unrecorded");
            }
        });
    }

    #[tracing::instrument(skip_all)]
    pub async fn resume(&self) -> Result<(), sqlx::Error> {
        let Some(inner) = self.0.clone() else {
            return Ok(());
        };
        sqlx::query!(
            "
            UPDATE meeting_recordings
            SET state='processing', stopped_at=clock_timestamp()
            WHERE state='recording'
            "
        )
        .execute(&inner.pool)
        .await?;
        let kept: HashSet<Uuid> = sqlx::query_scalar!(
            "SELECT id FROM meeting_recordings WHERE state IN ('processing', 'failed')"
        )
        .fetch_all(&inner.pool)
        .await?
        .into_iter()
        .collect();
        sweep(inner.storage.scratch(DIRECTORY), &kept).await;
        tokio::spawn(work(inner));
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, recording = %id))]
    pub async fn retry(&self, actor: &Actor, meeting: Uuid, id: Uuid) -> Result<(), IdentityError> {
        let inner = self.0.as_ref().ok_or(IdentityError::NotFound)?;
        crate::meetings::attendee(&inner.pool, actor.id, meeting)
            .await?
            .ok_or(IdentityError::NotFound)?;
        let retried = sqlx::query!(
            "
            UPDATE meeting_recordings
            SET state='processing', progress=0, error=NULL, finished_at=NULL
            WHERE id=$1 AND meeting_id=$2 AND state='failed'
            ",
            id,
            meeting,
        )
        .execute(&inner.pool)
        .await?;
        if retried.rows_affected() == 0 {
            return Err(IdentityError::Conflict);
        }
        inner.wake.notify_one();
        Ok(())
    }
}

pub(crate) async fn listed(pool: &PgPool, meeting: Uuid) -> Result<Vec<Recording>, sqlx::Error> {
    sqlx::query_as!(
        Recording,
        r#"
        SELECT r.id, r.state, r.progress,
            COALESCE(NULLIF(h.profile->>'name', ''), h.login) AS "started_by!",
            (to_jsonb(r.started_at) #>> '{}') AS "started_at!",
            (to_jsonb(r.stopped_at) #>> '{}') AS stopped_at,
            r.duration_ms, f.id AS "file?", b.size AS "size?", r.error
        FROM meeting_recordings r
        JOIN human_accounts h ON h.id=r.started_by
        LEFT JOIN files f ON f.id=r.file_id
        LEFT JOIN blobs b ON b.id=f.blob_id
        WHERE r.meeting_id=$1
        ORDER BY r.started_at
        "#,
        meeting,
    )
    .fetch_all(pool)
    .await
}

async fn sweep(root: PathBuf, kept: &HashSet<Uuid>) {
    let Ok(mut entries) = tokio::fs::read_dir(&root).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let id = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok());
        if id.is_none_or(|id| !kept.contains(&id))
            && let Err(error) = tokio::fs::remove_dir_all(entry.path()).await
        {
            tracing::warn!(%error, path = %entry.path().display(), "stale recording files left");
        }
    }
}

#[tracing::instrument(skip_all, fields(recording = %id, meeting = %meeting))]
async fn record(
    inner: Arc<Inner>,
    id: Uuid,
    meeting: Uuid,
    by: Uuid,
    captures: mpsc::Receiver<Capture>,
) {
    let inserted = sqlx::query!(
        "INSERT INTO meeting_recordings (id, meeting_id, started_by) VALUES ($1, $2, $3)",
        id,
        meeting,
        by,
    )
    .execute(&inner.pool)
    .await;
    if let Err(error) = inserted {
        tracing::error!(?error, "recording could not start");
        return;
    }
    let directory = inner.storage.scratch(DIRECTORY).join(id.to_string());
    let captured =
        tokio::task::spawn_blocking(move || Recorder::create(&directory)?.run(captures)).await;
    match captured {
        Ok(Ok(duration)) => tracing::info!(duration, "recording captured"),
        Ok(Err(error)) => tracing::error!(%error, "recording capture stopped early"),
        Err(error) => tracing::error!(%error, "recording capture panicked"),
    }
    let stopped = sqlx::query!(
        "
        UPDATE meeting_recordings
        SET state='processing', stopped_at=clock_timestamp()
        WHERE id=$1 AND state='recording'
        ",
        id,
    )
    .execute(&inner.pool)
    .await;
    if let Err(error) = stopped {
        tracing::error!(?error, "recording left in progress until restart");
    }
    inner.wake.notify_one();
}

async fn work(inner: Arc<Inner>) {
    loop {
        let next = sqlx::query_scalar!(
            "
            SELECT id
            FROM meeting_recordings
            WHERE state='processing'
            ORDER BY stopped_at, id
            LIMIT 1
            "
        )
        .fetch_optional(&inner.pool)
        .await;
        match next {
            Ok(Some(id)) => process(&inner, id).await,
            Ok(None) => {
                let _ = tokio::time::timeout(IDLE, inner.wake.notified()).await;
            }
            Err(error) => {
                tracing::warn!(?error, "recording queue unreadable");
                tokio::time::sleep(IDLE).await;
            }
        }
    }
}

struct Details {
    title: String,
    owner: Uuid,
    name: String,
    readers: Vec<Uuid>,
}

async fn details(pool: &PgPool, id: Uuid) -> Result<Details, sqlx::Error> {
    sqlx::query_as!(
        Details,
        r#"
        SELECT m.title, m.owner_id AS owner,
            format('%s, %s.mp4', left(translate(m.title, '/', '-'), 120),
                to_char(r.started_at AT TIME ZONE COALESCE(
                    (SELECT z.name FROM pg_timezone_names z WHERE z.name = o.profile->>'time_zone'),
                    'UTC'), 'YYYY-MM-DD HH24.MI')) AS "name!",
            ARRAY(SELECT i.account_id FROM meeting_invitations i
                WHERE i.meeting_id=m.id AND i.account_id IS NOT NULL) AS "readers!"
        FROM meeting_recordings r
        JOIN meetings m ON m.id=r.meeting_id
        JOIN human_accounts o ON o.id=m.owner_id
        WHERE r.id=$1
        "#,
        id,
    )
    .fetch_one(pool)
    .await
}

fn reason(error: &StorageError) -> String {
    match error {
        StorageError::Full => "Storage is full. Free some space in Files, then try again.".into(),
        StorageError::Busy => "Storage was busy. Try again in a moment.".into(),
        _ => "The recording could not be saved to Files.".into(),
    }
}

#[tracing::instrument(skip_all, fields(recording = %id))]
async fn process(inner: &Arc<Inner>, id: Uuid) {
    let outcome = match details(&inner.pool, id).await {
        Ok(details) => produce(inner, id, &details).await,
        Err(error) => {
            tracing::error!(?error, "recording details unreadable");
            Err("The meeting for this recording is unreadable.".into())
        }
    };
    let saved = match outcome {
        Ok((file, duration)) => sqlx::query!(
            "
            UPDATE meeting_recordings
            SET state='ready', progress=100, file_id=$2, duration_ms=$3, finished_at=clock_timestamp()
            WHERE id=$1
            ",
            id,
            file,
            i64::try_from(duration).unwrap_or(i64::MAX),
        )
        .execute(&inner.pool)
        .await,
        Err(error) => sqlx::query!(
            "
            UPDATE meeting_recordings
            SET state='failed', error=$2, finished_at=clock_timestamp()
            WHERE id=$1
            ",
            id,
            error.chars().take(ERROR_CHARS).collect::<String>(),
        )
        .execute(&inner.pool)
        .await,
    };
    if let Err(error) = saved {
        tracing::error!(?error, "recording outcome not saved");
        tokio::time::sleep(IDLE).await;
    }
}

async fn render(inner: &Arc<Inner>, id: Uuid, title: &str) -> Result<(PathBuf, u64), String> {
    let directory = inner.storage.scratch(DIRECTORY).join(id.to_string());
    let progress = Arc::new(AtomicU8::new(0));
    let ticker = tokio::spawn(report(inner.pool.clone(), id, progress.clone()));
    let (title, tools) = (title.to_owned(), inner.tools.clone());
    let rendered = tokio::task::spawn_blocking(move || {
        Job {
            directory: &directory,
            title: &title,
            tools: &tools,
            progress: &progress,
        }
        .run()
    })
    .await;
    ticker.abort();
    outcome(rendered)
}

fn outcome(
    rendered: Result<std::io::Result<(PathBuf, u64)>, tokio::task::JoinError>,
) -> Result<(PathBuf, u64), String> {
    let failure = match rendered {
        Ok(Ok(rendered)) => return Ok(rendered),
        Ok(Err(error)) => format!("Processing failed: {error}"),
        Err(error) => format!("Processing stopped unexpectedly: {error}"),
    };
    tracing::error!(%failure, "recording could not be processed");
    Err(failure)
}

async fn produce(inner: &Arc<Inner>, id: Uuid, details: &Details) -> Result<(Uuid, u64), String> {
    let (path, duration) = render(inner, id, &details.title).await?;
    let file = inner
        .storage
        .keep_recording(details.owner, &details.readers, &details.name, id, &path)
        .await
        .map_err(|error| {
            tracing::error!(?error, "recording could not be filed");
            reason(&error)
        })?;
    let directory = inner.storage.scratch(DIRECTORY).join(id.to_string());
    if let Err(error) = tokio::fs::remove_dir_all(&directory).await {
        tracing::warn!(%error, "processed recording files left for the next sweep");
    }
    Ok((file, duration))
}

async fn report(pool: PgPool, id: Uuid, progress: Arc<AtomicU8>) {
    let mut shown = 0;
    loop {
        tokio::time::sleep(TICK).await;
        let now = progress.load(Ordering::Relaxed);
        if now != shown {
            shown = now;
            let _ = sqlx::query!(
                "UPDATE meeting_recordings SET progress=$2 WHERE id=$1 AND state='processing'",
                id,
                i16::from(now),
            )
            .execute(&pool)
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;
    use cubebot_protocol::desktop::Tile;
    use std::process::Command;

    fn vp8_frames(directory: &std::path::Path) -> Vec<(bool, Vec<u8>)> {
        let path = directory.join("source.ivf");
        let made = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=s=320x180:r=25:d=2",
            ])
            .args(["-c:v", "libvpx", "-g", "25", "-f", "ivf", "-y"])
            .arg(&path)
            .status()
            .unwrap();
        assert!(made.success());
        let bytes = std::fs::read(path).unwrap();
        let mut frames = Vec::new();
        let mut at = 32;
        while at + 12 <= bytes.len() {
            let size = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            let chunk = bytes[at + 12..at + 12 + size].to_vec();
            frames.push((chunk[0] & 1 == 0, chunk));
            at += 12 + size;
        }
        frames
    }

    fn tile() -> String {
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, 64, 64);
        encoder.set_color(png::ColorType::Rgb);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[200; 64 * 64 * 3]).unwrap();
        writer.finish().unwrap();
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, png)
    }

    fn probe(path: &std::path::Path) -> String {
        let output = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type,width,height",
            ])
            .args(["-show_entries", "format=duration", "-of", "compact"])
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap()
    }

    #[test]
    fn a_captured_meeting_becomes_one_video_with_everyone_heard() {
        let Some(tools) = Tools::detect() else {
            return;
        };
        let directory = std::env::temp_dir().join(format!("cubebot-recording-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let frames = vp8_frames(&directory);
        let recorder = Recorder::create(&directory.join("capture")).unwrap();
        let (sender, receiver) = mpsc::channel(QUEUE);
        let capturing = std::thread::spawn(move || recorder.run(receiver));
        let send = |capture| sender.blocking_send(capture).unwrap();
        send(Capture::Name {
            slot: 1,
            name: "Ana".into(),
        });
        send(Capture::Name {
            slot: 2,
            name: "Atlas".into(),
        });
        let codec = Some(("vp8".to_owned(), 320, 180));
        send(Capture::Track {
            slot: 1,
            stream: Stream::Camera,
            codec,
        });
        let loud: Vec<u8> = std::iter::repeat_n(4000_i16.to_le_bytes(), 1600)
            .flatten()
            .collect();
        for (index, (key, chunk)) in frames.into_iter().enumerate() {
            let mut payload = vec![u8::from(key), 0, 0, 0, 0];
            payload.extend(chunk);
            let payload = Bytes::from(payload);
            send(Capture::Frame {
                slot: 1,
                stream: Stream::Camera,
                key,
                payload,
            });
            if index % 2 == 0 {
                send(Capture::Audio {
                    slot: 1,
                    rate: 16_000,
                    pcm: Bytes::from(loud.clone()),
                });
            }
            if index == 30 {
                let tiles = vec![Tile {
                    x: 0,
                    y: 0,
                    png: tile(),
                }];
                send(Capture::Tiles { slot: 2, tiles });
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        send(Capture::DesktopEnded { slot: 2 });
        drop(sender);
        let captured = capturing.join().unwrap().unwrap();
        assert!((1800..3000).contains(&captured), "{captured} ms captured");
        let progress = AtomicU8::new(0);
        let job = Job {
            directory: &directory.join("capture"),
            title: "Weekly sync",
            tools: &tools,
            progress: &progress,
        };
        let (video, duration) = job.run().unwrap();
        assert_eq!(duration, captured);
        let probed = probe(&video);
        assert!(
            probed.contains("codec_type=video|width=1280|height=720"),
            "{probed}"
        );
        assert!(probed.contains("codec_type=audio"), "{probed}");
        let seconds: f64 = probed
            .lines()
            .find_map(|line| line.strip_prefix("format|duration="))
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            (seconds * 1000.0 - captured as f64).abs() < 200.0,
            "{seconds} s"
        );
        assert_eq!(progress.load(Ordering::Relaxed), 95);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

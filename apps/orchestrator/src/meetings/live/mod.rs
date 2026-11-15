mod attendants;
mod floor;
mod hearing;
mod room;
mod screens;
mod socket;
mod voices;
pub mod wire;

pub use room::{Event, Refusal};
pub use screens::{Answer, Presenter};
pub use socket::{Entry, Recheck};

use super::summary::summarize;
use crate::agents::{Avatar, sandbox::Desktop};
use crate::connections::Connections;
use crate::speech::Speech;
use room::{Command, Joined, Joiner, Loaded, Record, Room, Seating, Who};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;
use wire::{Media, Server};

const COMMANDS: usize = 1024;
const EVENTS: usize = 1024;

#[derive(Clone)]
pub struct Live {
    pool: PgPool,
    rooms: Arc<Mutex<HashMap<Uuid, Handle>>>,
    pub(super) speech: Speech,
    connections: Option<Connections>,
    pub(super) desktop: Option<Desktop>,
}

#[derive(Clone)]
pub struct Handle {
    commands: mpsc::Sender<Command>,
    invited: mpsc::Sender<()>,
}

impl Live {
    pub(super) fn new(pool: PgPool, connections: Option<Connections>, speech: Speech) -> Self {
        Self {
            pool,
            rooms: Arc::default(),
            speech,
            connections,
            desktop: None,
        }
    }

    fn rooms(&self) -> MutexGuard<'_, HashMap<Uuid, Handle>> {
        self.rooms.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn room(&self, id: Uuid) -> Option<Handle> {
        self.rooms()
            .get(&id)
            .filter(|room| !room.commands.is_closed())
            .cloned()
    }

    pub(super) async fn relink(&self, id: Uuid, token: Option<String>) {
        if let Some(room) = self.room(id) {
            let _ = room.commands.send(Command::Link { token }).await;
        }
    }

    pub(super) fn invitations_changed(&self, id: Uuid) {
        if let Some(room) = self.room(id) {
            let _ = room.invited.try_send(());
        }
    }

    pub(super) async fn flags(&self, id: Uuid, open_join: bool, locked: bool) {
        if let Some(room) = self.room(id) {
            let flags = Command::Flags { open_join, locked };
            let _ = room.commands.send(flags).await;
        }
    }

    async fn enter(&self, id: Uuid, joiner: Joiner) -> Result<(Handle, Joined), Refusal> {
        let guest = matches!(joiner.who, Who::Guest { .. });
        let room = self.open(id, guest).await?;
        let (reply, answer) = oneshot::channel();
        let join = Command::Join { joiner, reply };
        room.commands.send(join).await.map_err(|_| Refusal::Gone)?;
        let joined = answer.await.map_err(|_| Refusal::Gone)??;
        Ok((room, joined))
    }

    async fn open(&self, id: Uuid, guest: bool) -> Result<Handle, Refusal> {
        let existing = self.rooms().get(&id).cloned();
        if let Some(room) = existing {
            return alive(room);
        }
        let loaded = load(&self.pool, id)
            .await
            .map_err(|error| {
                tracing::warn!(?error, meeting = %id, "meeting unreadable");
                Refusal::Gone
            })?
            .ok_or(Refusal::Gone)?;
        if guest && loaded.ended {
            return Err(Refusal::Ended);
        }
        if guest && loaded.locked {
            return Err(Refusal::Locked);
        }
        let mut rooms = self.rooms();
        if let Some(room) = rooms.get(&id) {
            return alive(room.clone());
        }
        let room = self.spawn(id, loaded);
        rooms.insert(id, room.clone());
        Ok(room)
    }

    fn spawn(&self, id: Uuid, loaded: Loaded) -> Handle {
        let (commands, receiver) = mpsc::channel(COMMANDS);
        let (records, stored) = mpsc::unbounded_channel();
        let (events, heard) = mpsc::channel(EVENTS);
        let (invited, invitations) = mpsc::channel(1);
        let _ = commands.try_send(Command::Subscribe { events });
        let room = Room::new(id, loaded, records);
        let handle = Handle { commands, invited };
        let live = self.clone();
        let kept = handle.clone();
        let inbox = attendants::Inbox {
            events: heard,
            invited: invitations,
        };
        let attending = tokio::spawn(attendants::attend(self.clone(), id, handle.clone(), inbox));
        tokio::spawn(async move {
            let keeping = tokio::spawn(keep(live.pool.clone(), id, stored));
            room.run(receiver).await;
            let _ = keeping.await;
            let _ = attending.await;
            live.end(id).await;
            live.forget(id, &kept);
        });
        handle
    }

    async fn end(&self, id: Uuid) {
        match end(&self.pool, id).await {
            Ok(true) => {
                let summary = summarize(self.pool.clone(), self.connections.clone(), id);
                tokio::spawn(summary);
            }
            Ok(false) => {}
            Err(error) => tracing::warn!(?error, meeting = %id, "meeting end not recorded"),
        }
    }

    fn forget(&self, id: Uuid, room: &Handle) {
        let mut rooms = self.rooms();
        if rooms
            .get(&id)
            .is_some_and(|held| held.commands.same_channel(&room.commands))
        {
            rooms.remove(&id);
        }
    }
}

impl Handle {
    fn media(&self, slot: u16, media: Media) {
        let _ = self.commands.try_send(Command::Media { slot, media });
    }

    async fn command(&self, command: Command) -> bool {
        self.commands.send(command).await.is_ok()
    }

    pub async fn subscribe(&self) -> Option<mpsc::Receiver<Event>> {
        let (events, receiver) = mpsc::channel(EVENTS);
        self.command(Command::Subscribe { events })
            .await
            .then_some(receiver)
    }

    pub async fn add_agent(&self, agent: Uuid, name: String, avatar: Avatar) -> Option<u16> {
        let (reply, answer) = oneshot::channel();
        let add = Seating::Add {
            agent,
            name,
            avatar,
            reply,
        };
        self.command(Command::Agent(add)).await;
        answer.await.ok().flatten()
    }

    pub async fn remove_agent(&self, slot: u16) -> bool {
        self.command(Command::Agent(Seating::Remove { slot })).await
    }

    pub async fn agent_audio(&self, slot: u16, rate: u32, pcm: &[u8]) -> bool {
        let frame = wire::audio(slot, rate, pcm);
        self.command(Command::Agent(Seating::Audio { slot, frame }))
            .await
    }

    pub async fn agent_screen(&self, slot: u16, sharing: bool) -> bool {
        self.command(Command::AgentScreen { slot, sharing }).await
    }

    pub async fn announce(&self, message: Server, to: Option<u16>) -> bool {
        self.command(Command::Announce { message, to }).await
    }

    pub async fn present(&self, agent: Uuid, share: bool) -> Answer {
        let (reply, answer) = oneshot::channel();
        let present = Seating::Present {
            agent,
            share,
            reply,
        };
        self.command(Command::Agent(present)).await;
        answer.await.unwrap_or(Err(Refusal::Gone.reason()))
    }

    pub async fn may_present(&self, slot: u16, agents: Vec<Uuid>) -> bool {
        self.command(Command::Agent(Seating::MayPresent { slot, agents }))
            .await
    }
}

fn alive(room: Handle) -> Result<Handle, Refusal> {
    (!room.commands.is_closed())
        .then_some(room)
        .ok_or(Refusal::Gone)
}

async fn load(pool: &PgPool, id: Uuid) -> Result<Option<Loaded>, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT m.title, m.owner_id, m.link_token, m.open_join, m.locked,
            m.ended_at IS NOT NULL AS "ended!", s.max_participants
        FROM meetings m, meeting_settings s
        WHERE m.id=$1
        "#,
        id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| Loaded {
        title: row.title,
        owner: row.owner_id,
        token: row.link_token,
        open_join: row.open_join,
        locked: row.locked,
        ended: row.ended,
        capacity: usize::try_from(row.max_participants).unwrap_or_default(),
    }))
}

async fn keep(pool: PgPool, id: Uuid, mut records: mpsc::UnboundedReceiver<Record>) {
    while let Some(record) = records.recv().await {
        if let Err(error) = store(&pool, id, record).await {
            tracing::warn!(?error, meeting = %id, "meeting record not stored");
        }
    }
}

async fn store(pool: &PgPool, id: Uuid, record: Record) -> Result<(), sqlx::Error> {
    match record {
        Record::Started => {
            sqlx::query!(
                "UPDATE meetings SET started_at=COALESCE(started_at, clock_timestamp()), ended_at=NULL WHERE id=$1",
                id,
            )
            .execute(pool)
            .await?;
        }
        Record::Joined {
            attendance,
            account,
            guest,
        } => {
            sqlx::query!(
                "INSERT INTO meeting_attendance (id, meeting_id, account_id, guest_name) VALUES ($1, $2, $3, $4)",
                attendance,
                id,
                account,
                guest,
            )
            .execute(pool)
            .await?;
        }
        Record::Left { attendance } => {
            sqlx::query!(
                "UPDATE meeting_attendance SET left_at=clock_timestamp() WHERE id=$1 AND left_at IS NULL",
                attendance,
            )
            .execute(pool)
            .await?;
        }
        Record::Locked(locked) => {
            sqlx::query!("UPDATE meetings SET locked=$2 WHERE id=$1", id, locked)
                .execute(pool)
                .await?;
        }
        Record::Audit(event) => crate::identity::audit::record(pool, &event).await?,
    }
    Ok(())
}

async fn end(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let ended = sqlx::query!(
        "
        UPDATE meetings SET ended_at=clock_timestamp()
        WHERE id=$1 AND started_at IS NOT NULL AND ended_at IS NULL
        ",
        id,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE meeting_attendance SET left_at=clock_timestamp() WHERE meeting_id=$1 AND left_at IS NULL",
        id,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(ended.rows_affected() == 1)
}

pub(super) async fn close_open(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query!(
        "
        UPDATE meetings m
        SET ended_at=GREATEST(m.started_at,
            (SELECT max(GREATEST(a.joined_at, a.left_at)) FROM meeting_attendance a WHERE a.meeting_id=m.id),
            (SELECT max(u.at) FROM meeting_utterances u WHERE u.meeting_id=m.id))
        WHERE m.started_at IS NOT NULL AND m.ended_at IS NULL
        ",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "
        UPDATE meeting_attendance a SET left_at=GREATEST(a.joined_at, m.ended_at)
        FROM meetings m
        WHERE m.id=a.meeting_id AND a.left_at IS NULL AND m.ended_at IS NOT NULL
        ",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

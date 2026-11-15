use super::room::{Command, Inbound, Joiner, Refusal, Who, close, opened};
use super::wire::{self, Budget, Client, Server};
use super::{Handle, Live};
use crate::identity::Identity;
use crate::meetings::{attendee, public::guest_name};
use axum::extract::ws::{Message, WebSocket};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use sqlx::PgPool;
use std::time::Duration;
use tokio::time::Instant;
use uuid::Uuid;

const SILENCE: Duration = Duration::from_secs(45);
const RECHECK: Duration = Duration::from_secs(30);
const DRAIN: Duration = Duration::from_secs(5);
const QUIET: &str = "The connection went quiet.";
const ACCESS_ENDED: &str = "Your access to this meeting ended.";
const HELLO: &str = "The meeting did not hear who you are.";
const NAME: &str =
    "Choose a name of 1 to 40 letters, digits, spaces, periods, apostrophes or hyphens.";

pub struct Recheck {
    pub identity: Identity,
    pub token: String,
}

pub enum Entry {
    Member {
        account: Uuid,
        name: String,
        recheck: Recheck,
    },
    Guest {
        token: String,
    },
}

struct Watch {
    recheck: Recheck,
    pool: PgPool,
    meeting: Uuid,
}

impl Watch {
    async fn allowed(&self) -> bool {
        let Ok(actor) = self
            .recheck
            .identity
            .authenticate(&self.recheck.token)
            .await
        else {
            return false;
        };
        matches!(
            attendee(&self.pool, actor.id, self.meeting).await,
            Ok(Some(_))
        )
    }
}

impl Live {
    pub async fn attend(self, mut socket: WebSocket, id: Uuid, entry: Entry) {
        let (joiner, recheck) = match introduced(&mut socket, entry).await {
            Ok(introduced) => introduced,
            Err(reason) => return refuse(socket, reason).await,
        };
        let (room, joined) = match self.enter(id, joiner).await {
            Ok(entered) => entered,
            Err(refusal) => {
                if refusal == Refusal::Ended {
                    let _ = socket.send(Server::Ended.message()).await;
                }
                return refuse(socket, refusal.reason()).await;
            }
        };
        let watch = recheck.map(|recheck| Watch {
            recheck,
            pool: self.pool.clone(),
            meeting: id,
        });
        let (sink, stream) = socket.split();
        let mut writer = tokio::spawn(write(sink, joined.inbound));
        let (reason, writing) = tokio::select! {
            reason = read(stream, &room, joined.slot, watch.as_ref()) => (reason, true),
            _ = &mut writer => (None, false),
        };
        room.command(Command::Leave {
            slot: joined.slot,
            reason,
        })
        .await;
        if writing {
            let _ = tokio::time::timeout(DRAIN, writer).await;
        }
    }
}

async fn refuse(mut socket: WebSocket, reason: &'static str) {
    let _ = socket.send(close(1008, reason)).await;
}

async fn introduced(
    socket: &mut WebSocket,
    entry: Entry,
) -> Result<(Joiner, Option<Recheck>), &'static str> {
    let named = hello(socket).await.ok_or(HELLO)?;
    match entry {
        Entry::Member {
            account,
            name,
            recheck,
        } => {
            let who = Who::Member(account);
            Ok((Joiner { who, name }, Some(recheck)))
        }
        Entry::Guest { token } => {
            let name = named.as_deref().and_then(guest_name).ok_or(NAME)?;
            let who = Who::Guest { token };
            Ok((Joiner { who, name }, None))
        }
    }
}

async fn hello(socket: &mut WebSocket) -> Option<Option<String>> {
    let first = tokio::time::timeout(SILENCE, socket.recv()).await.ok()??;
    let Ok(Message::Text(text)) = first else {
        return None;
    };
    match serde_json::from_str(&text).ok()? {
        Client::Hello { name } => Some(name),
        _ => None,
    }
}

async fn write(mut sink: SplitSink<WebSocket, Message>, inbound: Inbound) {
    let Inbound {
        mut control,
        mut media,
        roster,
    } = inbound;
    loop {
        let message = tokio::select! {
            biased;
            Some(out) = control.recv() => match opened(&roster, out) {
                Some(message) => message,
                None => continue,
            },
            Some(message) = media.recv() => message,
            else => return,
        };
        let closing = matches!(message, Message::Close(_));
        if sink.send(message).await.is_err() || closing {
            return;
        }
    }
}

async fn read(
    mut stream: SplitStream<WebSocket>,
    room: &Handle,
    slot: u16,
    watch: Option<&Watch>,
) -> Option<&'static str> {
    let mut budget = Budget::new(std::time::Instant::now());
    let mut heard = Instant::now();
    let mut checks = tokio::time::interval(RECHECK);
    checks.reset();
    loop {
        tokio::select! {
            next = stream.next() => {
                heard = Instant::now();
                let Some(Ok(message)) = next else {
                    return None;
                };
                if !forward(room, slot, message, &mut budget).await {
                    return None;
                }
            }
            () = tokio::time::sleep_until(heard + SILENCE) => return Some(QUIET),
            _ = checks.tick(), if watch.is_some() => {
                if let Some(watch) = watch && !watch.allowed().await {
                    return Some(ACCESS_ENDED);
                }
            }
        }
    }
}

async fn forward(room: &Handle, slot: u16, message: Message, budget: &mut Budget) -> bool {
    let now = std::time::Instant::now();
    match message {
        Message::Binary(frame) if budget.allows(frame.len(), now) => {
            if let Some(media) = wire::media(&frame) {
                room.media(slot, media);
            }
        }
        Message::Text(text) if budget.allows_text(text.len(), now) => {
            let said = serde_json::from_str::<Client>(&text).ok();
            if let Some(message) =
                said.filter(|said| !matches!(said, Client::Ping | Client::Hello { .. }))
            {
                room.command(Command::Said { slot, message }).await;
            }
        }
        Message::Close(_) => return false,
        _ => {}
    }
    true
}

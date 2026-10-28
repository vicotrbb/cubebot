//! The agent's own loop: one channel to the switchboard and one turn at a time. Events wait in an
//! outbox until the ledger acknowledges them, so a reconnect resends rather than loses them.
mod attempts;
mod process;
mod turn;
use crate::control::{Channel, Config, Error, RuntimeClient};
use attempts::Store;
use cubebot_protocol::runtime::{Delivery, Report, TurnEvent};
use futures_util::{SinkExt, StreamExt};
use std::{
    os::unix::fs::MetadataExt,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    signal::unix::{Signal, SignalKind},
    sync::{mpsc, watch},
};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

pub async fn run() -> Result<(), &'static str> {
    let config = Config::from_environment().map_err(|_| "invalid runtime configuration")?;
    let root =
        std::env::var_os("CUBEBOT_JOURNAL_DIRECTORY").ok_or("runtime storage not configured")?;
    let mut agent = Agent::open(Path::new(&root))?;
    let mut client = RuntimeClient::new(config).map_err(|_| "runtime transport unavailable")?;
    let mut terminate = tokio::signal::unix::signal(SignalKind::terminate())
        .map_err(|_| "runtime signal handler unavailable")?;
    loop {
        tokio::select! {
            _ = shutdown(&mut terminate) => return Ok(()),
            next = agent.session(&mut client) => match next {
                Ok(Next::Exit) => return Ok(()),
                Ok(Next::Reconnect) => (),
                Err(error) => eprintln!("Runtime control waiting: {error}"),
            }
        }
        tokio::select! {
            _ = shutdown(&mut terminate) => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(5)) => (),
        }
    }
}

async fn shutdown(terminate: &mut Signal) {
    tokio::select! {
        _ = terminate.recv() => (),
        _ = tokio::signal::ctrl_c() => (),
    }
}

enum Next {
    Exit,
    Reconnect,
}
struct Active {
    turn: Uuid,
    cancel: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}
struct Agent {
    store: Arc<Mutex<Store>>,
    outbox: Vec<turn::Event>,
    sender: mpsc::UnboundedSender<turn::Event>,
    events: mpsc::UnboundedReceiver<turn::Event>,
    bootstrapped: bool,
    active: Option<Active>,
    drain: Option<Uuid>,
}

impl Agent {
    fn open(root: &Path) -> Result<Self, &'static str> {
        let metadata =
            std::fs::symlink_metadata(root).map_err(|_| "private runtime storage unavailable")?;
        if !metadata.is_dir() || metadata.mode() & 0o077 != 0 {
            return Err("private runtime storage required");
        }
        let store = Store::open(&root.join("tools"))
            .map_err(|_| "runtime journal unavailable; recovery required")?;
        // Tool effects from before a crash are reported once, with their outcome or as uncertain.
        let outbox = store
            .pending()
            .map_err(|_| "runtime journal unreadable; recovery required")?
            .into_iter()
            .map(|(entry, journal)| (entry, Some(journal)))
            .collect();
        let (sender, events) = mpsc::unbounded_channel();
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            outbox,
            sender,
            events,
            bootstrapped: false,
            active: None,
            drain: None,
        })
    }

    async fn session(&mut self, client: &mut RuntimeClient) -> Result<Next, Error> {
        if !self.bootstrapped {
            client.bootstrap().await?;
            self.bootstrapped = true;
        }
        let mut channel = client.connect().await?;
        for (entry, _) in &self.outbox {
            send(&mut channel, &report(entry)).await?;
        }
        self.idle(&mut channel).await?;
        loop {
            tokio::select! {
                message = channel.next() => {
                    let text = match message {
                        Some(Ok(Message::Text(text))) => text,
                        Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                        _ => return Ok(Next::Reconnect),
                    };
                    let delivery = serde_json::from_str(&text).map_err(|_| Error::Protocol)?;
                    if let Some(next) = self.deliver(&mut channel, delivery).await? {
                        return Ok(next);
                    }
                }
                Some(event) = self.events.recv() => self.forward(&mut channel, event).await?,
            }
        }
    }

    async fn forward(&mut self, channel: &mut Channel, event: turn::Event) -> Result<(), Error> {
        let finished = matches!(
            event.0.event,
            TurnEvent::Done { .. } | TurnEvent::Error { .. }
        );
        let message = report(&event.0);
        self.outbox.push(event);
        send(channel, &message).await?;
        if finished {
            self.active = None;
            self.idle(channel).await?;
        }
        Ok(())
    }

    /// Without an active turn, reports a requested drain or asks for work.
    async fn idle(&self, channel: &mut Channel) -> Result<(), Error> {
        if self.active.is_some() {
            return Ok(());
        }
        let report = match self.drain {
            Some(request_id) => Report::Drained { request_id },
            None => Report::Pull,
        };
        send(channel, &report).await
    }

    async fn deliver(
        &mut self,
        channel: &mut Channel,
        delivery: Delivery,
    ) -> Result<Option<Next>, Error> {
        match delivery {
            Delivery::Turn(work) => {
                if self.active.is_some() || self.drain.is_some() {
                    return Err(Error::Protocol);
                }
                let (cancel, cancelled) = watch::channel(false);
                let turn = work.turn;
                let task = tokio::spawn(turn::run(
                    work,
                    self.sender.clone(),
                    self.store.clone(),
                    cancelled,
                ));
                self.active = Some(Active { turn, cancel, task });
            }
            Delivery::Ack { turn, sequence } => self.acknowledge(turn, sequence)?,
            Delivery::Cancel { turn } => {
                if let Some(active) = self.active.as_ref().filter(|active| active.turn == turn) {
                    let _ = active.cancel.send(true);
                }
            }
            Delivery::Drain { request_id } => {
                self.drain = Some(request_id);
                self.idle(channel).await?;
            }
            Delivery::Revoked => {
                // Unreported effects stay uncertain in the ledger; stopping the sandbox ends them.
                if let Some(active) = self.active.take() {
                    active.task.abort();
                }
                return Ok(Some(Next::Exit));
            }
        }
        Ok(None)
    }

    fn acknowledge(&mut self, turn: Uuid, sequence: u64) -> Result<(), Error> {
        let Some(index) = self
            .outbox
            .iter()
            .position(|(entry, _)| entry.turn == turn && entry.sequence == sequence)
        else {
            return Ok(());
        };
        if let (_, Some(journal)) = self.outbox.remove(index) {
            self.store
                .lock()
                .map_err(|_| Error::Journal)?
                .acknowledge(journal)
                .map_err(|_| Error::Journal)?;
        }
        Ok(())
    }
}

fn report(entry: &attempts::Entry) -> Report {
    Report::Event {
        turn: entry.turn,
        sequence: entry.sequence,
        event: entry.event.clone(),
    }
}
async fn send(channel: &mut Channel, report: &Report) -> Result<(), Error> {
    let text = serde_json::to_string(report).map_err(|_| Error::Protocol)?;
    channel
        .send(Message::Text(text.into()))
        .await
        .map_err(|_| Error::Unavailable)
}

//! The agent's own loop: one channel to the switchboard and one turn at a time. Events wait in an
//! outbox until the ledger acknowledges them, so a reconnect resends rather than loses them.
//! With no turn, no unacknowledged event and no drain for the idle delay, the agent dreams.
mod attempts;
mod dream;
mod turn;
use crate::control::{Channel, Config, Error, MemoryClient, RuntimeClient};
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
    time::Instant,
};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

pub async fn run() -> Result<(), &'static str> {
    let config = Config::from_environment().map_err(|_| "invalid runtime configuration")?;
    let root =
        std::env::var_os("CUBEBOT_JOURNAL_DIRECTORY").ok_or("runtime storage not configured")?;
    let tools = crate::tools::Registry::load().await?.map(Arc::new);
    let mut agent = Agent::open(Path::new(&root), tools)?;
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
    tools: Option<Arc<crate::tools::Registry>>,
    memory: Option<MemoryClient>,
    active: Option<Active>,
    drain: Option<Uuid>,
    /// When to dream, armed by the switchboard's idle notice.
    dream_at: Option<Instant>,
    dreaming: Option<(watch::Sender<bool>, tokio::task::JoinHandle<()>)>,
}

impl Agent {
    fn open(root: &Path, tools: Option<Arc<crate::tools::Registry>>) -> Result<Self, &'static str> {
        let metadata =
            std::fs::symlink_metadata(root).map_err(|_| "private runtime storage unavailable")?;
        if !metadata.is_dir() || metadata.mode() & 0o077 != 0 {
            return Err("private runtime storage required");
        }
        let mut store = Store::open(&root.join("tools"))
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
            tools,
            memory: None,
            active: None,
            drain: None,
            dream_at: None,
            dreaming: None,
        })
    }

    async fn session(&mut self, client: &mut RuntimeClient) -> Result<Next, Error> {
        if !self.bootstrapped {
            client.bootstrap().await?;
            self.bootstrapped = true;
            self.memory = client.memory();
        }
        let mut channel = client.connect().await?;
        for (entry, _) in &self.outbox {
            send(&mut channel, &report(entry)).await?;
        }
        self.idle(&mut channel).await?;
        loop {
            let dream_at = self.dream_at;
            tokio::select! {
                message = channel.next() => {
                    let text = match message {
                        Some(Ok(Message::Text(text))) => text,
                        Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                        _ => return Ok(Next::Reconnect),
                    };
                    self.wake();
                    let delivery = serde_json::from_str(&text).map_err(|_| Error::Protocol)?;
                    if let Some(next) = self.deliver(&mut channel, delivery).await? {
                        return Ok(next);
                    }
                }
                Some(event) = self.events.recv() => self.forward(&mut channel, event).await?,
                _ = until(dream_at) => self.dream(),
            }
        }
    }

    /// Any channel message disarms the idle timer and makes a running dream yield.
    fn wake(&mut self) {
        self.dream_at = None;
        if let Some((stop, _)) = &self.dreaming {
            let _ = stop.send(true);
        }
    }

    /// Dreams only with every lane idle and every reported outcome acknowledged.
    fn dream(&mut self) {
        self.dream_at = None;
        let dreaming = self
            .dreaming
            .as_ref()
            .is_some_and(|(_, task)| !task.is_finished());
        let Some(client) = self.memory.clone() else {
            return;
        };
        if dreaming || self.active.is_some() || self.drain.is_some() || !self.outbox.is_empty() {
            return;
        }
        let (stop, stopped) = watch::channel(false);
        let task = tokio::spawn(dream::run(client, Uuid::new_v4(), stopped));
        self.dreaming = Some((stop, task));
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
                    self.tools.clone(),
                    crate::memory::Memories::new(self.memory.clone()),
                    crate::skills::Skills::new(self.memory.clone()),
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
            Delivery::Idle {
                dream_after_seconds,
            } => {
                if self.active.is_none() {
                    let delay = Duration::from_secs(dream_after_seconds);
                    self.dream_at = Some(Instant::now() + delay);
                }
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

async fn until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
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

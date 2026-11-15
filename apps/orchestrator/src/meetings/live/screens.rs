use super::attendants::may_view;
use super::room::idle;
use super::wire::Server;
use super::{Handle, Live};
use crate::agents::runtime::RuntimeActor;
use crate::agents::{
    AgentId,
    sandbox::{ChannelError, Desktop, DesktopChannel},
};
use cubebot_protocol::desktop::{HostRequest, Mode, Response, Tile};
use std::future::Future;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};
use uuid::Uuid;

const POLL: Duration = Duration::from_millis(100);
const REPAINT: Duration = Duration::from_secs(2);
pub(super) const SHARING: &str = "Your desktop is now shown to everyone in the meeting.";
pub(super) const STOPPED: &str = "The desktop is no longer shown in the meeting.";
pub(super) const UNAVAILABLE: &str = "The agent's desktop is not available right now.";
pub(super) const NOT_SEATED: &str = "The agent is not in this meeting.";
pub(super) const TURNED_OFF: &str =
    "Agents may not share their screen in meetings on this platform.";
pub(super) const NOT_ALLOWED: &str = "You may not view this agent's desktop.";
pub(super) const GUESTS: &str = "Only signed-in members can ask an agent to share its screen.";
const NOT_LIVE: &str = "The meeting is not live.";
const NOT_MEETING: &str = "This turn is not part of a meeting.";
const TALK_ONLY: &str = "This request may not share your desktop.";
const NOT_PERMITTED: &str = "The person you are answering may not view your desktop.";

pub type Answer = Result<&'static str, &'static str>;

#[derive(Debug)]
pub enum Presenter {
    Member { slot: u16, account: Uuid },
    Agent(oneshot::Sender<Answer>),
}

impl Presenter {
    pub(super) async fn answer(self, room: &Handle, answer: Answer) {
        match (self, answer) {
            (Self::Member { slot, .. }, Err(text)) => {
                let notice = Server::Notice { text: text.into() };
                room.announce(notice, Some(slot)).await;
            }
            (Self::Member { .. }, Ok(_)) => {}
            (Self::Agent(reply), answer) => {
                let _ = reply.send(answer);
            }
        }
    }
}

pub(super) trait Desk: Send + 'static {
    fn open(&mut self) -> impl Future<Output = Result<(), ChannelError>> + Send;
    fn frame(&mut self) -> impl Future<Output = Result<Vec<Tile>, ChannelError>> + Send;
    fn close(self) -> impl Future<Output = ()> + Send;
}

pub(super) struct Watcher {
    desktop: Desktop,
    agent: AgentId,
    channel: Option<DesktopChannel>,
}

impl Watcher {
    pub(super) fn new(desktop: Desktop, agent: Uuid) -> Self {
        Self {
            desktop,
            agent: AgentId(agent),
            channel: None,
        }
    }
}

fn mode(mode: Mode) -> HostRequest {
    HostRequest::Mode {
        mode,
        account: None,
        handoff: false,
    }
}

impl Desk for Watcher {
    async fn open(&mut self) -> Result<(), ChannelError> {
        let mut channel = self.desktop.channel(self.agent).await?;
        if self.channel.is_none() {
            self.desktop.watch(self.agent);
            if let Err(error) = channel.request(&mode(Mode::Watch)).await {
                self.desktop.unwatch(self.agent);
                return Err(error);
            }
        }
        self.channel = Some(channel);
        Ok(())
    }

    async fn frame(&mut self) -> Result<Vec<Tile>, ChannelError> {
        let channel = self.channel.as_mut().ok_or(UNAVAILABLE)?;
        let line = channel.request(&HostRequest::Frame).await?;
        Ok(serde_json::from_str::<Response>(&line)?.tiles)
    }

    async fn close(self) {
        let Some(mut channel) = self.channel else {
            return;
        };
        if self.desktop.unwatch(self.agent) {
            let _ = channel.request(&mode(Mode::Agent)).await;
        }
    }
}

pub(super) struct Sharing {
    repaint: Option<mpsc::Sender<()>>,
    task: JoinHandle<()>,
}

impl Sharing {
    pub(super) fn start<D: Desk>(
        desk: D,
        room: Handle,
        slot: u16,
        previous: Option<Self>,
    ) -> (Self, oneshot::Receiver<bool>) {
        let (repaint, repaints) = mpsc::channel(1);
        let (started, start) = oneshot::channel();
        let previous = previous.map(|sharing| sharing.task);
        let task = tokio::spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            share(desk, room, slot, repaints, started).await;
        });
        let sharing = Self {
            repaint: Some(repaint),
            task,
        };
        (sharing, start)
    }

    pub(super) fn sharing(&self) -> bool {
        self.repaint.is_some() && !self.task.is_finished()
    }

    pub(super) fn repaint(&self) {
        if let Some(repaint) = &self.repaint {
            let _ = repaint.try_send(());
        }
    }

    pub(super) fn stop(&mut self) {
        self.repaint = None;
    }
}

async fn share<D: Desk>(
    mut desk: D,
    room: Handle,
    slot: u16,
    mut repaints: mpsc::Receiver<()>,
    started: oneshot::Sender<bool>,
) {
    let opened = desk.open().await;
    let _ = started.send(opened.is_ok());
    match opened {
        Ok(()) => {
            let (refresh, refreshes) = mpsc::channel(1);
            room.agent_screen(slot, Some(refresh)).await;
            relay(&mut desk, &room, slot, (&mut repaints, refreshes)).await;
            ended(&room, slot).await;
        }
        Err(error) => tracing::info!(?error, slot, "meeting screen share not started"),
    }
    desk.close().await;
}

async fn ended(room: &Handle, slot: u16) {
    let ended = Server::Desktop {
        source: slot,
        tiles: None,
        ended: true,
    };
    room.announce(ended, None).await;
    room.agent_screen(slot, None).await;
}

fn repaint_at(due: Option<Instant>, painted: Option<Instant>) -> Option<Instant> {
    let now = Instant::now();
    due.or_else(|| Some(painted.map_or(now, |at| (at + REPAINT).max(now))))
}

async fn relay<D: Desk>(
    desk: &mut D,
    room: &Handle,
    slot: u16,
    (repaints, mut refreshes): (&mut mpsc::Receiver<()>, mpsc::Receiver<()>),
) {
    let mut ticks = tokio::time::interval(POLL);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let (mut due, mut painted) = (None, None);
    loop {
        let polled = tokio::select! {
            repaint = repaints.recv() => match repaint {
                Some(()) => {
                    due = repaint_at(due, painted);
                    continue;
                }
                None => return,
            },
            Some(()) = refreshes.recv() => {
                due = repaint_at(due, painted);
                continue;
            }
            () = idle(due) => {
                (due, painted) = (None, Some(Instant::now()));
                desk.open().await.map(|()| Vec::new())
            }
            _ = ticks.tick() => desk.frame().await,
        };
        if !deliver(room, slot, polled).await {
            return;
        }
    }
}

async fn deliver(room: &Handle, slot: u16, polled: Result<Vec<Tile>, ChannelError>) -> bool {
    match polled {
        Ok(tiles) if tiles.is_empty() => true,
        Ok(tiles) => {
            let frame = Server::Desktop {
                source: slot,
                tiles: Some(tiles),
                ended: false,
            };
            room.announce(frame, None).await
        }
        Err(error) => {
            tracing::info!(
                ?error,
                slot,
                "meeting screen share ended: the desktop stopped answering"
            );
            false
        }
    }
}

impl Live {
    #[tracing::instrument(skip_all, fields(agent = %actor.agent.0, turn = %turn))]
    pub async fn agent_screen(
        &self,
        actor: &RuntimeActor,
        turn: Uuid,
        share: bool,
    ) -> Result<Answer, sqlx::Error> {
        let asked = sqlx::query!(
            "
            SELECT meeting_id, actor_id, talk_only
            FROM chat_turns
            WHERE id=$1 AND conversation_id=$2 AND status='running' AND attempt_id=$3
            ",
            turn,
            actor.agent.0,
            actor.attempt.0,
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some((meeting, asked)) = asked.and_then(|asked| Some((asked.meeting_id?, asked)))
        else {
            return Ok(Err(NOT_MEETING));
        };
        if asked.talk_only {
            return Ok(Err(TALK_ONLY));
        }
        if !may_view(&self.pool, asked.actor_id, actor.agent.0).await {
            return Ok(Err(NOT_PERMITTED));
        }
        let Some(room) = self.room(meeting) else {
            return Ok(Err(NOT_LIVE));
        };
        Ok(room.present(actor.agent.0, share).await)
    }
}

#[cfg(test)]
mod tests {
    use super::super::room::Command;
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    type Frames = Arc<Mutex<VecDeque<Result<Vec<Tile>, &'static str>>>>;

    #[derive(Clone, Default)]
    struct Fake {
        opened: Arc<Mutex<usize>>,
        closed: Arc<Mutex<bool>>,
        frames: Frames,
    }

    fn tile(x: u32) -> Tile {
        Tile {
            x,
            y: 0,
            png: "iVBORw0KGgo=".into(),
        }
    }

    impl Desk for Fake {
        async fn open(&mut self) -> Result<(), ChannelError> {
            *self.opened.lock().unwrap() += 1;
            let mut frames = self.frames.lock().unwrap();
            frames.clear();
            frames.push_back(Ok(vec![tile(0), tile(64)]));
            Ok(())
        }

        async fn frame(&mut self) -> Result<Vec<Tile>, ChannelError> {
            let next = self.frames.lock().unwrap().pop_front();
            Ok(next.unwrap_or(Ok(Vec::new()))?)
        }

        async fn close(self) {
            *self.closed.lock().unwrap() = true;
        }
    }

    fn room() -> (Handle, mpsc::Receiver<Command>) {
        let (commands, received) = mpsc::channel(64);
        let (invited, _) = mpsc::channel(1);
        (Handle { commands, invited }, received)
    }

    enum Seen {
        Screen(bool),
        Tiles(usize),
        Ended,
    }

    async fn next(received: &mut mpsc::Receiver<Command>) -> Seen {
        loop {
            let command = tokio::time::timeout(Duration::from_secs(5), received.recv())
                .await
                .unwrap()
                .unwrap();
            match command {
                Command::AgentScreen { refresh, .. } => return Seen::Screen(refresh.is_some()),
                Command::Announce {
                    message:
                        Server::Desktop {
                            tiles: Some(tiles), ..
                        },
                    to: None,
                } => return Seen::Tiles(tiles.len()),
                Command::Announce {
                    message: Server::Desktop { ended: true, .. },
                    to: None,
                } => return Seen::Ended,
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn a_share_sends_every_tile_first_repaints_for_a_newcomer_and_ends_cleanly() {
        let (handle, mut received) = room();
        let fake = Fake::default();
        let (mut sharing, start) = Sharing::start(fake.clone(), handle, 4, None);
        assert!(start.await.unwrap());
        assert!(matches!(next(&mut received).await, Seen::Screen(true)));
        assert!(matches!(next(&mut received).await, Seen::Tiles(2)));
        fake.frames.lock().unwrap().push_back(Ok(vec![tile(128)]));
        assert!(matches!(next(&mut received).await, Seen::Tiles(1)));
        sharing.repaint();
        assert!(matches!(next(&mut received).await, Seen::Tiles(2)));
        assert_eq!(*fake.opened.lock().unwrap(), 2);
        assert!(sharing.sharing());
        sharing.stop();
        assert!(!sharing.sharing());
        assert!(matches!(next(&mut received).await, Seen::Ended));
        assert!(matches!(next(&mut received).await, Seen::Screen(false)));
        sharing.task.await.unwrap();
        assert!(*fake.closed.lock().unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn the_room_asks_for_repaints_and_gets_at_most_one_every_two_seconds() {
        let (handle, mut received) = room();
        let fake = Fake::default();
        let (_sharing, start) = Sharing::start(fake.clone(), handle, 4, None);
        assert!(start.await.unwrap());
        let refresh = loop {
            if let Some(Command::AgentScreen {
                refresh: Some(refresh),
                ..
            }) = received.recv().await
            {
                break refresh;
            }
        };
        assert!(matches!(next(&mut received).await, Seen::Tiles(2)));
        let begun = Instant::now();
        refresh.try_send(()).unwrap();
        assert!(matches!(next(&mut received).await, Seen::Tiles(2)));
        assert!(begun.elapsed() < REPAINT);
        refresh.try_send(()).unwrap();
        assert!(matches!(next(&mut received).await, Seen::Tiles(2)));
        assert!(begun.elapsed() >= REPAINT);
        assert_eq!(*fake.opened.lock().unwrap(), 3);
    }

    #[tokio::test]
    async fn a_desktop_that_stops_answering_ends_the_share() {
        let (handle, mut received) = room();
        let fake = Fake::default();
        let (sharing, start) = Sharing::start(fake.clone(), handle, 4, None);
        assert!(start.await.unwrap());
        assert!(matches!(next(&mut received).await, Seen::Screen(true)));
        assert!(matches!(next(&mut received).await, Seen::Tiles(2)));
        fake.frames.lock().unwrap().push_back(Err("gone"));
        assert!(matches!(next(&mut received).await, Seen::Ended));
        assert!(matches!(next(&mut received).await, Seen::Screen(false)));
        sharing.task.await.unwrap();
        assert!(*fake.closed.lock().unwrap());
    }
}

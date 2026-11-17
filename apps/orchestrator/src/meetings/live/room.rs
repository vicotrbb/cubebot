use super::screens::{self, Answer, Presenter};
use super::wire::{self, Client, Kind, Media, Participant, Server, Stream, Waiting};
use crate::agents::Avatar;
use crate::identity::audit::Event as Audit;
use axum::body::Bytes;
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes};
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio::sync::oneshot;
use tokio::time::Instant;
use uuid::Uuid;

const MEDIA_QUEUE: usize = 256;
const CONTROL_QUEUE: usize = 64;
const LOBBY_LIMIT: usize = 20;
const CODEC_CHARS: usize = 64;
const EMPTY_GRACE: Duration = Duration::from_secs(300);
const KEYFRAME_SPACING: Duration = Duration::from_secs(1);
const ENDED: &str = "The meeting has ended.";
const FULL_NOTICE: &str = "The meeting is full.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Full,
    LobbyFull,
    Locked,
    Ended,
    Link,
    Gone,
}

impl Refusal {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Full => FULL_NOTICE,
            Self::LobbyFull => "Too many people are waiting to join. Try again later.",
            Self::Locked => "The host locked the meeting.",
            Self::Ended => ENDED,
            Self::Link => "This meeting link no longer works.",
            Self::Gone => "The meeting is not available right now. Try again.",
        }
    }
}

#[derive(Debug)]
pub enum Event {
    Joined {
        slot: u16,
        account: Option<Uuid>,
        name: String,
    },
    Left {
        slot: u16,
    },
    Microphone {
        slot: u16,
        pcm: Bytes,
    },
    AgentRemoved {
        slot: u16,
    },
    Present {
        agent: Uuid,
        share: bool,
        presenter: Presenter,
    },
}

pub(super) enum Who {
    Member(Uuid),
    Guest { token: String },
    Agent { agent: Uuid, avatar: Avatar },
}

pub(super) struct Joiner {
    pub who: Who,
    pub name: String,
}

pub(super) struct Joined {
    pub slot: u16,
    pub inbound: Inbound,
}

pub(super) enum Out {
    Frame(Message),
    Roster,
}

pub(super) type Latest = Arc<Mutex<Option<Message>>>;

pub(super) fn opened(roster: &Latest, out: Out) -> Option<Message> {
    match out {
        Out::Frame(message) => Some(message),
        Out::Roster => roster.lock().unwrap_or_else(PoisonError::into_inner).take(),
    }
}

pub(super) struct Inbound {
    pub control: mpsc::Receiver<Out>,
    pub media: mpsc::Receiver<Message>,
    pub roster: Latest,
}

struct Outbound {
    control: mpsc::Sender<Out>,
    media: mpsc::Sender<Message>,
    roster: Latest,
}

fn outbound() -> (Outbound, Inbound) {
    let (control, control_rx) = mpsc::channel(CONTROL_QUEUE);
    let (media, media_rx) = mpsc::channel(MEDIA_QUEUE);
    let roster = Latest::default();
    let inbound = Inbound {
        control: control_rx,
        media: media_rx,
        roster: roster.clone(),
    };
    let outbound = Outbound {
        control,
        media,
        roster,
    };
    (outbound, inbound)
}

pub(super) struct Subscriber {
    pub control: mpsc::UnboundedSender<Event>,
    pub audio: mpsc::Sender<Event>,
}

pub(super) fn close(code: u16, reason: &'static str) -> Message {
    Message::Close(Some(CloseFrame {
        code,
        reason: Utf8Bytes::from_static(reason),
    }))
}

pub(super) enum Command {
    Join {
        joiner: Joiner,
        reply: oneshot::Sender<Result<Joined, Refusal>>,
    },
    Leave {
        slot: u16,
        reason: Option<&'static str>,
    },
    Media {
        slot: u16,
        media: Media,
    },
    Said {
        slot: u16,
        message: Client,
    },
    Link {
        token: Option<String>,
    },
    Flags {
        open_join: bool,
        locked: bool,
    },
    Subscribe {
        events: Subscriber,
    },
    Agent(Seating),
    AgentScreen {
        slot: u16,
        refresh: Option<mpsc::Sender<()>>,
    },
    Announce {
        message: Server,
        to: Option<u16>,
    },
}

pub(super) enum Seating {
    Add {
        agent: Uuid,
        name: String,
        avatar: Avatar,
        reply: oneshot::Sender<Option<u16>>,
    },
    Remove {
        slot: u16,
    },
    Listening {
        agent: Uuid,
        listening: bool,
    },
    Audio {
        slot: u16,
        frame: Bytes,
    },
    Present {
        agent: Uuid,
        share: bool,
        reply: oneshot::Sender<Answer>,
    },
    MayPresent {
        slot: u16,
        agents: Vec<Uuid>,
    },
}

pub(super) enum Record {
    Started,
    Joined {
        attendance: Uuid,
        account: Option<Uuid>,
        guest: Option<String>,
    },
    Left {
        attendance: Uuid,
    },
    Locked(bool),
    Audit(Audit),
}

pub(super) struct Loaded {
    pub title: String,
    pub owner: Uuid,
    pub token: Option<String>,
    pub open_join: bool,
    pub locked: bool,
    pub ended: bool,
    pub capacity: usize,
}

struct Seat {
    who: Who,
    name: String,
    admitted: bool,
    mic: bool,
    camera: bool,
    screen: bool,
    hand: bool,
    listening: bool,
    tracks: HashMap<Stream, Message>,
    out: Option<Outbound>,
    blocked: HashSet<(u16, Stream)>,
    asked: HashMap<Stream, Instant>,
    attendance: Option<Uuid>,
    presents: HashSet<Uuid>,
    refresh: Option<mpsc::Sender<()>>,
}

impl Seat {
    fn new(who: Who, name: String, out: Option<Outbound>) -> Self {
        Self {
            who,
            name,
            admitted: false,
            mic: false,
            camera: false,
            screen: false,
            hand: false,
            listening: false,
            tracks: HashMap::new(),
            out,
            blocked: HashSet::new(),
            asked: HashMap::new(),
            attendance: None,
            presents: HashSet::new(),
            refresh: None,
        }
    }

    fn member(&self) -> Option<Uuid> {
        match self.who {
            Who::Member(account) => Some(account),
            _ => None,
        }
    }

    fn agent(&self) -> bool {
        self.agent_id().is_some()
    }

    fn agent_id(&self) -> Option<Uuid> {
        match self.who {
            Who::Agent { agent, .. } => Some(agent),
            _ => None,
        }
    }

    fn guest(&self) -> bool {
        matches!(self.who, Who::Guest { .. })
    }

    fn participant(&self, slot: u16, owner: Uuid, viewer: Option<&HashSet<Uuid>>) -> Participant {
        let members = viewer.is_some();
        let (kind, account, agent, avatar) = match &self.who {
            Who::Member(account) => (Kind::Person, Some(*account), None, None),
            Who::Guest { .. } => (Kind::Guest, None, None, None),
            Who::Agent { agent, avatar } => (Kind::Agent, None, Some(*agent), Some(*avatar)),
        };
        Participant {
            slot,
            kind,
            name: self.name.clone(),
            account: account.filter(|_| members),
            agent: agent.filter(|_| members),
            avatar,
            host: account == Some(owner),
            mic: self.mic,
            camera: self.camera,
            screen: self.screen,
            hand: self.hand,
            listening: self.listening,
            can_present: agent
                .zip(viewer)
                .map(|(agent, presents)| presents.contains(&agent)),
        }
    }
}

fn tell(slow: &mut Vec<u16>, slot: u16, seat: &Seat, message: Message) {
    let Some(out) = &seat.out else {
        return;
    };
    if let Err(TrySendError::Full(_)) = out.control.try_send(Out::Frame(message)) {
        slow.push(slot);
    }
}

fn post_roster(slow: &mut Vec<u16>, slot: u16, seat: &Seat, roster: Message) {
    let Some(out) = &seat.out else {
        return;
    };
    let mut latest = out.roster.lock().unwrap_or_else(PoisonError::into_inner);
    if latest.replace(roster).is_none()
        && let Err(TrySendError::Full(_)) = out.control.try_send(Out::Roster)
    {
        slow.push(slot);
    }
}

fn deliver(
    blocked: &mut HashSet<(u16, Stream)>,
    out: &mpsc::Sender<Message>,
    from: (u16, Stream),
    key: bool,
    frame: &Message,
) -> bool {
    if !key && blocked.contains(&from) {
        return false;
    }
    match out.try_send(frame.clone()) {
        Ok(()) => {
            blocked.remove(&from);
            false
        }
        Err(TrySendError::Full(_)) => blocked.insert(from) || key,
        Err(TrySendError::Closed(_)) => false,
    }
}

pub(super) async fn idle(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

pub(super) struct Room {
    id: Uuid,
    title: String,
    owner: Uuid,
    token: Option<String>,
    open_join: bool,
    locked: bool,
    capacity: usize,
    next: u16,
    seats: BTreeMap<u16, Seat>,
    started: bool,
    ended: bool,
    deadline: Option<Instant>,
    slow: Vec<u16>,
    records: mpsc::UnboundedSender<Record>,
    events: Option<Subscriber>,
}

impl Room {
    pub(super) fn new(id: Uuid, loaded: Loaded, records: mpsc::UnboundedSender<Record>) -> Self {
        Self {
            id,
            title: loaded.title,
            owner: loaded.owner,
            token: loaded.token,
            open_join: loaded.open_join,
            locked: loaded.locked,
            capacity: loaded.capacity,
            next: 1,
            seats: BTreeMap::new(),
            started: false,
            ended: false,
            deadline: Some(Instant::now() + EMPTY_GRACE),
            slow: Vec::new(),
            records,
            events: None,
        }
    }

    pub(super) async fn run(mut self, mut commands: mpsc::Receiver<Command>) {
        while !self.ended {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(command) => self.handle(command),
                    None => break,
                },
                () = idle(self.deadline) => self.ended = true,
            }
        }
        self.farewell();
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Join { joiner, reply } => {
                let _ = reply.send(self.join(joiner));
            }
            Command::Leave { slot, reason } => self.leave(slot, reason),
            Command::Media { slot, media } => self.media(slot, media),
            Command::Said { slot, message } => self.said(slot, message),
            Command::Link { token } => self.relink(token),
            Command::Flags { open_join, locked } => self.flags(open_join, locked),
            Command::Subscribe { events } => self.events = Some(events),
            Command::Agent(seating) => self.seating(seating),
            Command::AgentScreen { slot, refresh } => self.agent_screen(slot, refresh),
            Command::Announce { message, to } => self.announce(&message, to),
        }
        while let Some(slot) = self.slow.pop() {
            tracing::warn!(meeting = %self.id, slot, "meeting participant too far behind");
            self.leave(slot, None);
        }
    }

    fn seating(&mut self, seating: Seating) {
        match seating {
            Seating::Add {
                agent,
                name,
                avatar,
                reply,
            } => {
                let _ = reply.send(self.add_agent(agent, name, avatar));
            }
            Seating::Remove { slot } => self.remove_agent(slot),
            Seating::Listening { agent, listening } => self.agent_listening(agent, listening),
            Seating::Audio { slot, frame } => self.agent_audio(slot, frame),
            Seating::Present {
                agent,
                share,
                reply,
            } => self.agent_present(agent, share, reply),
            Seating::MayPresent { slot, agents } => self.may_present(slot, agents),
        }
    }

    fn farewell(&mut self) {
        let ended = Server::Ended.message();
        for seat in self.seats.values() {
            if let Some(out) = &seat.out {
                let _ = out.control.try_send(Out::Frame(ended.clone()));
                let _ = out.control.try_send(Out::Frame(close(1000, ENDED)));
            }
        }
        self.seats.clear();
    }

    fn seated(&self) -> usize {
        self.seats.values().filter(|seat| seat.admitted).count()
    }

    fn waiting(&self) -> usize {
        self.seats.values().filter(|seat| !seat.admitted).count()
    }

    fn slot(&mut self) -> Option<u16> {
        let slot = self.next;
        self.next = self.next.checked_add(1)?;
        Some(slot)
    }

    fn record(&self, record: Record) {
        let _ = self.records.send(record);
    }

    fn emit(&mut self, event: Event) {
        let Some(events) = &self.events else {
            return;
        };
        let open = if matches!(event, Event::Microphone { .. }) {
            match events.audio.try_send(event) {
                Err(TrySendError::Full(_)) => {
                    tracing::debug!(meeting = %self.id, "meeting audio dropped: listeners behind");
                    true
                }
                sent => sent.is_ok(),
            }
        } else {
            events.control.send(event).is_ok()
        };
        if !open {
            self.events = None;
        }
    }

    fn send(&mut self, slot: u16, server: &Server) {
        if let Some(seat) = self.seats.get(&slot) {
            tell(&mut self.slow, slot, seat, server.message());
        }
    }

    fn broadcast(&mut self, server: &Server, except: Option<u16>) {
        let message = server.message();
        for (&slot, seat) in &self.seats {
            if seat.admitted && Some(slot) != except {
                tell(&mut self.slow, slot, seat, message.clone());
            }
        }
    }

    fn announce(&mut self, server: &Server, to: Option<u16>) {
        match (server, to) {
            (
                Server::Desktop {
                    source,
                    tiles: Some(_),
                    ..
                },
                None,
            ) => self.tiles(*source, server),
            (_, Some(slot)) => self.send(slot, server),
            (_, None) => self.broadcast(server, None),
        }
    }

    fn tiles(&self, source: u16, server: &Server) {
        let frame = server.message();
        let mut dropped = false;
        for (&slot, seat) in &self.seats {
            if let Some(out) = seat
                .out
                .as_ref()
                .filter(|_| seat.admitted && slot != source)
            {
                dropped |= matches!(
                    out.media.try_send(frame.clone()),
                    Err(TrySendError::Full(_))
                );
            }
        }
        let refresh = self
            .seats
            .get(&source)
            .and_then(|seat| seat.refresh.as_ref());
        if let Some(refresh) = refresh.filter(|_| dropped) {
            let _ = refresh.try_send(());
        }
    }

    fn listing(&self, viewer: Option<&HashSet<Uuid>>) -> Message {
        let participants = self
            .seats
            .iter()
            .filter(|(_, seat)| seat.admitted)
            .map(|(&slot, seat)| seat.participant(slot, self.owner, viewer))
            .collect();
        Server::Roster { participants }.message()
    }

    fn roster(&mut self) {
        let guests = self.listing(None);
        let rosters: Vec<(u16, Message)> = self
            .seats
            .iter()
            .filter(|(_, seat)| seat.admitted && seat.out.is_some())
            .map(|(&slot, seat)| match seat.member() {
                Some(_) => (slot, self.listing(Some(&seat.presents))),
                None => (slot, guests.clone()),
            })
            .collect();
        for (slot, roster) in rosters {
            if let Some(seat) = self.seats.get(&slot) {
                post_roster(&mut self.slow, slot, seat, roster);
            }
        }
    }

    fn lobby(&mut self, to: Option<u16>) {
        let waiting = self
            .seats
            .iter()
            .filter(|(_, seat)| !seat.admitted)
            .map(|(slot, seat)| Waiting {
                id: slot.to_string(),
                name: seat.name.clone(),
            })
            .collect();
        let message = Server::Lobby { waiting }.message();
        for (&slot, seat) in &self.seats {
            if seat.admitted && seat.member().is_some() && to.is_none_or(|to| to == slot) {
                tell(&mut self.slow, slot, seat, message.clone());
            }
        }
    }

    fn join(&mut self, joiner: Joiner) -> Result<Joined, Refusal> {
        if let Who::Guest { token } = &joiner.who {
            if self.token.as_ref() != Some(token) {
                return Err(Refusal::Link);
            }
            if self.locked {
                return Err(Refusal::Locked);
            }
        }
        let waits = matches!(joiner.who, Who::Guest { .. }) && !self.open_join;
        if waits && self.waiting() >= LOBBY_LIMIT {
            return Err(Refusal::LobbyFull);
        }
        if !waits && self.seated() >= self.capacity {
            return Err(Refusal::Full);
        }
        let slot = self.slot().ok_or(Refusal::Full)?;
        let (out, inbound) = outbound();
        let seat = Seat::new(joiner.who, joiner.name, Some(out));
        self.seats.insert(slot, seat);
        if waits {
            self.send(slot, &Server::Waiting);
            self.lobby(None);
        } else {
            self.seat(slot);
        }
        Ok(Joined { slot, inbound })
    }

    fn seat(&mut self, slot: u16) {
        let Some(seat) = self.seats.get_mut(&slot) else {
            return;
        };
        let attendance = Uuid::new_v4();
        seat.admitted = true;
        self.deadline = None;
        seat.attendance = Some(attendance);
        let (account, guest, name) = (seat.member(), seat.guest(), seat.name.clone());
        if !std::mem::replace(&mut self.started, true) {
            self.record(Record::Started);
        }
        self.record(Record::Joined {
            attendance,
            account,
            guest: guest.then(|| name.clone()),
        });
        if guest {
            let detail = json!({"name": self.title, "guest": name});
            let event = Audit::anonymous("meeting.guest_join").on(self.id);
            self.record(Record::Audit(event.detail(detail)));
        }
        let welcome = Server::Welcome {
            you: slot,
            title: self.title.clone(),
            host: account == Some(self.owner),
            member: account.is_some(),
            open_join: self.open_join,
            locked: self.locked,
            captions: true,
        };
        self.send(slot, &welcome);
        self.roster();
        self.lobby(Some(slot));
        self.introduce(slot);
        self.emit(Event::Joined {
            slot,
            account,
            name,
        });
    }

    fn introduce(&mut self, slot: u16) {
        let mut tracks = Vec::new();
        for (&other, seat) in &self.seats {
            if other != slot && seat.admitted {
                tracks.extend(
                    seat.tracks
                        .iter()
                        .map(|(&stream, track)| (other, stream, track)),
                );
            }
        }
        let senders: Vec<(u16, Stream)> = tracks
            .iter()
            .map(|&(other, stream, _)| (other, stream))
            .collect();
        if let Some(seat) = self.seats.get(&slot) {
            for (_, _, track) in tracks {
                tell(&mut self.slow, slot, seat, track.clone());
            }
        }
        for (other, stream) in senders {
            self.ask_keyframe(other, stream);
        }
    }

    fn ask_keyframe(&mut self, slot: u16, stream: Stream) {
        let now = Instant::now();
        let Some(seat) = self.seats.get_mut(&slot).filter(|seat| seat.admitted) else {
            return;
        };
        let recent = seat.asked.get(&stream);
        if recent.is_some_and(|at| now.duration_since(*at) < KEYFRAME_SPACING) {
            return;
        }
        seat.asked.insert(stream, now);
        self.send(slot, &Server::Keyframe { stream });
    }

    fn leave(&mut self, slot: u16, reason: Option<&'static str>) {
        let Some(seat) = self.seats.remove(&slot) else {
            return;
        };
        if let (Some(reason), Some(out)) = (reason, &seat.out) {
            let _ = out.control.try_send(Out::Frame(close(1000, reason)));
        }
        for other in self.seats.values_mut() {
            other.blocked.retain(|(from, _)| *from != slot);
        }
        if !seat.admitted {
            self.lobby(None);
        } else {
            if let Some(attendance) = seat.attendance {
                self.record(Record::Left { attendance });
            }
            self.roster();
            if !seat.agent() {
                self.emit(Event::Left { slot });
            }
        }
        let people = self
            .seats
            .values()
            .any(|seat| seat.admitted && !seat.agent());
        if !people && self.deadline.is_none() {
            self.deadline = Some(Instant::now() + EMPTY_GRACE);
        }
    }

    fn kick(&mut self, slot: u16, server: &Server, reason: &'static str) {
        self.send(slot, server);
        self.leave(slot, Some(reason));
    }

    fn media(&mut self, slot: u16, media: Media) {
        let Some(seat) = self.seats.get(&slot).filter(|seat| seat.admitted) else {
            return;
        };
        match media {
            Media::Audio(pcm) if seat.mic => {
                let frame = Message::Binary(wire::audio(slot, wire::PEOPLE_RATE, &pcm));
                self.relay_audio(slot, &frame);
                self.emit(Event::Microphone { slot, pcm });
            }
            Media::Audio(_) => {}
            Media::Video {
                stream,
                key,
                payload,
            } => self.video(slot, stream, key, &payload),
        }
    }

    fn relay_audio(&self, from: u16, frame: &Message) {
        for (&slot, seat) in &self.seats {
            if let Some(out) = seat.out.as_ref().filter(|_| seat.admitted && slot != from) {
                let _ = out.media.try_send(frame.clone());
            }
        }
    }

    fn video(&mut self, from: u16, stream: Stream, key: bool, payload: &[u8]) {
        let frame = Message::Binary(wire::video(stream, from, payload));
        let mut starved = false;
        for (&slot, seat) in &mut self.seats {
            if slot == from || !seat.admitted {
                continue;
            }
            if let Some(out) = &seat.out {
                starved |= deliver(&mut seat.blocked, &out.media, (from, stream), key, &frame);
            }
        }
        if starved {
            self.ask_keyframe(from, stream);
        }
    }

    fn said(&mut self, slot: u16, message: Client) {
        let Some(seat) = self.seats.get(&slot).filter(|seat| seat.admitted) else {
            return;
        };
        let account = seat.member();
        match message {
            Client::State {
                mic,
                camera,
                screen,
                hand,
            } => self.state(slot, [mic, camera, screen, hand]),
            Client::Track {
                stream,
                codec,
                width,
                height,
            } => self.track(slot, stream, codec, (width, height)),
            Client::WantKeyframe { source, stream } => self.ask_keyframe(source, stream),
            Client::Present { source, share } => self.present(slot, source, share),
            ordered @ (Client::Admit { .. }
            | Client::Deny { .. }
            | Client::Remove { .. }
            | Client::Lock { .. }
            | Client::End) => {
                if let Some(account) = account {
                    self.ordered(slot, account, ordered);
                }
            }
            Client::Hello { .. } | Client::Ping => {}
        }
    }

    fn ordered(&mut self, slot: u16, by: Uuid, message: Client) {
        let host = by == self.owner;
        match message {
            Client::Admit { id } => self.admit(slot, by, &id),
            Client::Deny { id } => self.deny(by, &id),
            Client::Remove { source } if host => self.remove(by, source),
            Client::Lock { locked } if host => self.lock(by, locked),
            Client::End if host => self.end(by),
            _ => {}
        }
    }

    fn state(&mut self, slot: u16, [mic, camera, screen, hand]: [bool; 4]) {
        let Some(seat) = self.seats.get_mut(&slot) else {
            return;
        };
        let state = (mic, camera, screen, hand);
        if (seat.mic, seat.camera, seat.screen, seat.hand) != state {
            (seat.mic, seat.camera, seat.screen, seat.hand) = state;
            self.roster();
        }
    }

    fn track(&mut self, slot: u16, stream: Stream, codec: Option<String>, size: (u32, u32)) {
        if codec
            .as_ref()
            .is_some_and(|codec| codec.chars().count() > CODEC_CHARS)
        {
            return;
        }
        let live = codec.is_some();
        let (width, height) = size;
        let server = Server::Track {
            source: slot,
            stream,
            codec,
            width,
            height,
        };
        let Some(seat) = self.seats.get_mut(&slot) else {
            return;
        };
        let message = server.message();
        let changed = if live {
            seat.tracks.insert(stream, message.clone()).as_ref() != Some(&message)
        } else {
            seat.tracks.remove(&stream).is_some()
        };
        if changed {
            self.broadcast(&server, Some(slot));
        }
    }

    fn present(&mut self, slot: u16, source: u16, share: bool) {
        let Some(account) = self.seats.get(&slot).and_then(Seat::member) else {
            let notice = Server::Notice {
                text: screens::GUESTS.into(),
            };
            return self.send(slot, &notice);
        };
        let Some(agent) = self.seats.get(&source).and_then(Seat::agent_id) else {
            return;
        };
        let presenter = Presenter::Member { slot, account };
        self.emit(Event::Present {
            agent,
            share,
            presenter,
        });
    }

    fn waiting_slot(&self, id: &str) -> Option<(u16, String)> {
        let slot = id.parse().ok()?;
        let seat = self.seats.get(&slot).filter(|seat| !seat.admitted)?;
        Some((slot, seat.name.clone()))
    }

    fn audit(&self, by: Uuid, kind: &'static str, detail: serde_json::Value) {
        let event = Audit::by(by, kind).on(self.id).detail(detail);
        self.record(Record::Audit(event));
    }

    fn admit(&mut self, slot: u16, by: Uuid, id: &str) {
        let Some((guest, name)) = self.waiting_slot(id) else {
            return;
        };
        if self.seated() >= self.capacity {
            let notice = Server::Notice {
                text: FULL_NOTICE.into(),
            };
            return self.send(slot, &notice);
        }
        self.seat(guest);
        self.lobby(None);
        self.audit(
            by,
            "meeting.admit",
            json!({"name": self.title, "guest": name}),
        );
    }

    fn deny(&mut self, by: Uuid, id: &str) {
        let Some((guest, name)) = self.waiting_slot(id) else {
            return;
        };
        self.kick(guest, &Server::Denied, "You were not let in.");
        self.audit(
            by,
            "meeting.deny",
            json!({"name": self.title, "guest": name}),
        );
    }

    fn remove(&mut self, by: Uuid, source: u16) {
        let Some(seat) = self.seats.get(&source).filter(|seat| seat.admitted) else {
            return;
        };
        if seat.member() == Some(self.owner) {
            return;
        }
        let (name, agent) = (seat.name.clone(), seat.agent());
        if agent {
            self.leave(source, None);
            self.emit(Event::AgentRemoved { slot: source });
        } else {
            self.kick(
                source,
                &Server::Removed,
                "The host removed you from the meeting.",
            );
        }
        let detail = json!({"name": self.title, "participant": name});
        self.audit(by, "meeting.remove", detail);
    }

    fn lock(&mut self, by: Uuid, locked: bool) {
        self.locked = locked;
        self.record(Record::Locked(locked));
        self.flags(self.open_join, locked);
        let detail = json!({"name": self.title, "locked": locked});
        self.audit(by, "meeting.lock", detail);
    }

    fn end(&mut self, by: Uuid) {
        self.audit(by, "meeting.end", json!({"name": self.title}));
        self.ended = true;
    }

    fn flags(&mut self, open_join: bool, locked: bool) {
        (self.open_join, self.locked) = (open_join, locked);
        let meeting = Server::Meeting { locked, open_join };
        self.broadcast(&meeting, None);
    }

    fn relink(&mut self, token: Option<String>) {
        let stale: Vec<u16> = self
            .seats
            .iter()
            .filter(|(_, seat)| matches!(&seat.who, Who::Guest { token: held } if Some(held) != token.as_ref()))
            .map(|(&slot, _)| slot)
            .collect();
        self.token = token;
        for slot in stale {
            self.leave(slot, Some(Refusal::Link.reason()));
        }
    }

    fn add_agent(&mut self, agent: Uuid, name: String, avatar: Avatar) -> Option<u16> {
        if self.seated() >= self.capacity {
            return None;
        }
        let slot = self.slot()?;
        let mut seat = Seat::new(Who::Agent { agent, avatar }, name, None);
        seat.admitted = true;
        self.seats.insert(slot, seat);
        self.roster();
        Some(slot)
    }

    fn agent_listening(&mut self, agent: Uuid, listening: bool) {
        let seat = self
            .seats
            .values_mut()
            .find(|seat| seat.agent_id() == Some(agent));
        if let Some(seat) = seat.filter(|seat| seat.listening != listening) {
            seat.listening = listening;
            self.roster();
        }
    }

    fn remove_agent(&mut self, slot: u16) {
        if self.seats.get(&slot).is_some_and(Seat::agent) {
            self.leave(slot, None);
        }
    }

    fn agent_audio(&self, slot: u16, frame: Bytes) {
        if self.seats.get(&slot).is_some_and(Seat::agent) {
            self.relay_audio(slot, &Message::Binary(frame));
        }
    }

    fn agent_present(&mut self, agent: Uuid, share: bool, reply: oneshot::Sender<Answer>) {
        if !self
            .seats
            .values()
            .any(|seat| seat.agent_id() == Some(agent))
        {
            let _ = reply.send(Err(screens::NOT_SEATED));
            return;
        }
        let presenter = Presenter::Agent(reply);
        self.emit(Event::Present {
            agent,
            share,
            presenter,
        });
    }

    fn may_present(&mut self, slot: u16, agents: Vec<Uuid>) {
        if let Some(seat) = self.seats.get_mut(&slot) {
            seat.presents.extend(agents);
            self.roster();
        }
    }

    fn agent_screen(&mut self, slot: u16, refresh: Option<mpsc::Sender<()>>) {
        if let Some(seat) = self.seats.get_mut(&slot).filter(|seat| seat.agent()) {
            seat.screen = refresh.is_some();
            seat.refresh = refresh;
            self.roster();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: Uuid = Uuid::from_u128(1);
    const TOKEN: &str = "token";

    fn opened(open_join: bool, capacity: usize) -> (Room, mpsc::UnboundedReceiver<Record>) {
        let (records, stored) = mpsc::unbounded_channel();
        let loaded = Loaded {
            title: "Sync".into(),
            owner: OWNER,
            token: Some(TOKEN.into()),
            open_join,
            locked: false,
            ended: false,
            capacity,
        };
        (Room::new(Uuid::from_u128(9), loaded, records), stored)
    }

    fn member(room: &mut Room, account: u128) -> Joined {
        let joiner = Joiner {
            who: Who::Member(Uuid::from_u128(account)),
            name: format!("Member {account}"),
        };
        room.join(joiner).ok().unwrap()
    }

    fn guest(room: &mut Room, name: &str) -> Result<Joined, Refusal> {
        let joiner = Joiner {
            who: Who::Guest {
                token: TOKEN.into(),
            },
            name: name.into(),
        };
        room.join(joiner)
    }

    fn texts(inbound: &mut Inbound) -> Vec<serde_json::Value> {
        let mut seen = Vec::new();
        while let Ok(out) = inbound.control.try_recv() {
            if let Some(Message::Text(text)) = super::opened(&inbound.roster, out) {
                seen.push(serde_json::from_str(&text).unwrap());
            }
        }
        seen
    }

    fn kinds(inbound: &mut Inbound) -> Vec<String> {
        texts(inbound)
            .iter()
            .map(|value| value["kind"].as_str().unwrap().to_owned())
            .collect()
    }

    fn camera(key: bool) -> Media {
        Media::Video {
            stream: Stream::Camera,
            key,
            payload: Bytes::from_static(&[1, 0, 0, 0, 0, 7]),
        }
    }

    #[test]
    fn a_full_receiver_drops_video_until_a_key_frame_and_the_sender_is_asked_for_one() {
        let (mut room, _stored) = opened(true, 12);
        let mut sender = member(&mut room, 1);
        let mut receiver = member(&mut room, 2);
        texts(&mut sender.inbound);
        for _ in 0..MEDIA_QUEUE {
            room.media(sender.slot, camera(false));
        }
        assert!(texts(&mut sender.inbound).is_empty());
        room.media(sender.slot, camera(false));
        let asked = texts(&mut sender.inbound);
        assert_eq!(asked, vec![json!({"kind": "keyframe", "stream": "camera"})]);
        while receiver.inbound.media.try_recv().is_ok() {}
        room.media(sender.slot, camera(false));
        assert!(
            receiver.inbound.media.try_recv().is_err(),
            "a delta waits for a key frame"
        );
        room.media(sender.slot, camera(true));
        let Ok(Message::Binary(frame)) = receiver.inbound.media.try_recv() else {
            panic!("the key frame is delivered");
        };
        assert_eq!(&frame[..4], &[2, sender.slot as u8, 0, 1]);
        room.media(sender.slot, camera(false));
        assert!(
            receiver.inbound.media.try_recv().is_ok(),
            "deltas follow again"
        );
    }

    #[test]
    fn microphone_audio_is_relayed_with_its_slot_only_while_the_microphone_is_on() {
        let (mut room, _stored) = opened(true, 12);
        let speaker = member(&mut room, 1);
        let mut listener = member(&mut room, 2);
        let pcm = Bytes::from_static(&[1, 2]);
        room.media(speaker.slot, Media::Audio(pcm.clone()));
        assert!(listener.inbound.media.try_recv().is_err());
        room.state(speaker.slot, [true, false, false, false]);
        room.media(speaker.slot, Media::Audio(pcm));
        let Ok(Message::Binary(frame)) = listener.inbound.media.try_recv() else {
            panic!("the audio is relayed");
        };
        assert_eq!(&frame[..], &wire::audio(speaker.slot, 16_000, &[1, 2])[..]);
    }

    #[test]
    fn a_guest_waits_until_a_member_admits_them_and_never_sees_ids() {
        let (mut room, mut stored) = opened(false, 12);
        let mut host = member(&mut room, 1);
        let mut dee = guest(&mut room, "Dee").ok().unwrap();
        assert_eq!(kinds(&mut dee.inbound), vec!["waiting"]);
        let lobby = texts(&mut host.inbound).pop().unwrap();
        assert_eq!(lobby["waiting"][0]["name"], "Dee");
        let id = lobby["waiting"][0]["id"].as_str().unwrap().to_owned();
        room.said(host.slot, Client::Admit { id });
        let seen = texts(&mut dee.inbound);
        assert_eq!(seen[0]["kind"], "welcome");
        assert_eq!(seen[0]["member"], false);
        let roster = seen.iter().find(|value| value["kind"] == "roster").unwrap();
        assert!(roster["participants"][0].get("account").is_none());
        assert_eq!(roster["participants"][0]["host"], true);
        let mut audited = Vec::new();
        while let Ok(record) = stored.try_recv() {
            if let Record::Audit(event) = record {
                audited.push(event);
            }
        }
        assert_eq!(audited.len(), 2, "the guest join and the admission");
    }

    #[test]
    fn guests_are_refused_by_link_lock_capacity_and_a_full_lobby() {
        let (mut room, _stored) = opened(true, 2);
        member(&mut room, 1);
        guest(&mut room, "One").ok().unwrap();
        assert_eq!(guest(&mut room, "Two").err(), Some(Refusal::Full));
        room.relink(Some("other".into()));
        assert_eq!(guest(&mut room, "Two").err(), Some(Refusal::Link));
        assert_eq!(room.seated(), 1, "the guest on the old link is dropped");
        room.relink(Some(TOKEN.into()));
        room.locked = true;
        assert_eq!(guest(&mut room, "Two").err(), Some(Refusal::Locked));
        let (mut lobby, _stored) = opened(false, 2);
        for index in 0..LOBBY_LIMIT {
            guest(&mut lobby, &format!("Guest {index}")).ok().unwrap();
        }
        assert_eq!(guest(&mut lobby, "Late").err(), Some(Refusal::LobbyFull));
    }

    #[test]
    fn only_the_host_ends_and_everyone_hears_it() {
        let (mut room, _stored) = opened(true, 12);
        let mut host = member(&mut room, 1);
        let mut other = member(&mut room, 2);
        room.said(other.slot, Client::End);
        assert!(!room.ended);
        room.said(host.slot, Client::End);
        assert!(room.ended);
        room.farewell();
        texts(&mut host.inbound);
        assert_eq!(kinds(&mut other.inbound).last().unwrap(), "ended");
    }

    #[test]
    fn agents_take_a_seat_speak_to_everyone_and_count_toward_capacity() {
        let (mut room, _stored) = opened(true, 2);
        let mut person = member(&mut room, 1);
        let avatar: Avatar =
            serde_json::from_value(json!({"paint": "sky", "eyes": "wink", "accessory": "crown"}))
                .unwrap();
        let slot = room
            .add_agent(Uuid::from_u128(5), "Scout".into(), avatar)
            .unwrap();
        assert_eq!(
            room.add_agent(Uuid::from_u128(6), "Full".into(), avatar),
            None
        );
        let seen = texts(&mut person.inbound);
        let roster = seen.iter().find(|value| value["kind"] == "roster").unwrap();
        assert_eq!(
            roster["participants"][1]["agent"],
            json!(Uuid::from_u128(5))
        );
        room.agent_audio(slot, wire::audio(slot, 24_000, &[0, 0]));
        assert!(person.inbound.media.try_recv().is_ok());
        let (refresh, _) = mpsc::channel(1);
        room.agent_screen(slot, Some(refresh));
        let roster = texts(&mut person.inbound).pop().unwrap();
        assert_eq!(roster["participants"][1]["screen"], true);
    }

    fn scout(room: &mut Room) -> u16 {
        let avatar: Avatar =
            serde_json::from_value(json!({"paint": "sky", "eyes": "wink", "accessory": "crown"}))
                .unwrap();
        room.add_agent(Uuid::from_u128(5), "Scout".into(), avatar)
            .unwrap()
    }

    #[test]
    fn state_and_track_frames_that_change_nothing_reach_no_one() {
        let (mut room, _stored) = opened(true, 12);
        let sender = member(&mut room, 1);
        let mut receiver = member(&mut room, 2);
        texts(&mut receiver.inbound);
        room.state(sender.slot, [true, false, false, false]);
        room.state(sender.slot, [true, false, false, false]);
        assert_eq!(kinds(&mut receiver.inbound), vec!["roster"]);
        let track = || Client::Track {
            stream: Stream::Camera,
            codec: Some("vp8".into()),
            width: 640,
            height: 480,
        };
        room.said(sender.slot, track());
        room.said(sender.slot, track());
        assert_eq!(kinds(&mut receiver.inbound), vec!["track"]);
        let off = || Client::Track {
            stream: Stream::Camera,
            codec: None,
            width: 0,
            height: 0,
        };
        room.said(sender.slot, off());
        room.said(sender.slot, off());
        assert_eq!(kinds(&mut receiver.inbound), vec!["track"]);
    }

    #[test]
    fn a_receiver_holds_at_most_one_roster_the_latest_and_is_never_dropped_for_rosters() {
        let (mut room, _stored) = opened(true, 12);
        let sender = member(&mut room, 1);
        let mut receiver = member(&mut room, 2);
        texts(&mut receiver.inbound);
        for turn in 0..CONTROL_QUEUE * 2 {
            let on = turn % 2 == 0;
            room.handle(Command::Said {
                slot: sender.slot,
                message: Client::State {
                    mic: on,
                    camera: false,
                    screen: false,
                    hand: !on,
                },
            });
        }
        room.state(sender.slot, [false, true, false, false]);
        assert_eq!(room.seated(), 2, "the receiver keeps its seat");
        let seen = texts(&mut receiver.inbound);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0]["participants"][0]["camera"], true);
    }

    #[test]
    fn desktop_tiles_a_receiver_cannot_take_are_dropped_and_ask_the_share_to_repaint() {
        let (mut room, _stored) = opened(true, 12);
        let mut receiver = member(&mut room, 1);
        let slot = scout(&mut room);
        let (refresh, mut refreshes) = mpsc::channel(1);
        room.agent_screen(slot, Some(refresh));
        let tiles = Server::Desktop {
            source: slot,
            tiles: Some(Vec::new()),
            ended: false,
        };
        for _ in 0..MEDIA_QUEUE {
            room.handle(Command::Announce {
                message: tiles.clone(),
                to: None,
            });
        }
        assert!(refreshes.try_recv().is_err());
        room.handle(Command::Announce {
            message: tiles,
            to: None,
        });
        assert!(refreshes.try_recv().is_ok());
        assert_eq!(room.seated(), 2, "the slow receiver keeps its seat");
        let Ok(Message::Text(first)) = receiver.inbound.media.try_recv() else {
            panic!("tiles travel with the media");
        };
        assert!(first.contains("desktop"));
    }

    #[test]
    fn only_admitted_people_keep_the_room_open() {
        let (mut room, _stored) = opened(false, 12);
        let dee = guest(&mut room, "Dee").ok().unwrap();
        assert!(
            room.deadline.is_some(),
            "a guest in the lobby keeps nothing open"
        );
        let host = member(&mut room, 1);
        assert!(room.deadline.is_none());
        scout(&mut room);
        room.leave(host.slot, None);
        assert!(
            room.deadline.is_some(),
            "a waiting guest and an agent are not people in the room"
        );
        room.leave(dee.slot, None);
        assert!(room.deadline.is_some());
    }

    #[test]
    fn room_events_are_never_dropped_and_only_microphone_audio_may_be() {
        let (mut room, _stored) = opened(true, 12);
        let (control, mut events) = mpsc::unbounded_channel();
        let (audio, mut heard) = mpsc::channel(1);
        room.handle(Command::Subscribe {
            events: Subscriber { control, audio },
        });
        for slot in 0..2000 {
            room.emit(Event::Left { slot });
        }
        for _ in 0..3 {
            room.emit(Event::Microphone {
                slot: 1,
                pcm: Bytes::from_static(&[0, 0]),
            });
        }
        let mut left = 0;
        while let Ok(Event::Left { .. }) = events.try_recv() {
            left += 1;
        }
        assert_eq!(left, 2000);
        assert!(heard.try_recv().is_ok());
        assert!(
            heard.try_recv().is_err(),
            "audio beyond the queue is dropped"
        );
        assert!(room.events.is_some());
    }
}

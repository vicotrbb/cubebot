//! The screen arbiter (ADR 0085): the only reader of the screen and the only writer of the
//! keyboard and tablet. It serves the guest agent's desktop streams and the host channel, a Unix
//! socket that `cubebot-sandbox desktop` relays for the orchestrator, one request at a time. It
//! holds the mode, refuses agent input while a person holds control and until the agent looks
//! again after they hand it back, refuses every agent request while a person holds control for a
//! login handoff (ADR 0039), and logs every request with its source: the agent and its tool
//! call, or the person's account. Each host channel connection gets frames of the tiles that
//! changed since its last one, encoded on its own relay thread.
use super::virtio::{Slot, gpu::Gpu, input::Input, vsock::Vsock};
use crate::kvm::memory::GuestMemory;
use cubebot_protocol::desktop::{
    Action, AgentRequest, Button, HostRequest, LINE_LIMIT, LOGIN_PENDING, Mode, PERSON_HAS_CONTROL,
    Pointer, REOBSERVE, Response, TILE, Tile,
};
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::Path,
    sync::mpsc::{self, Receiver, Sender},
};
use uuid::Uuid;

/// The host channel's socket; `CUBEBOT_GUEST_DESKTOP_SOCKET` moves it, such as for several
/// sandboxes sharing one host's /tmp.
pub const SOCKET: &str = "/tmp/cubebot-desktop.sock";
const NO_CONTROL: &str = "this person does not have control";
const OTHER_HAS_CONTROL: &str = "another person has control";
const SHIFT: u16 = 42;
/// Bounds what one request queues on the input devices.
const MAX_CLICKS: u8 = 3;
const MAX_PATH: usize = 100;
const MAX_WHEEL: i32 = 50;
const MAX_KEYS: usize = 8;
/// Input events per delivery, one shifted character, and the pause that lets the guest read one
/// delivery before the next, near xdotool's default 12 ms per key. A slower reader, such as Xorg
/// opening a popup, still drops a burst; the guest has no flow control back to the host.
/// shortcut: a long `type` holds the arbiter, and so every viewer's frames, for about 10 ms per
/// character; queue input apart from frames if that shows.
const BATCH: usize = 8;
const BATCH_PAUSE: std::time::Duration = std::time::Duration::from_millis(10);
/// US layout rows: unshifted and shifted characters from the first key code.
const ROWS: [(&str, &str, u16); 4] = [
    ("1234567890-=", "!@#$%^&*()_+", 2),
    ("qwertyuiop[]", "QWERTYUIOP{}", 16),
    ("asdfghjkl;'`", "ASDFGHJKL:\"~", 30),
    ("\\zxcvbnm,./", "|ZXCVBNM<>?", 43),
];
/// Key names as xdotool and the OpenAI and Anthropic computer tools write them, lowercased.
const KEYS: [(&str, u16); 73] = [
    ("escape", 1),
    ("esc", 1),
    ("backspace", 14),
    ("tab", 15),
    ("enter", 28),
    ("return", 28),
    ("ctrl", 29),
    ("control", 29),
    ("control_l", 29),
    ("ctrl_l", 29),
    ("shift", 42),
    ("shift_l", 42),
    ("alt", 56),
    ("alt_l", 56),
    ("option", 56),
    ("space", 57),
    ("capslock", 58),
    ("caps_lock", 58),
    ("f1", 59),
    ("f2", 60),
    ("f3", 61),
    ("f4", 62),
    ("f5", 63),
    ("f6", 64),
    ("f7", 65),
    ("f8", 66),
    ("f9", 67),
    ("f10", 68),
    ("f11", 87),
    ("f12", 88),
    ("home", 102),
    ("up", 103),
    ("arrowup", 103),
    ("pageup", 104),
    ("page_up", 104),
    ("prior", 104),
    ("left", 105),
    ("arrowleft", 105),
    ("right", 106),
    ("arrowright", 106),
    ("end", 107),
    ("down", 108),
    ("arrowdown", 108),
    ("pagedown", 109),
    ("page_down", 109),
    ("next", 109),
    ("insert", 110),
    ("delete", 111),
    ("del", 111),
    ("super", 125),
    ("super_l", 125),
    ("meta", 125),
    ("cmd", 125),
    ("command", 125),
    ("win", 125),
    ("minus", 12),
    ("equal", 13),
    ("comma", 51),
    ("period", 52),
    ("slash", 53),
    ("semicolon", 39),
    ("apostrophe", 40),
    ("grave", 41),
    ("bracketleft", 26),
    ("bracketright", 27),
    ("backslash", 43),
    ("ctrl_r", 97),
    ("control_r", 97),
    ("shift_r", 54),
    ("alt_r", 100),
    ("kp_enter", 96),
    ("print", 99),
    ("menu", 139),
];

/// One request and where its response goes.
pub enum Request {
    /// From the guest agent's stream on `port`.
    Guest { port: u32, line: Vec<u8> },
    Host {
        line: Vec<u8>,
        reply: Sender<Vec<u8>>,
    },
    /// The screen for a host channel frame.
    Frame { reply: Sender<Snapshot> },
}

pub struct Snapshot {
    mode: Mode,
    pointer: Pointer,
    /// BGRX rows of the screen's width.
    screen: Vec<u8>,
}

/// An evdev (type, code, value) event.
type Event = (u16, u16, u32);

pub struct Arbiter<'a> {
    pub gpu: &'a Slot<Gpu>,
    pub keyboard: &'a Slot<Input>,
    pub tablet: &'a Slot<Input>,
    pub memory: &'a GuestMemory,
    pub mode: Mode,
    holder: Option<Uuid>,
    pointer: [u32; 2],
    /// Who last moved the pointer: a person, or the agent when `None`.
    pointer_by: Option<Uuid>,
    /// Set when a person hands control back, until the agent's next screenshot.
    stale: bool,
    /// Set while a person holds control for a login handoff.
    handoff: bool,
}

impl<'a> Arbiter<'a> {
    pub fn new(
        gpu: &'a Slot<Gpu>,
        keyboard: &'a Slot<Input>,
        tablet: &'a Slot<Input>,
        memory: &'a GuestMemory,
    ) -> Self {
        Self {
            gpu,
            keyboard,
            tablet,
            memory,
            mode: Mode::Agent,
            holder: None,
            pointer: [0, 0],
            pointer_by: None,
            stale: false,
            handoff: false,
        }
    }

    /// Serves requests in arrival order for the life of the VM.
    pub fn serve(mut self, requests: Receiver<Request>, vsock: &Slot<Vsock>) {
        for request in requests {
            match request {
                Request::Guest { port, line } => {
                    let delivered = encode(&self.agent(&line)).and_then(|response| {
                        vsock.run(|vsock, queues| {
                            vsock.respond(port, &response);
                            vsock.deliver(queues, self.memory)
                        })
                    });
                    if let Err(error) = delivered {
                        super::fail(&format!("guest desktop stream: {error}"));
                    }
                }
                // A relay that went away no longer wants its response.
                // An unencodable response drops the reply, which the relay reports as an error.
                Request::Host { line, reply } => {
                    if let Ok(response) = encode(&self.host(&line)) {
                        drop(reply.send(response));
                    }
                }
                Request::Frame { reply } => drop(reply.send(self.snapshot())),
            }
        }
    }

    pub fn agent(&mut self, line: &[u8]) -> Response {
        let Ok(request) = serde_json::from_slice::<AgentRequest>(line) else {
            return self.invalid(json!({"source":"agent"}));
        };
        let source = json!({"source":"agent","call":request.call});
        let refusal = match request.action.is_input() {
            _ if self.handoff => Some(LOGIN_PENDING),
            true if self.mode == Mode::Human => Some(PERSON_HAS_CONTROL),
            true if self.stale => Some(REOBSERVE),
            _ => None,
        };
        let response = self.act(None, source, &request.action, refusal);
        if response.png.is_some() {
            self.stale = false;
        }
        response
    }

    pub fn host(&mut self, line: &[u8]) -> Response {
        match serde_json::from_slice(line) {
            Ok(HostRequest::Mode {
                mode,
                account,
                handoff,
            }) => self.switch(mode, account, handoff),
            Ok(HostRequest::Input { account, action }) => {
                let holds = self.mode == Mode::Human && self.holder == Some(account);
                let refusal = action.is_input() && !holds;
                let source = json!({"source":"person","account":account});
                self.act(
                    Some(account),
                    source,
                    &action,
                    refusal.then_some(NO_CONTROL),
                )
            }
            Ok(HostRequest::Frame) | Err(_) => self.invalid(json!({"source":"host"})),
        }
    }

    fn invalid(&self, source: Value) -> Response {
        eprintln!("Desktop refused an invalid request from {source}");
        self.response(Some("invalid desktop request".into()), None)
    }

    fn response(&self, error: Option<String>, png: Option<String>) -> Response {
        Response {
            mode: self.mode,
            error,
            png,
            ..Response::default()
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            mode: self.mode,
            pointer: Pointer {
                at: self.pointer,
                account: self.pointer_by,
            },
            screen: self.gpu.lock().device.screen.clone(),
        }
    }

    /// `human` hands control to `account`; any other mode takes it back. Only the person
    /// holding control changes the mode while they hold it.
    fn switch(&mut self, mode: Mode, account: Option<Uuid>, handoff: bool) -> Response {
        if mode == Mode::Human && account.is_none() {
            return self.response(Some("taking control names the person".into()), None);
        }
        if self.mode == Mode::Human && self.holder != account {
            return self.response(Some(OTHER_HAS_CONTROL.into()), None);
        }
        self.stale |= self.mode == Mode::Human && mode != Mode::Human;
        self.mode = mode;
        self.holder = account.filter(|_| mode == Mode::Human);
        self.handoff = handoff && mode == Mode::Human;
        eprintln!(
            "Desktop mode {}",
            json!({"mode":mode,"account":self.holder,"handoff":self.handoff})
        );
        self.response(None, None)
    }

    /// `by` is the person acting, or the agent when `None`.
    fn act(
        &mut self,
        by: Option<Uuid>,
        mut record: Value,
        action: &Action,
        refusal: Option<&str>,
    ) -> Response {
        let result = match refusal {
            Some(refusal) => Err(refusal.to_owned()),
            None => self.perform(action),
        };
        if result.is_ok()
            && !matches!(
                action,
                Action::Screenshot | Action::Mode | Action::Type { .. } | Action::Key { .. }
            )
        {
            self.pointer_by = by;
        }
        record["action"] = logged(action);
        if let Err(error) = &result {
            record["error"] = json!(error);
        }
        eprintln!("Desktop event {record}");
        match result {
            Ok(png) => self.response(None, png),
            Err(error) => self.response(Some(error), None),
        }
    }

    /// Sends the action's input, or returns the screen for a screenshot.
    fn perform(&mut self, action: &Action) -> Result<Option<String>, String> {
        let pointer = match action {
            Action::Screenshot => return Ok(Some(self.screenshot())),
            Action::Mode => return Ok(None),
            Action::Type { text } => return self.keys(typed(text)?),
            Action::Key { keys } => return self.keys(combination(keys)?),
            Action::Move { x, y } => (&Vec::new(), self.moved([*x, *y])?),
            Action::Click {
                at,
                button,
                count,
                hold,
            } => (hold, self.click(*at, *button, *count)?),
            Action::Drag { path, hold } => (hold, self.drag(path)?),
            Action::Scroll {
                at,
                down,
                right,
                hold,
            } => (hold, self.scroll(*at, *down, *right)?),
        };
        self.pointer(pointer.0, pointer.1)
    }

    fn screenshot(&self) -> String {
        let screen = self.gpu.lock().device.screen.clone();
        let png = crate::png::encode(
            super::virtio::gpu::WIDTH,
            super::virtio::gpu::HEIGHT,
            &screen,
        );
        crate::png::base64(&png)
    }

    fn keys(&self, events: Vec<Event>) -> Result<Option<String>, String> {
        send(self.keyboard, &events, self.memory)?;
        Ok(None)
    }

    /// Pointer events with `hold` keys pressed around them.
    fn pointer(&self, hold: &[String], events: Vec<Event>) -> Result<Option<String>, String> {
        let codes = hold
            .iter()
            .map(|name| key(name))
            .collect::<Result<Vec<_>, _>>()?;
        let (press, release) = held(&codes);
        send(self.keyboard, &press, self.memory)?;
        send(self.tablet, &events, self.memory)?;
        send(self.keyboard, &release, self.memory)?;
        Ok(None)
    }

    fn moved(&mut self, [x, y]: [u32; 2]) -> Result<Vec<Event>, String> {
        let (width, height) = (super::virtio::gpu::WIDTH, super::virtio::gpu::HEIGHT);
        if x >= width || y >= height {
            return Err(format!("({x}, {y}) is outside the {width}x{height} screen"));
        }
        self.pointer = [x, y];
        Ok(vec![(3, 0, x), (3, 1, y), (0, 0, 0)])
    }

    fn click(
        &mut self,
        at: Option<[u32; 2]>,
        button: Button,
        count: u8,
    ) -> Result<Vec<Event>, String> {
        if !(1..=MAX_CLICKS).contains(&count) {
            return Err(format!("a click counts 1 to {MAX_CLICKS} presses"));
        }
        let mut events = self.moved(at.unwrap_or(self.pointer))?;
        let code = match button {
            Button::Left => 0x110,
            Button::Right => 0x111,
            Button::Middle => 0x112,
        };
        for _ in 0..count {
            events.extend(pressed(code));
        }
        Ok(events)
    }

    fn drag(&mut self, path: &[[u32; 2]]) -> Result<Vec<Event>, String> {
        if !(2..=MAX_PATH).contains(&path.len()) {
            return Err(format!("a drag path has 2 to {MAX_PATH} points"));
        }
        let mut events = self.moved(path[0])?;
        events.extend([(1, 0x110, 1), (0, 0, 0)]);
        for &point in &path[1..] {
            events.extend(self.moved(point)?);
        }
        events.extend([(1, 0x110, 0), (0, 0, 0)]);
        Ok(events)
    }

    fn scroll(
        &mut self,
        at: Option<[u32; 2]>,
        down: i32,
        right: i32,
    ) -> Result<Vec<Event>, String> {
        if down.abs() > MAX_WHEEL || right.abs() > MAX_WHEEL {
            return Err(format!("a scroll moves at most {MAX_WHEEL} wheel clicks"));
        }
        let mut events = self.moved(at.unwrap_or(self.pointer))?;
        // REL_WHEEL counts up and REL_HWHEEL right, one click per event.
        let up = (-down.signum()) as u32;
        events.extend((0..down.abs()).flat_map(|_| [(2, 8, up), (0, 0, 0)]));
        let across = right.signum() as u32;
        events.extend((0..right.abs()).flat_map(|_| [(2, 6, across), (0, 0, 0)]));
        Ok(events)
    }
}

/// Sends events in batches the guest can take: its evdev clients buffer 64 events, and one
/// that overflows drops everything queued, so a burst such as a typed URL would vanish.
fn send(device: &Slot<Input>, events: &[Event], memory: &GuestMemory) -> Result<(), String> {
    for (at, batch) in events.chunks(BATCH).enumerate() {
        if at > 0 {
            std::thread::sleep(BATCH_PAUSE);
        }
        device
            .run(|input, queues| input.send(batch, queues, memory))
            .map_err(|error| format!("input device: {error}"))?;
    }
    Ok(())
}

fn pressed(code: u16) -> [Event; 4] {
    [(1, code, 1), (0, 0, 0), (1, code, 0), (0, 0, 0)]
}

/// Presses `codes` in order, and releases them in reverse.
fn held(codes: &[u16]) -> (Vec<Event>, Vec<Event>) {
    let press = codes.iter().flat_map(|&code| [(1, code, 1), (0, 0, 0)]);
    let release = codes
        .iter()
        .rev()
        .flat_map(|&code| [(1, code, 0), (0, 0, 0)]);
    (press.collect(), release.collect())
}

/// The key code and whether shift types `character` on a US layout.
fn character(character: char) -> Option<(u16, bool)> {
    match character {
        ' ' => return Some((57, false)),
        '\n' => return Some((28, false)),
        '\t' => return Some((15, false)),
        _ => (),
    }
    ROWS.iter().find_map(|&(plain, shifted, first)| {
        let at = |row: &str| row.chars().position(|c| c == character);
        at(plain)
            .map(|at| (first + at as u16, false))
            .or_else(|| at(shifted).map(|at| (first + at as u16, true)))
    })
}

fn key(name: &str) -> Result<u16, String> {
    let lower = name.to_ascii_lowercase();
    let mut characters = lower.chars();
    let single = match (characters.next(), characters.next()) {
        (Some(only), None) => character(only).map(|(code, _)| code),
        _ => None,
    };
    KEYS.iter()
        .find(|(known, _)| *known == lower)
        .map(|&(_, code)| code)
        .or(single)
        .ok_or_else(|| format!("unknown key {name:?}"))
}

fn combination(keys: &[String]) -> Result<Vec<Event>, String> {
    if keys.is_empty() || keys.len() > MAX_KEYS {
        return Err(format!("a key combination has 1 to {MAX_KEYS} keys"));
    }
    let codes = keys
        .iter()
        .map(|name| key(name))
        .collect::<Result<Vec<_>, _>>()?;
    let (mut press, release) = held(&codes);
    press.extend(release);
    Ok(press)
}

/// Every character's key presses, or which character a US layout cannot type.
fn typed(text: &str) -> Result<Vec<Event>, String> {
    let mut events = Vec::new();
    for c in text.chars() {
        let (code, shift) = character(c).ok_or_else(|| format!("cannot type {c:?}"))?;
        let codes: &[u16] = if shift { &[SHIFT, code] } else { &[code] };
        let (press, release) = held(codes);
        events.extend(press);
        events.extend(release);
    }
    Ok(events)
}

/// The action as logged: typed text may be a credential, so only its length is kept.
fn logged(action: &Action) -> Value {
    let mut value = json!(action);
    if let Action::Type { text } = action {
        value["text"] = json!(format!("{} characters", text.chars().count()));
    }
    value
}

fn encode(response: &Response) -> io::Result<Vec<u8>> {
    Ok(serde_json::to_vec(response)?)
}

/// Relays host channel lines to the arbiter, one connection per thread.
#[tracing::instrument(name = "desktop_listen", skip_all)]
pub fn listen(path: &Path, requests: Sender<Request>) -> io::Result<()> {
    // A socket left by an earlier run of this sandbox; the path is this sandbox's own.
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let requests = requests.clone();
            std::thread::spawn(move || {
                if let Err(error) = relay(stream, &requests) {
                    eprintln!("Desktop host channel closed: {error}");
                }
            });
        }
    });
    Ok(())
}

/// Serves one connection; whoever took control through it and still holds it when it closes,
/// such as an orchestrator that went away, hands control back.
#[tracing::instrument(name = "desktop_connection", skip_all)]
fn relay(stream: UnixStream, requests: &Sender<Request>) -> io::Result<()> {
    let mut held = None;
    let served = serve_connection(stream, requests, &mut held);
    if let Some(account) = held {
        let line = serde_json::to_vec(&HostRequest::Mode {
            mode: Mode::Watch,
            account: Some(account),
            handoff: false,
        })?;
        let (reply, response) = mpsc::channel();
        if requests.send(Request::Host { line, reply }).is_ok() {
            drop(response.recv());
        }
    }
    served
}

fn serve_connection(
    stream: UnixStream,
    requests: &Sender<Request>,
    held: &mut Option<Uuid>,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    // The screen as this connection's last frame showed it.
    let mut shown = Vec::new();
    loop {
        let mut line = Vec::new();
        let limit = LINE_LIMIT as u64 + 1;
        if reader.by_ref().take(limit).read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.pop() != Some(b'\n') {
            return Err(io::Error::other("request line too long or unterminated"));
        }
        let request = serde_json::from_slice(&line).ok();
        let mut response = match request {
            Some(HostRequest::Frame) => encode(&frame(requests, &mut shown)?)?,
            _ => ask(requests, line)?,
        };
        if let Some(HostRequest::Mode { mode, account, .. }) = request {
            // A refused change leaves a stale account here, whose release the arbiter refuses.
            *held = account.filter(|_| mode == Mode::Human);
        }
        response.push(b'\n');
        writer.write_all(&response)?;
    }
}

fn ask(requests: &Sender<Request>, line: Vec<u8>) -> io::Result<Vec<u8>> {
    let (reply, response) = mpsc::channel();
    requests
        .send(Request::Host { line, reply })
        .map_err(io::Error::other)?;
    response.recv().map_err(io::Error::other)
}

fn frame(requests: &Sender<Request>, shown: &mut Vec<u8>) -> io::Result<Response> {
    let (reply, snapshot) = mpsc::channel();
    requests
        .send(Request::Frame { reply })
        .map_err(io::Error::other)?;
    let snapshot = snapshot.recv().map_err(io::Error::other)?;
    Ok(Response {
        mode: snapshot.mode,
        tiles: tiles(&snapshot.screen, shown),
        pointer: Some(snapshot.pointer),
        ..Response::default()
    })
}

/// The tiles of `screen` that differ from `shown`, every tile when `shown` is not a screen of
/// the same size, as PNGs; `screen` becomes what is shown.
fn tiles(screen: &[u8], shown: &mut Vec<u8>) -> Vec<Tile> {
    let (width, height) = (super::virtio::gpu::WIDTH, super::virtio::gpu::HEIGHT);
    let stride = width as usize * 4;
    let fresh = shown.len() != screen.len();
    let mut changed = Vec::new();
    for y in (0..height).step_by(TILE as usize) {
        for x in (0..width).step_by(TILE as usize) {
            let (w, h) = (TILE.min(width - x), TILE.min(height - y));
            let rows = (y..y + h).map(|row| {
                let start = row as usize * stride + x as usize * 4;
                start..start + w as usize * 4
            });
            if !fresh && rows.clone().all(|row| screen[row.clone()] == shown[row]) {
                continue;
            }
            let pixels: Vec<u8> = rows.flat_map(|row| screen[row].iter().copied()).collect();
            let png = crate::png::base64(&crate::png::encode(w, h, &pixels));
            changed.push(Tile { x, y, png });
        }
    }
    shown.clear();
    shown.extend_from_slice(screen);
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Desktop {
        memory: GuestMemory,
        gpu: Slot<Gpu>,
        keyboard: Slot<Input>,
        tablet: Slot<Input>,
    }

    /// Devices whose interrupts go nowhere: the arbiter never raises one without a guest buffer.
    fn desktop() -> Desktop {
        let irq = crate::kvm::Irq { vm: -1, line: 0 };
        Desktop {
            memory: GuestMemory::map(1 << 16).unwrap(),
            gpu: Slot::new(Gpu::default(), irq),
            keyboard: Slot::new(Input::keyboard(), irq),
            tablet: Slot::new(Input::tablet(1280, 800), irq),
        }
    }

    fn line(value: Value) -> Vec<u8> {
        serde_json::to_vec(&value).unwrap()
    }

    #[test]
    fn agent_input_is_refused_while_a_person_holds_control() {
        let devices = desktop();
        let mut arbiter = Arbiter::new(
            &devices.gpu,
            &devices.keyboard,
            &devices.tablet,
            &devices.memory,
        );
        let call = Uuid::from_u128(1);
        let person = Uuid::from_u128(2);
        let click =
            json!({"call":call,"action":{"action":"click","at":[10,20],"button":"left","count":2}});
        assert_eq!(arbiter.agent(&line(click.clone())), Response::default());
        assert_eq!(arbiter.pointer, [10, 20]);
        let typing = json!({"input":{"account":person,"action":{"action":"type","text":"hi"}}});
        assert_eq!(
            arbiter.host(&line(typing.clone())).error.as_deref(),
            Some(NO_CONTROL)
        );
        let take = json!({"mode":{"mode":"human","account":person}});
        assert_eq!(arbiter.host(&line(take)).mode, Mode::Human);
        let refused = arbiter.agent(&line(click));
        assert_eq!(
            (refused.mode, refused.error.as_deref()),
            (Mode::Human, Some(PERSON_HAS_CONTROL))
        );
        let asked = arbiter.agent(&line(json!({"call":call,"action":{"action":"mode"}})));
        assert_eq!((asked.mode, asked.error), (Mode::Human, None));
        let screenshot = json!({"call":call,"action":{"action":"screenshot"}});
        let png = arbiter.agent(&line(screenshot)).png.unwrap();
        assert!(png.starts_with("iVBORw0KGgo"), "a base64 PNG");
        assert_eq!(arbiter.host(&line(typing.clone())).error, None);
        let other =
            json!({"input":{"account":Uuid::from_u128(3),"action":{"action":"key","keys":["a"]}}});
        assert_eq!(
            arbiter.host(&line(other)).error.as_deref(),
            Some(NO_CONTROL)
        );
        assert!(
            arbiter
                .host(&line(json!({"mode":{"mode":"human"}})))
                .error
                .is_some()
        );
        assert!(arbiter.agent(b"{\"call\":1}").error.is_some());
    }

    #[test]
    fn control_is_exclusive_and_the_agent_looks_again_after_it_returns() {
        let devices = desktop();
        let mut arbiter = Arbiter::new(
            &devices.gpu,
            &devices.keyboard,
            &devices.tablet,
            &devices.memory,
        );
        let (call, person) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let typing = json!({"input":{"account":person,"action":{"action":"type","text":"hi"}}});
        let take = json!({"mode":{"mode":"human","account":person}});
        assert_eq!(arbiter.host(&line(take)).mode, Mode::Human);
        let other_takes = json!({"mode":{"mode":"human","account":Uuid::from_u128(3)}});
        assert_eq!(
            arbiter.host(&line(other_takes)).error.as_deref(),
            Some(OTHER_HAS_CONTROL)
        );
        assert!(
            arbiter
                .host(&line(json!({"mode":{"mode":"watch"}})))
                .error
                .is_some()
        );
        let pointed = json!({"input":{"account":person,"action":{"action":"move","x":7,"y":8}}});
        assert_eq!(arbiter.host(&line(pointed)).error, None);
        assert_eq!(
            (arbiter.pointer, arbiter.pointer_by),
            ([7, 8], Some(person))
        );
        let hand_back = json!({"mode":{"mode":"watch","account":person}});
        assert_eq!(arbiter.host(&line(hand_back)).mode, Mode::Watch);
        assert_eq!(
            arbiter.host(&line(typing)).error.as_deref(),
            Some(NO_CONTROL)
        );
        let click =
            json!({"call":call,"action":{"action":"click","at":[10,20],"button":"left","count":1}});
        assert_eq!(
            arbiter.agent(&line(click.clone())).error.as_deref(),
            Some(REOBSERVE)
        );
        let screenshot = json!({"call":call,"action":{"action":"screenshot"}});
        assert!(arbiter.agent(&line(screenshot)).png.is_some());
        assert_eq!(arbiter.agent(&line(click)).error, None);
        assert_eq!(arbiter.pointer_by, None);
    }

    #[test]
    fn a_login_handoff_refuses_every_agent_request_until_control_returns() {
        let devices = desktop();
        let mut arbiter = Arbiter::new(
            &devices.gpu,
            &devices.keyboard,
            &devices.tablet,
            &devices.memory,
        );
        let (call, person) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let take = json!({"mode":{"mode":"human","account":person,"handoff":true}});
        assert_eq!(arbiter.host(&line(take)).error, None);
        let screenshot = json!({"call":call,"action":{"action":"screenshot"}});
        let mode = json!({"call":call,"action":{"action":"mode"}});
        for request in [&screenshot, &mode] {
            let refused = arbiter.agent(&line(request.clone()));
            assert_eq!(
                (refused.error.as_deref(), refused.png),
                (Some(LOGIN_PENDING), None)
            );
        }
        let typing = json!({"input":{"account":person,"action":{"action":"type","text":"pw"}}});
        assert_eq!(arbiter.host(&line(typing)).error, None);
        let hand_back = json!({"mode":{"mode":"watch","account":person}});
        assert_eq!(arbiter.host(&line(hand_back)).error, None);
        assert!(arbiter.agent(&line(screenshot)).png.is_some());
        assert_eq!(arbiter.agent(&line(mode)).error, None);
    }

    #[test]
    fn frames_carry_only_changed_tiles() {
        let (width, height) = (
            super::super::virtio::gpu::WIDTH,
            super::super::virtio::gpu::HEIGHT,
        );
        let mut screen = vec![0; width as usize * height as usize * 4];
        let mut shown = Vec::new();
        let all = tiles(&screen, &mut shown);
        assert_eq!(all.len(), 20 * 13, "the first frame has every tile");
        assert_eq!(shown, screen);
        assert!(tiles(&screen, &mut shown).is_empty());
        // One pixel in the bottom row of tiles, which is 32 pixels tall.
        screen[((790 * width + 130) * 4) as usize] = 255;
        let changed = tiles(&screen, &mut shown);
        assert_eq!(
            changed
                .iter()
                .map(|tile| (tile.x, tile.y))
                .collect::<Vec<_>>(),
            [(128, 768)]
        );
        assert!(changed[0].png.starts_with("iVBORw0KGgo"), "a base64 PNG");
    }

    #[test]
    fn actions_become_evdev_events() {
        let devices = desktop();
        let mut arbiter = Arbiter::new(
            &devices.gpu,
            &devices.keyboard,
            &devices.tablet,
            &devices.memory,
        );
        assert_eq!(
            arbiter.perform(&Action::Type { text: "A b".into() }),
            Ok(None)
        );
        let shifted_a = [
            (1, 42, 1),
            (0, 0, 0),
            (1, 30, 1),
            (0, 0, 0),
            (1, 30, 0),
            (0, 0, 0),
            (1, 42, 0),
            (0, 0, 0),
        ];
        assert_eq!(typed("A").unwrap(), shifted_a);
        let control_l = combination(&["Ctrl".into(), "L".into()]).unwrap();
        assert_eq!(
            control_l[..4],
            [(1, 29, 1), (0, 0, 0), (1, 38, 1), (0, 0, 0)]
        );
        assert_eq!(control_l[4..6], [(1, 38, 0), (0, 0, 0)]);
        assert!(typed("é").unwrap_err().contains("cannot type"));
        assert!(key("hyper").is_err());
        assert_eq!(key("Return"), Ok(28));
        let scrolled = arbiter.scroll(Some([5, 5]), 2, -1).unwrap();
        assert_eq!(scrolled[..3], [(3, 0, 5), (3, 1, 5), (0, 0, 0)]);
        let wheel = [
            (2, 8, u32::MAX),
            (0, 0, 0),
            (2, 8, u32::MAX),
            (0, 0, 0),
            (2, 6, u32::MAX),
            (0, 0, 0),
        ];
        assert_eq!(scrolled[3..], wheel);
        let clicked = arbiter.click(None, Button::Right, 1).unwrap();
        assert_eq!(
            clicked[3..],
            [(1, 0x111, 1), (0, 0, 0), (1, 0x111, 0), (0, 0, 0)]
        );
        assert!(arbiter.click(None, Button::Left, 4).is_err());
        assert!(arbiter.moved([1280, 0]).is_err());
        assert!(arbiter.drag(&[[1, 1]]).is_err());
        assert_eq!(
            logged(&Action::Type {
                text: "secret".into()
            })["text"],
            "6 characters"
        );
    }
}

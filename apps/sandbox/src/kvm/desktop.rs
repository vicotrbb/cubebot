//! The screen arbiter (ADR 0085): the only reader of the screen and the only writer of the
//! keyboard and tablet. It serves the guest agent's desktop streams and the host channel, a Unix
//! socket that `cubebot-sandbox desktop` relays for the orchestrator, one request at a time. It
//! holds the mode, refuses agent input while a person holds control, and logs every request with
//! its source: the agent and its tool call, or the person's account.
use super::virtio::{Slot, gpu::Gpu, input::Input, vsock::Vsock};
use crate::kvm::memory::GuestMemory;
use cubebot_protocol::desktop::{
    Action, AgentRequest, Button, HostRequest, LINE_LIMIT, Mode, PERSON_HAS_CONTROL, Response,
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
const SHIFT: u16 = 42;
/// Bounds what one request queues on the input devices.
const MAX_CLICKS: u8 = 3;
const MAX_PATH: usize = 100;
const MAX_WHEEL: i32 = 50;
const MAX_KEYS: usize = 8;
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
        }
    }

    /// Serves requests in arrival order for the life of the VM.
    pub fn serve(mut self, requests: Receiver<Request>, vsock: &Slot<Vsock>) {
        for request in requests {
            match request {
                Request::Guest { port, line } => {
                    let response = encode(&self.agent(&line));
                    let delivered = vsock.run(|vsock, queues| {
                        vsock.respond(port, &response);
                        vsock.deliver(queues, self.memory)
                    });
                    if let Err(error) = delivered {
                        super::fail(&format!("guest desktop stream: {error}"));
                    }
                }
                // A relay that went away no longer wants its response.
                Request::Host { line, reply } => drop(reply.send(encode(&self.host(&line)))),
            }
        }
    }

    pub fn agent(&mut self, line: &[u8]) -> Response {
        let Ok(request) = serde_json::from_slice::<AgentRequest>(line) else {
            return self.invalid(json!({"source":"agent"}));
        };
        let source = json!({"source":"agent","call":request.call});
        let refusal = request.action.is_input() && self.mode == Mode::Human;
        self.act(
            source,
            &request.action,
            refusal.then_some(PERSON_HAS_CONTROL),
        )
    }

    pub fn host(&mut self, line: &[u8]) -> Response {
        match serde_json::from_slice(line) {
            Ok(HostRequest::Mode { mode, account }) => self.switch(mode, account),
            Ok(HostRequest::Input { account, action }) => {
                let holds = self.mode == Mode::Human && self.holder == Some(account);
                let refusal = action.is_input() && !holds;
                let source = json!({"source":"person","account":account});
                self.act(source, &action, refusal.then_some(NO_CONTROL))
            }
            Err(_) => self.invalid(json!({"source":"host"})),
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
        }
    }

    /// `human` hands control to `account`; any other mode takes it back.
    fn switch(&mut self, mode: Mode, account: Option<Uuid>) -> Response {
        if mode == Mode::Human && account.is_none() {
            return self.response(Some("taking control names the person".into()), None);
        }
        self.mode = mode;
        self.holder = account.filter(|_| mode == Mode::Human);
        eprintln!(
            "Desktop mode {}",
            json!({"mode":mode,"account":self.holder})
        );
        self.response(None, None)
    }

    fn act(&mut self, mut record: Value, action: &Action, refusal: Option<&str>) -> Response {
        let result = match refusal {
            Some(refusal) => Err(refusal.to_owned()),
            None => self.perform(action),
        };
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

fn send(device: &Slot<Input>, events: &[Event], memory: &GuestMemory) -> Result<(), String> {
    device
        .run(|input, queues| input.send(events, queues, memory))
        .map_err(|error| format!("input device: {error}"))
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

fn encode(response: &Response) -> Vec<u8> {
    serde_json::to_vec(response).expect("response serializes")
}

/// Relays host channel lines to the arbiter, one connection per thread.
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

fn relay(stream: UnixStream, requests: &Sender<Request>) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    loop {
        let mut line = Vec::new();
        let limit = LINE_LIMIT as u64 + 1;
        if reader.by_ref().take(limit).read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if line.pop() != Some(b'\n') {
            return Err(io::Error::other("request line too long or unterminated"));
        }
        let (reply, response) = mpsc::channel();
        requests
            .send(Request::Host { line, reply })
            .map_err(io::Error::other)?;
        let mut response = response.recv().map_err(io::Error::other)?;
        response.push(b'\n');
        writer.write_all(&response)?;
    }
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
        assert_eq!(
            arbiter.host(&line(json!({"mode":{"mode":"watch"}}))).mode,
            Mode::Watch
        );
        assert_eq!(
            arbiter.host(&line(typing)).error.as_deref(),
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

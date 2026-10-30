//! The computer tool (ADR 0085): the provider's own computer-use tool, served by the sandbox's
//! screen arbiter over the guest's desktop port. Anthropic toolset calls arrive as
//! `{"action": member, ...input}` and OpenAI calls as `{"actions": [...]}`; each becomes arbiter
//! requests tagged with the call's id. An OpenAI call always answers with a screenshot.
use super::{Running, Sandbox, Tool};
use cubebot_protocol::{
    desktop::{Action, AgentRequest, Button, Response},
    terminal::Outcome,
};
use serde_json::Value;
use std::{
    fs::File,
    io::{self, BufRead, BufReader, Write},
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

/// Longest `wait` a call may ask for.
const LONGEST_WAIT: Duration = Duration::from_secs(30);
/// Lets the guest repaint before an OpenAI call's closing screenshot.
const SETTLE: Duration = Duration::from_millis(500);
/// Pixels per wheel click, as Chromium scrolls on Linux.
const WHEEL_PIXELS: i64 = 100;

pub(super) struct Computer {
    definition: cubebot_providers::Tool,
    /// One stream to the arbiter, opened on first use and reopened after a failure.
    desktop: Arc<Mutex<Option<BufReader<File>>>>,
}

pub(super) fn tool() -> Box<dyn Tool> {
    Box::new(Computer {
        definition: cubebot_providers::Tool {
            name: "computer".into(),
            description: String::new(),
            parameters: serde_json::json!({}),
            strict: false,
            computer: true,
        },
        desktop: Arc::default(),
    })
}

impl Tool for Computer {
    fn definition(&self) -> &cubebot_providers::Tool {
        &self.definition
    }

    fn call<'a>(
        &'a self,
        _sandbox: &'a Sandbox,
        id: Uuid,
        input: &Value,
    ) -> Result<Running<'a>, &'static str> {
        let plan = plan(input)?;
        let desktop = self.desktop.clone();
        Ok(Box::pin(async move {
            let ran = tokio::task::spawn_blocking(move || run(&desktop, id, plan)).await;
            ran.unwrap_or_else(|_| Outcome::Exited {
                code: None,
                output: "The desktop call failed.".into(),
            })
        }))
    }
}

#[derive(Debug, PartialEq)]
enum Step {
    Act(Action),
    Wait(Duration),
}

#[derive(Debug, PartialEq)]
struct Plan {
    steps: Vec<Step>,
    /// The call answers with the screen after its steps.
    screenshot: bool,
}

fn plan(input: &Value) -> Result<Plan, &'static str> {
    if let Some(actions) = input["actions"].as_array() {
        let mut steps = actions.iter().map(openai).collect::<Result<Vec<_>, _>>()?;
        if !steps.is_empty() {
            steps.push(Step::Wait(SETTLE));
        }
        return Ok(Plan {
            steps,
            screenshot: true,
        });
    }
    let action = input["action"].as_str().ok_or("no computer action")?;
    if action == "screenshot" {
        return Ok(Plan {
            steps: Vec::new(),
            screenshot: true,
        });
    }
    Ok(Plan {
        steps: anthropic(action, input)?,
        screenshot: false,
    })
}

/// Anthropic's click members.
const CLICKS: [(&str, Button, u8); 5] = [
    ("left_click", Button::Left, 1),
    ("right_click", Button::Right, 1),
    ("middle_click", Button::Middle, 1),
    ("double_click", Button::Left, 2),
    ("triple_click", Button::Left, 3),
];

/// One Anthropic computer toolset member.
fn anthropic(action: &str, input: &Value) -> Result<Vec<Step>, &'static str> {
    let at = point(&input["coordinate"]);
    let hold = keys(&input["text"], '+');
    if let Some(&(_, button, count)) = CLICKS.iter().find(|(name, ..)| *name == action) {
        let click = Action::Click {
            at,
            button,
            count,
            hold,
        };
        return Ok(vec![Step::Act(click)]);
    }
    let step = match action {
        "mouse_move" => {
            let [x, y] = at.ok_or("mouse_move needs a coordinate")?;
            Action::Move { x, y }
        }
        "left_click_drag" => {
            let start = point(&input["start_coordinate"]).ok_or("a drag needs a start")?;
            let path = vec![start, at.ok_or("a drag needs an end")?];
            Action::Drag { path, hold }
        }
        "scroll" => scroll(input, at, hold)?,
        "type" => Action::Type {
            text: input["text"].as_str().ok_or("type needs text")?.into(),
        },
        "key" => return repeated(input),
        "wait" => return Ok(vec![Step::Wait(seconds(&input["duration"]))]),
        _ => return Err("this computer action is not supported"),
    };
    Ok(vec![Step::Act(step)])
}

fn scroll(input: &Value, at: Option<[u32; 2]>, hold: Vec<String>) -> Result<Action, &'static str> {
    let amount = input["scroll_amount"].as_i64().unwrap_or(3).clamp(0, 50) as i32;
    let (down, right) = match input["scroll_direction"].as_str() {
        Some("down") => (amount, 0),
        Some("up") => (-amount, 0),
        Some("right") => (0, amount),
        Some("left") => (0, -amount),
        _ => return Err("scroll needs a direction"),
    };
    Ok(Action::Scroll {
        at,
        down,
        right,
        hold,
    })
}

fn repeated(input: &Value) -> Result<Vec<Step>, &'static str> {
    let keys = keys(&input["text"], '+');
    if keys.is_empty() {
        return Err("key needs a key");
    }
    let repeat = input["repeat"].as_u64().unwrap_or(1).clamp(1, 100) as usize;
    Ok((0..repeat)
        .map(|_| Step::Act(Action::Key { keys: keys.clone() }))
        .collect())
}

/// One action of an OpenAI computer call.
fn openai(action: &Value) -> Result<Step, &'static str> {
    let at = point(&json_pair(action));
    let hold: Vec<String> = action["keys"]
        .as_array()
        .map(|keys| {
            keys.iter()
                .filter_map(Value::as_str)
                .map(Into::into)
                .collect()
        })
        .unwrap_or_default();
    let step = match action["type"].as_str() {
        Some("screenshot") => return Ok(Step::Wait(Duration::ZERO)),
        Some("wait") => return Ok(Step::Wait(Duration::from_secs(1))),
        Some("click") => Action::Click {
            at,
            button: button(&action["button"])?,
            count: 1,
            hold,
        },
        Some("double_click") => Action::Click {
            at,
            button: Button::Left,
            count: 2,
            hold,
        },
        Some("move") => {
            let [x, y] = at.ok_or("move needs x and y")?;
            Action::Move { x, y }
        }
        Some("drag") => Action::Drag {
            path: path(&action["path"])?,
            hold: Vec::new(),
        },
        Some("scroll") => Action::Scroll {
            at,
            down: wheel(&action["scroll_y"]),
            right: wheel(&action["scroll_x"]),
            hold,
        },
        Some("type") => Action::Type {
            text: action["text"].as_str().ok_or("type needs text")?.into(),
        },
        Some("keypress") => Action::Key { keys: hold },
        _ => return Err("this computer action is not supported"),
    };
    Ok(Step::Act(step))
}

fn path(points: &Value) -> Result<Vec<[u32; 2]>, &'static str> {
    let points = points.as_array().ok_or("drag needs a path")?;
    let point = |value| point_of(value).ok_or("a drag point needs x and y");
    points.iter().map(point).collect()
}

fn button(value: &Value) -> Result<Button, &'static str> {
    match value.as_str().unwrap_or("left") {
        "left" => Ok(Button::Left),
        "right" => Ok(Button::Right),
        "wheel" | "middle" => Ok(Button::Middle),
        _ => Err("this mouse button is not supported"),
    }
}

/// Pixels to wheel clicks, at least one for any movement.
fn wheel(pixels: &Value) -> i32 {
    let pixels = pixels.as_i64().unwrap_or(0);
    let clicks = (pixels / WHEEL_PIXELS).clamp(-50, 50) as i32;
    match (clicks, pixels.signum()) {
        (0, sign) => sign as i32,
        (clicks, _) => clicks,
    }
}

fn json_pair(action: &Value) -> Value {
    serde_json::json!([action["x"], action["y"]])
}

fn point_of(value: &Value) -> Option<[u32; 2]> {
    point(&json_pair(value))
}

fn point(value: &Value) -> Option<[u32; 2]> {
    let [x, y] = value.as_array()?.as_slice() else {
        return None;
    };
    Some([x.as_u64()?.try_into().ok()?, y.as_u64()?.try_into().ok()?])
}

fn keys(value: &Value, separator: char) -> Vec<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(|text| text.split(separator).map(Into::into).collect())
        .unwrap_or_default()
}

fn seconds(value: &Value) -> Duration {
    Duration::from_secs_f64(value.as_f64().unwrap_or(1.0).max(0.0)).min(LONGEST_WAIT)
}

/// Runs the steps until the arbiter refuses one, then takes the screenshot the call answers with.
fn run(desktop: &Mutex<Option<BufReader<File>>>, call: Uuid, plan: Plan) -> Outcome {
    let mut desktop = desktop
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut refusal = None;
    for step in plan.steps {
        let action = match step {
            Step::Wait(duration) => {
                std::thread::sleep(duration);
                continue;
            }
            Step::Act(action) => action,
        };
        match request(&mut desktop, call, action) {
            Ok(Response { error: None, .. }) => (),
            Ok(Response { error, .. }) => {
                refusal = error;
                break;
            }
            Err(error) => return unreachable_desktop(&error),
        }
    }
    let output = refusal.clone().unwrap_or_else(|| "OK".into());
    if !plan.screenshot {
        let code = if refusal.is_some() { 1 } else { 0 };
        return Outcome::Exited {
            code: Some(code),
            output,
        };
    }
    match request(&mut desktop, call, Action::Screenshot) {
        Ok(Response {
            png: Some(image), ..
        }) => Outcome::Screenshot { output, image },
        Ok(Response { error, .. }) => Outcome::Exited {
            code: Some(1),
            output: error.unwrap_or_else(|| "The desktop returned no screenshot.".into()),
        },
        Err(error) => unreachable_desktop(&error),
    }
}

fn unreachable_desktop(error: &io::Error) -> Outcome {
    Outcome::Exited {
        code: None,
        output: format!("The desktop was not reached: {error}"),
    }
}

/// One request and its response line; a broken stream is dropped so the next call reconnects.
fn request(
    desktop: &mut Option<BufReader<File>>,
    call: Uuid,
    action: Action,
) -> io::Result<Response> {
    if desktop.is_none() {
        *desktop = Some(BufReader::new(connect()?));
    }
    let stream = desktop.as_mut().expect("connected");
    let exchanged = exchange(stream, &AgentRequest { call, action });
    if exchanged.is_err() {
        *desktop = None;
    }
    exchanged
}

fn exchange(stream: &mut BufReader<File>, request: &AgentRequest) -> io::Result<Response> {
    let mut line = serde_json::to_vec(request)?;
    line.push(b'\n');
    stream.get_mut().write_all(&line)?;
    let mut response = Vec::new();
    if stream.read_until(b'\n', &mut response)? == 0 {
        return Err(io::Error::other("the desktop stream ended"));
    }
    Ok(serde_json::from_slice(&response)?)
}

#[cfg(target_os = "linux")]
fn connect() -> io::Result<File> {
    use nix::sys::socket::{AddressFamily, SockFlag, SockType, VsockAddr, connect, socket};
    use std::os::fd::AsRawFd;
    let socket = socket(
        AddressFamily::Vsock,
        SockType::Stream,
        SockFlag::SOCK_CLOEXEC,
        None,
    )?;
    let host = cubebot_protocol::guest::HOST_CID;
    connect(
        socket.as_raw_fd(),
        &VsockAddr::new(host, cubebot_protocol::desktop::PORT),
    )?;
    Ok(File::from(socket))
}

#[cfg(not(target_os = "linux"))]
fn connect() -> io::Result<File> {
    Err(io::Error::other("the desktop needs the sandbox's guest"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn anthropic_members_become_arbiter_actions() {
        let click =
            plan(&json!({"action":"double_click","coordinate":[5,6],"text":"shift"})).unwrap();
        let expected = Action::Click {
            at: Some([5, 6]),
            button: Button::Left,
            count: 2,
            hold: vec!["shift".into()],
        };
        assert_eq!(click.steps, [Step::Act(expected)]);
        assert!(!click.screenshot);
        assert!(plan(&json!({"action":"screenshot"})).unwrap().screenshot);
        let keys = plan(&json!({"action":"key","text":"ctrl+l","repeat":2})).unwrap();
        let control_l = Action::Key {
            keys: vec!["ctrl".into(), "l".into()],
        };
        assert_eq!(
            keys.steps,
            [Step::Act(control_l.clone()), Step::Act(control_l)]
        );
        let scrolled =
            plan(&json!({"action":"scroll","scroll_direction":"up","scroll_amount":4})).unwrap();
        assert!(matches!(
            scrolled.steps[0],
            Step::Act(Action::Scroll {
                down: -4,
                right: 0,
                ..
            })
        ));
        let drag = json!({"action":"left_click_drag","start_coordinate":[1,2],"coordinate":[3,4]});
        assert!(
            matches!(&plan(&drag).unwrap().steps[0], Step::Act(Action::Drag { path, .. }) if path == &[[1, 2], [3, 4]])
        );
        assert!(plan(&json!({"action":"zoom","region":[0,0,1,1]})).is_err());
        assert!(plan(&json!({"action":"mouse_move"})).is_err());
    }

    #[test]
    fn openai_batches_end_with_a_screenshot() {
        let batch = json!({"actions":[
            {"type":"click","button":"right","x":405,"y":157},
            {"type":"type","text":"penguin"},
            {"type":"scroll","x":1,"y":2,"scroll_x":0,"scroll_y":-250},
            {"type":"keypress","keys":["CTRL","L"]},
        ]});
        let plan = plan(&batch).unwrap();
        assert!(plan.screenshot);
        assert!(matches!(
            plan.steps[0],
            Step::Act(Action::Click {
                at: Some([405, 157]),
                button: Button::Right,
                count: 1,
                ..
            })
        ));
        assert!(matches!(
            plan.steps[2],
            Step::Act(Action::Scroll {
                down: -2,
                right: 0,
                ..
            })
        ));
        assert_eq!(
            plan.steps[3],
            Step::Act(Action::Key {
                keys: vec!["CTRL".into(), "L".into()]
            })
        );
        assert_eq!(plan.steps[4], Step::Wait(SETTLE));
        assert_eq!(wheel(&json!(30)), 1);
        let only = super::plan(&json!({"actions":[{"type":"screenshot"}]})).unwrap();
        assert_eq!((only.steps.len(), only.screenshot), (2, true));
        assert!(
            super::plan(&json!({"actions":[{"type":"click","button":"back","x":1,"y":1}]}))
                .is_err()
        );
    }
}

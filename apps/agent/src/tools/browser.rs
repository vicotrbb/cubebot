//! Browser tools (ADRs 0036 and 0037) on the desktop's Chromium, which guest init starts with its
//! profile on the workspace disk, driven through Chromium's DevTools protocol on loopback. The
//! `main` session is a page in the profile's own context; any other session name is a task
//! session with a browser context of its own, whose cookies end when it closes. Calls address a
//! session's page directly, so one session never moves another's page or takes its input. Input
//! is refused while a person holds the desktop (ADR 0085), which the arbiter is asked first.
use super::{Running, Sandbox, Tool, computer};
use cubebot_protocol::{
    desktop::{Action, Mode, PERSON_HAS_CONTROL},
    terminal::Outcome,
};
use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fmt::Write,
    sync::{
        Arc, Mutex as Blocking,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    net::TcpStream,
    sync::{Mutex, oneshot},
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};
use uuid::Uuid;

/// Guest init starts Chromium with `--remote-debugging-port=9222`, which listens on loopback.
const ENDPOINT: &str = "http://127.0.0.1:9222/json/version";
const MAIN: &str = "main";
/// Bounds each DevTools reply, so a hung page fails the call rather than the turn.
const REPLY: Duration = Duration::from_secs(30);
const LOAD: Duration = Duration::from_secs(15);
/// Accessibility outline shown to the model; the rest of a long page is cut.
const OUTLINE_BYTES: usize = 32 * 1024;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Navigate,
    Read,
    Click,
    Type,
    Screenshot,
    Close,
}

struct BrowserTool {
    definition: cubebot_providers::Tool,
    kind: Kind,
    browser: Arc<Browser>,
}

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    let browser = Arc::new(Browser::default());
    let text = json!({"type":"string"});
    let session = json!({"type":"string","description":"main, or a task name for a separate session with its own page and cookies"});
    let node = json!({"type":"integer","description":"An element number from browser_read"});
    [
        (Kind::Navigate, "browser_navigate", "Open a URL in the session's page, opening the session on first use, and wait up to 15 s for it to load.", json!({"session":session,"url":text})),
        (Kind::Read, "browser_read", "The session page's accessibility tree: each element's [number], role, name and value.", json!({"session":session})),
        (Kind::Click, "browser_click", "Click an element by its number from browser_read. Read again to see the result.", json!({"session":session,"node":node})),
        (Kind::Type, "browser_type", "Focus an element by its number from browser_read and insert text at its cursor.", json!({"session":session,"node":node,"text":text})),
        (Kind::Screenshot, "browser_screenshot", "A PNG of the session page's visible area.", json!({"session":session})),
        (Kind::Close, "browser_close", "Close the session's page; a task session's cookies end with it.", json!({"session":session})),
    ]
    .into_iter()
    .map(|(kind, name, description, properties)| -> Box<dyn Tool> {
        let required: Vec<&String> = properties.as_object().into_iter().flatten().map(|(key, _)| key).collect();
        Box::new(BrowserTool {
            definition: cubebot_providers::Tool {
                name: name.into(),
                description: description.into(),
                parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
                strict: true,
                computer: false,
            },
            kind,
            browser: browser.clone(),
        })
    })
    .collect()
}

impl Tool for BrowserTool {
    fn definition(&self) -> &cubebot_providers::Tool {
        &self.definition
    }

    fn call<'a>(
        &'a self,
        _sandbox: &'a Sandbox,
        id: Uuid,
        input: &Value,
    ) -> Result<Running<'a>, &'static str> {
        validate(self.kind, input)?;
        let input = input.clone();
        Ok(Box::pin(async move {
            let result = self.browser.run(id, self.kind, &input).await;
            result.unwrap_or_else(|output| Outcome::Exited {
                code: Some(1),
                output,
            })
        }))
    }
}

fn validate(kind: Kind, input: &Value) -> Result<(), &'static str> {
    let named = input["session"]
        .as_str()
        .is_some_and(|name| (1..=64).contains(&name.len()));
    match kind {
        _ if !named => Err("a browser call needs a session name of 1 to 64 bytes"),
        Kind::Navigate if !input["url"].is_string() => Err("browser_navigate needs a url"),
        Kind::Click | Kind::Type if !input["node"].is_u64() => {
            Err("this call needs an element number")
        }
        Kind::Type if !input["text"].is_string() => Err("browser_type needs text"),
        _ => Ok(()),
    }
}

/// A session's page and, for a task session, the browser context it owns.
#[derive(Clone)]
struct Page {
    target: String,
    session: String,
    context: Option<String>,
}

#[derive(Default)]
struct Browser {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    cdp: Option<Arc<Cdp>>,
    pages: HashMap<String, Page>,
}

impl Browser {
    async fn run(&self, call: Uuid, kind: Kind, input: &Value) -> Result<Outcome, String> {
        let name = input["session"].as_str().unwrap_or(MAIN);
        if matches!(kind, Kind::Navigate | Kind::Click | Kind::Type) {
            agent_has_control(call).await?;
        }
        let (cdp, page) = self.page(name, kind == Kind::Navigate).await?;
        let result = act(&cdp, &page, kind, input).await;
        let gone = cdp.send(
            None,
            "Target.getTargetInfo",
            json!({"targetId":page.target}),
        );
        if kind == Kind::Close || (result.is_err() && gone.await.is_err()) {
            let mut state = self.state.lock().await;
            if state
                .pages
                .get(name)
                .is_some_and(|open| open.target == page.target)
            {
                state.pages.remove(name);
            }
        }
        result
    }

    /// The named session's page, opened only by a navigation so a closed or lost session is
    /// never silently replaced. A closed connection ends every session, since its pages'
    /// handles belong to that connection.
    async fn page(&self, name: &str, open: bool) -> Result<(Arc<Cdp>, Page), String> {
        let mut state = self.state.lock().await;
        let cdp = match state.cdp.clone().filter(|cdp| cdp.alive()) {
            Some(cdp) => cdp,
            None => {
                state.pages.clear();
                let cdp = Arc::new(Cdp::connect().await?);
                state.cdp = Some(cdp.clone());
                cdp
            }
        };
        if let Some(page) = state.pages.get(name) {
            return Ok((cdp, page.clone()));
        }
        if !open {
            return Err(
                "No open browser session has this name; browser_navigate opens one.".into(),
            );
        }
        let page = open_page(&cdp, name != MAIN).await?;
        state.pages.insert(name.into(), page.clone());
        Ok((cdp, page))
    }
}

/// The browser bypasses the arbiter's input devices, so it asks the arbiter's mode itself.
async fn agent_has_control(call: Uuid) -> Result<(), String> {
    let asked = tokio::task::spawn_blocking(move || computer::ask(call, Action::Mode)).await;
    match asked {
        Ok(Ok(response)) if response.mode == Mode::Human => Err(PERSON_HAS_CONTROL.into()),
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) => Err(format!("The desktop was not reached: {error}")),
        Err(_) => Err("The desktop was not reached.".into()),
    }
}

async fn open_page(cdp: &Cdp, task: bool) -> Result<Page, String> {
    let mut params = json!({"url":"about:blank"});
    let mut context = None;
    if task {
        let created = cdp
            .send(None, "Target.createBrowserContext", json!({}))
            .await?;
        context = Some(field(&created, "browserContextId")?);
        params["browserContextId"] = json!(context);
    }
    let target = field(
        &cdp.send(None, "Target.createTarget", params).await?,
        "targetId",
    )?;
    let attach = json!({"targetId":target,"flatten":true});
    let attached = cdp.send(None, "Target.attachToTarget", attach).await?;
    Ok(Page {
        session: field(&attached, "sessionId")?,
        target,
        context,
    })
}

async fn act(cdp: &Cdp, page: &Page, kind: Kind, input: &Value) -> Result<Outcome, String> {
    let on = |method: &'static str, params: Value| cdp.send(Some(&page.session), method, params);
    let node = json!({"backendNodeId":input["node"]});
    match kind {
        Kind::Navigate => navigate(cdp, page, &input["url"]).await,
        Kind::Read => {
            let tree = on("Accessibility.getFullAXTree", json!({})).await?;
            Ok(done(outline(
                tree["nodes"].as_array().map_or(&[], Vec::as_slice),
            )))
        }
        Kind::Click => click(cdp, page, node).await,
        Kind::Type => {
            on("DOM.focus", node).await?;
            on("Input.insertText", json!({"text":input["text"]})).await?;
            Ok(done("Typed.".into()))
        }
        Kind::Screenshot => {
            let shot = on("Page.captureScreenshot", json!({"format":"png"})).await?;
            let image = field(&shot, "data")?;
            Ok(Outcome::Screenshot {
                output: "OK".into(),
                image,
            })
        }
        Kind::Close => close(cdp, page).await,
    }
}

async fn click(cdp: &Cdp, page: &Page, node: Value) -> Result<Outcome, String> {
    let on = |method: &'static str, params: Value| cdp.send(Some(&page.session), method, params);
    on("DOM.scrollIntoViewIfNeeded", node.clone()).await?;
    let [x, y] = center(&on("DOM.getBoxModel", node).await?)?;
    for event in ["mousePressed", "mouseReleased"] {
        let click = json!({"type":event,"x":x,"y":y,"button":"left","clickCount":1});
        on("Input.dispatchMouseEvent", click).await?;
    }
    Ok(done("Clicked.".into()))
}

/// A task session's context goes with its page, and with it the session's cookies.
async fn close(cdp: &Cdp, page: &Page) -> Result<Outcome, String> {
    match &page.context {
        Some(context) => {
            let context = json!({"browserContextId":context});
            cdp.send(None, "Target.disposeBrowserContext", context)
                .await?
        }
        None => {
            let target = json!({"targetId":page.target});
            cdp.send(None, "Target.closeTarget", target).await?
        }
    };
    Ok(done("Closed.".into()))
}

/// Navigates, then waits for the document to finish loading, up to [`LOAD`].
async fn navigate(cdp: &Cdp, page: &Page, url: &Value) -> Result<Outcome, String> {
    let navigated = cdp
        .send(Some(&page.session), "Page.navigate", json!({"url":url}))
        .await?;
    if let Some(error) = navigated["errorText"]
        .as_str()
        .filter(|error| !error.is_empty())
    {
        return Err(format!("Navigation failed: {error}"));
    }
    let started = Instant::now();
    let probe = json!({"expression":"[document.readyState, location.href, document.title]","returnByValue":true});
    loop {
        let state = cdp
            .send(Some(&page.session), "Runtime.evaluate", probe.clone())
            .await?;
        let [ready, href, title] = [0, 1, 2].map(|at| {
            state["result"]["value"][at]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        });
        if ready == "complete" || started.elapsed() > LOAD {
            let loaded = if ready == "complete" {
                "Loaded"
            } else {
                "Still loading"
            };
            return Ok(done(format!("{loaded} {href}\nTitle: {title}")));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The middle of an element's content box, in the page's viewport pixels.
fn center(model: &Value) -> Result<[f64; 2], String> {
    let quad: Vec<f64> = model["model"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect();
    if quad.len() != 8 {
        return Err("The element has no box to click.".into());
    }
    Ok([0, 1].map(|axis| quad.iter().skip(axis).step_by(2).sum::<f64>() / 4.0))
}

/// The accessibility tree as indented lines, leaving out ignored and unnamed structural nodes
/// and lifting their children.
fn outline(nodes: &[Value]) -> String {
    let by_id: HashMap<&str, &Value> = nodes
        .iter()
        .filter_map(|node| Some((node["nodeId"].as_str()?, node)))
        .collect();
    let mut shown = String::new();
    let mut stack: Vec<(&Value, usize)> = nodes.first().map(|root| (root, 0)).into_iter().collect();
    while let Some((node, depth)) = stack.pop() {
        if shown.len() > OUTLINE_BYTES {
            shown.push_str("[... the rest of the page is omitted ...]\n");
            break;
        }
        let line = line(node);
        if let Some(line) = &line {
            let _ = writeln!(shown, "{}{line}", "  ".repeat(depth));
        }
        let children: Vec<_> = node["childIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|id| by_id.get(id.as_str()?))
            .collect();
        let depth = depth + usize::from(line.is_some());
        stack.extend(children.into_iter().rev().map(|child| (*child, depth)));
    }
    shown
}

fn line(node: &Value) -> Option<String> {
    let role = node["role"]["value"].as_str().unwrap_or_default();
    let name = node["name"]["value"].as_str().unwrap_or_default();
    let structural = matches!(role, "" | "none" | "generic");
    // Inline text boxes repeat their static text.
    if node["ignored"] == true || role == "InlineTextBox" || (structural && name.is_empty()) {
        return None;
    }
    let mut line = match node["backendDOMNodeId"].as_u64() {
        Some(number) => format!("[{number}] {role}"),
        None => role.to_owned(),
    };
    if !name.is_empty() {
        let _ = write!(line, " {name:?}");
    }
    match &node["value"]["value"] {
        Value::Null => {}
        Value::String(value) => {
            let _ = write!(line, " = {value:?}");
        }
        value => {
            let _ = write!(line, " = {value}");
        }
    }
    Some(line)
}

fn done(output: String) -> Outcome {
    Outcome::Exited {
        code: Some(0),
        output,
    }
}

fn field(result: &Value, name: &str) -> Result<String, String> {
    result[name]
        .as_str()
        .map(Into::into)
        .ok_or_else(|| format!("The browser's reply had no {name}."))
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Waiting = Arc<Blocking<HashMap<u64, oneshot::Sender<Value>>>>;

/// One DevTools connection to the browser; page commands travel on it with their session id.
struct Cdp {
    sink: Mutex<SplitSink<Socket, Message>>,
    waiting: Waiting,
    next: AtomicU64,
    reader: tokio::task::JoinHandle<()>,
}

impl Cdp {
    async fn connect() -> Result<Self, String> {
        let unreached =
            |error: &dyn std::fmt::Display| format!("The browser was not reached: {error}");
        let version: Value = reqwest::get(ENDPOINT)
            .await
            .map_err(|error| unreached(&error))?
            .json()
            .await
            .map_err(|error| unreached(&error))?;
        let url = version["webSocketDebuggerUrl"]
            .as_str()
            .ok_or("The browser named no DevTools endpoint.")?;
        let (socket, _) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|error| unreached(&error))?;
        let (sink, mut stream) = socket.split();
        let waiting = Waiting::default();
        let replies = waiting.clone();
        // Replies go to their waiting command; events carry no id and are not used.
        let reader = tokio::spawn(async move {
            while let Some(Ok(message)) = stream.next().await {
                let Message::Text(text) = message else {
                    continue;
                };
                let Ok(reply) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                let waiter = reply["id"]
                    .as_u64()
                    .and_then(|id| lock(&replies).remove(&id));
                if let Some(waiter) = waiter {
                    let _ = waiter.send(reply);
                }
            }
            lock(&replies).clear();
        });
        Ok(Self {
            sink: Mutex::new(sink),
            waiting,
            next: AtomicU64::new(1),
            reader,
        })
    }

    fn alive(&self) -> bool {
        !self.reader.is_finished()
    }

    async fn send(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut command = json!({"id":id,"method":method,"params":params});
        if let Some(session) = session {
            command["sessionId"] = json!(session);
        }
        let (sender, reply) = oneshot::channel();
        lock(&self.waiting).insert(id, sender);
        let sent = self
            .sink
            .lock()
            .await
            .send(Message::Text(command.to_string().into()))
            .await;
        if let Err(error) = sent {
            lock(&self.waiting).remove(&id);
            return Err(format!("The browser was not reached: {error}"));
        }
        let mut reply = match tokio::time::timeout(REPLY, reply).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => return Err("The browser connection closed.".into()),
            Err(_) => {
                lock(&self.waiting).remove(&id);
                return Err(format!(
                    "{method} got no reply within {} s.",
                    REPLY.as_secs()
                ));
            }
        };
        match reply.get("error") {
            Some(error) => Err(format!(
                "{method} failed: {}",
                error["message"].as_str().unwrap_or("no reason given")
            )),
            None => Ok(reply["result"].take()),
        }
    }
}

fn lock(waiting: &Waiting) -> std::sync::MutexGuard<'_, HashMap<u64, oneshot::Sender<Value>>> {
    waiting
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Drop for Cdp {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_need_a_session_and_their_arguments() {
        let main = json!({"session":"main","node":7,"text":"hi","url":"https://example.com"});
        for kind in [
            Kind::Navigate,
            Kind::Read,
            Kind::Click,
            Kind::Type,
            Kind::Screenshot,
            Kind::Close,
        ] {
            assert!(validate(kind, &main).is_ok());
        }
        assert!(validate(Kind::Read, &json!({"session":""})).is_err());
        assert!(validate(Kind::Read, &json!({"session":"x".repeat(65)})).is_err());
        assert!(validate(Kind::Navigate, &json!({"session":"main"})).is_err());
        assert!(validate(Kind::Click, &json!({"session":"main","node":-1})).is_err());
        assert!(validate(Kind::Type, &json!({"session":"main","node":1})).is_err());
    }

    #[test]
    fn the_outline_numbers_elements_and_lifts_unnamed_structure() {
        let nodes = json!([
            {"nodeId":"1","role":{"value":"RootWebArea"},"name":{"value":"Sign in"},"backendDOMNodeId":1,"childIds":["2","5"]},
            {"nodeId":"2","role":{"value":"generic"},"childIds":["3","4"]},
            {"nodeId":"3","role":{"value":"textbox"},"name":{"value":"Email"},"value":{"value":"a@b.c"},"backendDOMNodeId":12,"childIds":[]},
            {"nodeId":"4","ignored":true,"role":{"value":"button"},"name":{"value":"Hidden"},"backendDOMNodeId":13},
            {"nodeId":"5","role":{"value":"button"},"name":{"value":"Next"},"backendDOMNodeId":14,"childIds":["6"]},
            {"nodeId":"6","role":{"value":"StaticText"},"name":{"value":"Next"},"childIds":["7"]},
            {"nodeId":"7","role":{"value":"InlineTextBox"},"name":{"value":"Next"},"childIds":[]},
        ]);
        let shown = outline(nodes.as_array().unwrap());
        assert_eq!(
            shown,
            "[1] RootWebArea \"Sign in\"\n  [12] textbox \"Email\" = \"a@b.c\"\n  [14] button \"Next\"\n    StaticText \"Next\"\n"
        );
    }

    #[test]
    fn a_click_lands_in_the_middle_of_the_content_box() {
        let model = json!({"model":{"content":[10.0,20.0,30.0,20.0,30.0,40.0,10.0,40.0]}});
        assert_eq!(center(&model).unwrap(), [20.0, 30.0]);
        assert!(center(&json!({"model":{"content":[]}})).is_err());
    }
}

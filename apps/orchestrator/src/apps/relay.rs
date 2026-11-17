use super::Apps;
use crate::{
    agents::AgentId,
    identity::{Actor, IdentityError},
    work::access,
};
use axum::{
    body::Body,
    extract::{
        Request,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use cubebot_protocol::apps::{AppNotice, AppReport, Frame, VIEWER_ID, VIEWER_NAME, decode, encode};
use hyper_util::rt::TokioIo;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::{mpsc, oneshot},
    time::Instant,
};
use uuid::Uuid;

const STREAM_BYTES: usize = 64 * 1024;
const CHUNK: usize = 16 * 1024;
const TEXT_BYTES: usize = 16 * 1024;
const QUEUED: usize = 8;
const OUTGOING: usize = 64;
const STALL: Duration = Duration::from_secs(10);
const PING: Duration = Duration::from_secs(5);
const SILENCE: Duration = Duration::from_secs(15);
const ANSWER: Duration = Duration::from_secs(60);
const LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);
const REUSE: Duration = Duration::from_secs(6 * 60 * 60);

const POLICY: &str = "sandbox allow-scripts allow-forms allow-popups allow-downloads; default-src 'self'; base-uri 'none'; frame-ancestors 'self'; connect-src 'self'; img-src 'self' data: blob:; media-src 'self' blob:; font-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'";
const UNAVAILABLE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Unavailable</title><style>body{margin:0;min-height:100vh;display:grid;place-items:center;font:16px system-ui,sans-serif;color:#555;background:#fafafa}@media (prefers-color-scheme:dark){body{color:#aaa;background:#161616}}</style></head><body><p>This app is unavailable right now</p></body></html>";

enum Out {
    Open(u32, mpsc::Sender<Vec<u8>>),
    Data(u32, Vec<u8>),
    Close(u32),
    Notice(AppNotice),
}

struct Link {
    out: mpsc::Sender<Out>,
    next: Arc<AtomicU32>,
    _stop: oneshot::Sender<()>,
}

#[derive(Clone, Default)]
pub struct Links(Arc<Mutex<HashMap<AgentId, Link>>>);

impl Links {
    fn lock(&self) -> MutexGuard<'_, HashMap<AgentId, Link>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn connected(&self, agent: AgentId) -> bool {
        self.lock().contains_key(&agent)
    }

    pub fn notify(&self, agent: AgentId, notice: AppNotice) {
        if let Some(link) = self.lock().get(&agent) {
            let _ = link.out.try_send(Out::Notice(notice));
        }
    }

    pub async fn open(&self, agent: AgentId) -> Option<DuplexStream> {
        let (out, stream) = {
            let links = self.lock();
            let link = links.get(&agent)?;
            (link.out.clone(), link.next.fetch_add(1, Ordering::Relaxed))
        };
        let (mine, theirs) = tokio::io::duplex(STREAM_BYTES);
        let (inbound, received) = mpsc::channel(QUEUED);
        out.send(Out::Open(stream, inbound)).await.ok()?;
        tokio::spawn(pump(stream, theirs, received, out));
        Some(mine)
    }

    pub async fn serve(
        &self,
        agent: AgentId,
        mut socket: WebSocket,
        reports: mpsc::Sender<AppReport>,
    ) {
        let (out, mut outgoing) = mpsc::channel(OUTGOING);
        let (stop, mut stopped) = oneshot::channel();
        let link = Link {
            out: out.clone(),
            next: Arc::new(AtomicU32::new(1)),
            _stop: stop,
        };
        self.lock().insert(agent, link);
        let mut streams = HashMap::new();
        let mut ping = tokio::time::interval(PING);
        let mut heard = Instant::now();
        loop {
            let going = tokio::select! {
                _ = &mut stopped => false,
                message = socket.recv() => {
                    heard = Instant::now();
                    inbound(&mut streams, &reports, message).await
                }
                Some(item) = outgoing.recv() => outbound(&mut socket, &mut streams, item).await,
                _ = ping.tick() => socket.send(Message::Ping(Default::default())).await.is_ok(),
                () = tokio::time::sleep_until(heard + SILENCE) => false,
            };
            if !going {
                break;
            }
        }
        let mut links = self.lock();
        if links
            .get(&agent)
            .is_some_and(|link| link.out.same_channel(&out))
        {
            links.remove(&agent);
        }
    }
}

async fn inbound(
    streams: &mut HashMap<u32, mpsc::Sender<Vec<u8>>>,
    reports: &mpsc::Sender<AppReport>,
    message: Option<Result<Message, axum::Error>>,
) -> bool {
    match message {
        Some(Ok(Message::Binary(bytes))) => {
            deliver(streams, &bytes).await;
            true
        }
        Some(Ok(Message::Text(text))) => {
            let report = (text.len() <= TEXT_BYTES)
                .then(|| serde_json::from_str::<AppReport>(text.as_str()).ok())
                .flatten();
            match report {
                Some(report) => reports.send(report).await.is_ok(),
                None => true,
            }
        }
        Some(Ok(Message::Ping(_) | Message::Pong(_))) => true,
        _ => false,
    }
}

async fn deliver(streams: &mut HashMap<u32, mpsc::Sender<Vec<u8>>>, bytes: &[u8]) {
    match decode(bytes) {
        Some(Frame::Data(stream, data)) => {
            let Some(sender) = streams.get(&stream) else {
                return;
            };
            let sent = tokio::time::timeout(STALL, sender.send(data.to_vec())).await;
            if !matches!(sent, Ok(Ok(()))) {
                streams.remove(&stream);
            }
        }
        Some(Frame::Close(stream)) => {
            streams.remove(&stream);
        }
        _ => {}
    }
}

async fn outbound(
    socket: &mut WebSocket,
    streams: &mut HashMap<u32, mpsc::Sender<Vec<u8>>>,
    item: Out,
) -> bool {
    let frame = match item {
        Out::Open(stream, sender) => {
            streams.insert(stream, sender);
            encode(&Frame::Open(stream))
        }
        Out::Data(stream, data) => encode(&Frame::Data(stream, &data)),
        Out::Close(stream) => {
            streams.remove(&stream);
            encode(&Frame::Close(stream))
        }
        Out::Notice(notice) => {
            let text = serde_json::to_string(&notice).unwrap_or_default();
            return socket.send(Message::Text(text.into())).await.is_ok();
        }
    };
    socket.send(Message::Binary(frame.into())).await.is_ok()
}

async fn pump(
    stream: u32,
    io: DuplexStream,
    mut inbound: mpsc::Receiver<Vec<u8>>,
    out: mpsc::Sender<Out>,
) {
    let (mut reader, mut writer) = tokio::io::split(io);
    let up = async {
        let mut buffer = vec![0; CHUNK];
        loop {
            let read = match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            if out
                .send(Out::Data(stream, buffer[..read].to_vec()))
                .await
                .is_err()
            {
                break;
            }
        }
    };
    let down = async {
        while let Some(data) = inbound.recv().await {
            if writer.write_all(&data).await.is_err() {
                break;
            }
        }
    };
    tokio::select! {
        () = up => {}
        () = down => {}
    }
    let _ = out.send(Out::Close(stream)).await;
}

#[derive(Clone)]
pub(super) struct View {
    pub(super) viewer: Uuid,
    pub(super) name: String,
    pub(super) app: Uuid,
    pub(super) agent: AgentId,
    pub(super) slug: String,
    expires: Instant,
}

#[derive(Clone, Default)]
pub(super) struct Views(Arc<Mutex<HashMap<String, View>>>);

impl Views {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, View>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn mint(
        &self,
        viewer: Uuid,
        name: String,
        app: Uuid,
        agent: AgentId,
        slug: &str,
    ) -> String {
        let now = Instant::now();
        let mut views = self.lock();
        views.retain(|_, view| view.expires > now);
        let reused = views.iter().find(|(_, view)| {
            view.viewer == viewer && view.app == app && view.expires > now + REUSE
        });
        if let Some((token, _)) = reused {
            return token.clone();
        }
        let token = crate::identity::token();
        let view = View {
            viewer,
            name,
            app,
            agent,
            slug: slug.to_owned(),
            expires: now + LIFETIME,
        };
        views.insert(token.clone(), view);
        token
    }

    pub(super) fn find(&self, slug: &str, token: &str) -> Option<View> {
        let mut views = self.lock();
        let view = views.get(token)?.clone();
        if view.expires <= Instant::now() {
            views.remove(token);
            return None;
        }
        (view.slug == slug).then_some(view)
    }
}

struct Target<'a> {
    slug: &'a str,
    token: &'a str,
    rest: Option<&'a str>,
}

fn target(path: &str) -> Option<Target<'_>> {
    let (slug, rest) = path.strip_prefix("/api/apps/")?.split_once('/')?;
    let rest = rest.strip_prefix("v/")?;
    let (token, rest) = match rest.find('/') {
        Some(at) => (&rest[..at], Some(&rest[at..])),
        None => (rest, None),
    };
    Some(Target { slug, token, rest })
}

impl Apps {
    #[tracing::instrument(skip_all, fields(actor = %actor.id, slug))]
    pub async fn view(&self, actor: &Actor, slug: &str) -> Result<String, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let (app, agent) = super::live(&mut tx, slug).await?;
        access::conversation(&mut tx, actor, agent, "conversation.send").await?;
        let name = sqlx::query_scalar!(
            r#"SELECT COALESCE(NULLIF(profile->>'name', ''), login) AS "name!" FROM human_accounts WHERE id = $1"#,
            actor.id,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(self.views.mint(actor.id, name, app, AgentId(agent), slug))
    }

    async fn admit(&self, slug: &str, token: &str) -> Result<View, IdentityError> {
        let view = self
            .views
            .find(slug, token)
            .ok_or(IdentityError::NotFound)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query_scalar!(
            "
            SELECT p.id
            FROM apps p
            JOIN agents g ON g.id = p.agent_id AND g.desired_state <> 'retired'
            WHERE p.id = $1 AND p.agent_id = $2 AND p.slug = $3
            ",
            view.app,
            view.agent.0,
            slug,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        let viewer = Actor {
            id: view.viewer,
            administrator: false,
        };
        access::conversation(&mut tx, &viewer, view.agent.0, "conversation.send").await?;
        tx.commit().await?;
        Ok(view)
    }

    pub async fn link(&self, agent: AgentId, socket: WebSocket) {
        let (reports, mut received) = mpsc::channel(OUTGOING);
        let apps = self.clone();
        tokio::spawn(async move {
            while let Some(report) = received.recv().await {
                if let Err(error) = apps.report(agent, report).await {
                    tracing::warn!(?error, "app report not recorded");
                }
            }
        });
        self.links.serve(agent, socket, reports).await;
    }

    pub async fn relay(&self, request: Request) -> Response {
        let uri = request.uri().clone();
        let Some(target) = target(uri.path()) else {
            return IdentityError::NotFound.into_response();
        };
        let prefix = format!("/api/apps/{}/v/{}", target.slug, target.token);
        let Some(rest) = target.rest else {
            let query = uri
                .query()
                .map(|query| format!("?{query}"))
                .unwrap_or_default();
            let location = format!("{prefix}/{query}");
            return (
                StatusCode::PERMANENT_REDIRECT,
                [(header::LOCATION, location)],
            )
                .into_response();
        };
        let view = match self.admit(target.slug, target.token).await {
            Ok(view) => view,
            Err(error) => return error.into_response(),
        };
        if request.method() == Method::OPTIONS
            && request
                .headers()
                .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD)
        {
            return preflight(request.headers());
        }
        forward(&self.links, &view, &prefix, rest, request).await
    }
}

pub(super) async fn forward(
    links: &Links,
    view: &View,
    prefix: &str,
    rest: &str,
    mut request: Request,
) -> Response {
    let upgrade = request
        .headers()
        .contains_key(header::UPGRADE)
        .then(|| hyper::upgrade::on(&mut request));
    let Some(io) = links.open(view.agent).await else {
        return unavailable();
    };
    let Some(request) = forwarded(request, view, rest) else {
        return unavailable();
    };
    let Some(mut response) = exchange(io, request).await else {
        return unavailable();
    };
    if response.status() == StatusCode::SWITCHING_PROTOCOLS
        && let Some(browser) = upgrade
    {
        let app = hyper::upgrade::on(&mut response);
        tokio::spawn(splice(browser, app));
    }
    answered(response, &view.slug, prefix)
}

async fn splice(browser: hyper::upgrade::OnUpgrade, app: hyper::upgrade::OnUpgrade) {
    let (Ok(browser), Ok(app)) = tokio::join!(browser, app) else {
        return;
    };
    let _ = tokio::io::copy_bidirectional(&mut TokioIo::new(browser), &mut TokioIo::new(app)).await;
}

async fn exchange(
    io: DuplexStream,
    request: Request,
) -> Option<hyper::Response<hyper::body::Incoming>> {
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(io))
        .await
        .ok()?;
    tokio::spawn(connection.with_upgrades());
    tokio::time::timeout(ANSWER, sender.send_request(request))
        .await
        .ok()?
        .ok()
}

fn withheld(name: &HeaderName) -> bool {
    let name = name.as_str();
    matches!(
        name,
        "cookie" | "authorization" | "proxy-authorization" | "host"
    ) || name.starts_with("x-cubebot-")
}

fn forwarded(request: Request, view: &View, rest: &str) -> Option<Request> {
    let (mut parts, body) = request.into_parts();
    let query = parts
        .uri
        .query()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    parts.uri = Uri::try_from(format!("/{}{rest}{query}", view.slug)).ok()?;
    let mut headers = HeaderMap::with_capacity(parts.headers.len() + 3);
    for (name, value) in &parts.headers {
        if !withheld(name) {
            headers.append(name.clone(), value.clone());
        }
    }
    headers.insert(header::HOST, HeaderValue::from_static("localhost"));
    headers.insert(
        VIEWER_ID,
        HeaderValue::from_str(&view.viewer.to_string()).ok()?,
    );
    headers.insert(
        VIEWER_NAME,
        HeaderValue::from_str(&encoded(&view.name)).ok()?,
    );
    parts.headers = headers;
    Some(Request::from_parts(parts, body))
}

fn encoded(name: &str) -> String {
    name.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b' ' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn located(location: &str, slug: &str, prefix: &str) -> Option<String> {
    let rest = location.strip_prefix('/')?.strip_prefix(slug)?;
    match rest.chars().next() {
        None => Some(format!("{prefix}/")),
        Some('/') => Some(format!("{prefix}{rest}")),
        Some('?') => Some(format!("{prefix}/{rest}")),
        Some(_) => None,
    }
}

fn answered(
    response: hyper::Response<hyper::body::Incoming>,
    slug: &str,
    prefix: &str,
) -> Response {
    let (mut parts, body) = response.into_parts();
    for name in [
        header::SET_COOKIE,
        header::CONTENT_SECURITY_POLICY,
        header::X_FRAME_OPTIONS,
    ] {
        parts.headers.remove(name);
    }
    let location = parts
        .headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| located(value, slug, prefix))
        .and_then(|value| HeaderValue::from_str(&value).ok());
    if let Some(location) = location {
        parts.headers.insert(header::LOCATION, location);
    }
    guarded(&mut parts.headers);
    Response::from_parts(parts, Body::new(body))
}

fn guarded(headers: &mut HeaderMap) {
    for (name, value) in [
        (header::CONTENT_SECURITY_POLICY, POLICY),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
}

fn preflight(request: &HeaderMap) -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    let headers = response.headers_mut();
    guarded(headers);
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE"),
    );
    if let Some(asked) = request.get(header::ACCESS_CONTROL_REQUEST_HEADERS) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, asked.clone());
    }
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("600"),
    );
    response
}

fn unavailable() -> Response {
    let mut response = (StatusCode::BAD_GATEWAY, UNAVAILABLE).into_response();
    let headers = response.headers_mut();
    guarded(headers);
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::WebSocketUpgrade, routing::get};
    use futures_util::{SinkExt, StreamExt};
    use http_body_util::BodyExt;
    use tokio_tungstenite::tungstenite::Message as Wire;

    const AGENT: AgentId = AgentId(Uuid::nil());
    const PREFIX: &str = "/api/apps/budget/v/token";
    type Answer = fn(&[u8]) -> Option<Vec<u8>>;

    fn view() -> View {
        View {
            viewer: Uuid::from_u128(7),
            name: "Ana Lúcia".into(),
            app: Uuid::from_u128(9),
            agent: AGENT,
            slug: "budget".into(),
            expires: Instant::now() + LIFETIME,
        }
    }

    async fn linked(answer: Answer) -> (Links, mpsc::Receiver<AppReport>) {
        let links = Links::default();
        let (reports, received) = mpsc::channel(8);
        let serving = links.clone();
        let router = axum::Router::new().route(
            "/",
            get(move |upgrade: WebSocketUpgrade| async move {
                upgrade.on_upgrade(move |socket| async move {
                    serving.serve(AGENT, socket, reports).await;
                })
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/"))
            .await
            .unwrap();
        tokio::spawn(agent(socket, answer));
        while !links.connected(AGENT) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        (links, received)
    }

    async fn agent<S>(mut socket: tokio_tungstenite::WebSocketStream<S>, answer: Answer)
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let report = r#"{"type":"state","slug":"budget","state":"running","memory":null}"#;
        socket.send(Wire::Text(report.into())).await.unwrap();
        let oversized = format!(
            r#"{{"type":"state","slug":"{}","state":"running","memory":null}}"#,
            "a".repeat(TEXT_BYTES)
        );
        socket.send(Wire::Text(oversized.into())).await.unwrap();
        let mut heads: HashMap<u32, Vec<u8>> = HashMap::new();
        while let Some(Ok(message)) = socket.next().await {
            let Wire::Binary(bytes) = message else {
                continue;
            };
            let Some(Frame::Data(stream, data)) = decode(&bytes) else {
                continue;
            };
            let head = heads.entry(stream).or_default();
            head.extend_from_slice(data);
            if !head.windows(4).any(|window| window == b"\r\n\r\n") {
                continue;
            }
            let Some(response) = answer(head) else { return };
            for chunk in response.chunks(CHUNK) {
                socket
                    .send(Wire::Binary(encode(&Frame::Data(stream, chunk)).into()))
                    .await
                    .unwrap();
            }
            socket
                .send(Wire::Binary(encode(&Frame::Close(stream)).into()))
                .await
                .unwrap();
        }
    }

    fn big(_: &[u8]) -> Option<Vec<u8>> {
        let body = "a".repeat(200 * 1024);
        Some(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            )
            .into_bytes(),
        )
    }

    fn echo(head: &[u8]) -> Option<Vec<u8>> {
        let mut response = format!(
            "HTTP/1.1 302 Found\r\ncontent-length: {}\r\nset-cookie: s=1\r\ncontent-security-policy: default-src *\r\nx-frame-options: DENY\r\nlocation: /budget/next?x=1\r\n\r\n",
            head.len()
        )
        .into_bytes();
        response.extend_from_slice(head);
        Some(response)
    }

    fn vanish(_: &[u8]) -> Option<Vec<u8>> {
        None
    }

    fn get_page() -> Request {
        Request::builder()
            .uri(format!("{PREFIX}/page?q=1"))
            .header("cookie", "__Host-cubebot_session=secret")
            .header("authorization", "Bearer secret")
            .header(VIEWER_ID, "forged")
            .header(VIEWER_NAME, "Mallory")
            .header("x-cubebot-anything", "forged")
            .header("x-custom", "kept")
            .body(Body::empty())
            .unwrap()
    }

    async fn text(response: Response) -> String {
        String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn a_request_round_trips_through_the_tunnel() {
        let (links, mut reports) = linked(big).await;
        let response = forward(&links, &view(), PREFIX, "/page", get_page()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(text(response).await, "a".repeat(200 * 1024));
        let second = forward(&links, &view(), PREFIX, "/page", get_page()).await;
        assert_eq!(text(second).await.len(), 200 * 1024);
        let report = reports.recv().await.unwrap();
        assert!(matches!(report, AppReport::State { ref slug, .. } if slug == "budget"));
        assert!(reports.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_dropped_link_ends_open_streams_with_502() {
        let response = forward(&Links::default(), &view(), PREFIX, "/page", get_page()).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let (links, _reports) = linked(vanish).await;
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            forward(&links, &view(), PREFIX, "/page", get_page()),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(response.headers()[header::CONTENT_SECURITY_POLICY], POLICY);
        assert!(
            text(response)
                .await
                .contains("This app is unavailable right now")
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while links.connected(AGENT) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn security_headers_replace_the_apps_own() {
        let (links, _reports) = linked(echo).await;
        let response = forward(&links, &view(), PREFIX, "/page", get_page()).await;
        let headers = response.headers();
        assert_eq!(headers[header::CONTENT_SECURITY_POLICY], POLICY);
        assert_eq!(
            headers
                .get_all(header::CONTENT_SECURITY_POLICY)
                .iter()
                .count(),
            1
        );
        assert_eq!(headers[header::REFERRER_POLICY], "no-referrer");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert_eq!(headers[header::LOCATION], format!("{PREFIX}/next?x=1"));
        assert!(!headers.contains_key(header::SET_COOKIE));
        assert!(!headers.contains_key(header::X_FRAME_OPTIONS));
        assert!(!headers.contains_key("cross-origin-resource-policy"));
    }

    #[tokio::test]
    async fn viewer_headers_from_the_browser_are_dropped() {
        let (links, _reports) = linked(echo).await;
        let response = forward(&links, &view(), PREFIX, "/page", get_page()).await;
        let head = text(response).await.to_lowercase();
        assert!(
            head.starts_with("get /budget/page?q=1 http/1.1\r\n"),
            "{head}"
        );
        assert!(head.contains("\r\nx-custom: kept\r\n"));
        assert!(head.contains("\r\nhost: localhost\r\n"));
        assert!(head.contains(&format!("\r\n{VIEWER_ID}: {}\r\n", Uuid::from_u128(7))));
        assert!(head.contains(&format!("\r\n{VIEWER_NAME}: ana l%c3%bacia\r\n")));
        for leaked in [
            "secret",
            "forged",
            "mallory",
            "cookie",
            "authorization",
            "x-cubebot-anything",
        ] {
            assert!(!head.contains(leaked), "{leaked} reached the app: {head}");
        }
    }

    #[test]
    fn preflights_echo_the_asked_headers() {
        let mut asked = HeaderMap::new();
        asked.insert(
            header::ACCESS_CONTROL_REQUEST_HEADERS,
            HeaderValue::from_static("content-type, x-thing"),
        );
        let response = preflight(&asked);
        let headers = response.headers();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert_eq!(
            headers[header::ACCESS_CONTROL_ALLOW_METHODS],
            "GET, POST, PUT, PATCH, DELETE"
        );
        assert_eq!(
            headers[header::ACCESS_CONTROL_ALLOW_HEADERS],
            "content-type, x-thing"
        );
        assert_eq!(headers[header::ACCESS_CONTROL_MAX_AGE], "600");
    }

    #[test]
    fn redirects_into_the_app_stay_inside_the_relay() {
        assert_eq!(
            located("/budget", "budget", PREFIX).unwrap(),
            format!("{PREFIX}/")
        );
        assert_eq!(
            located("/budget?q=1", "budget", PREFIX).unwrap(),
            format!("{PREFIX}/?q=1")
        );
        assert_eq!(
            located("/budget/a/b", "budget", PREFIX).unwrap(),
            format!("{PREFIX}/a/b")
        );
        for elsewhere in [
            "/budgets/a",
            "/other/",
            "https://evil.test/budget/",
            "budget/a",
            "//budget/",
        ] {
            assert_eq!(located(elsewhere, "budget", PREFIX), None, "{elsewhere}");
        }
        let parsed = target("/api/apps/budget/v/token/a/b").unwrap();
        assert_eq!(
            (parsed.slug, parsed.token, parsed.rest),
            ("budget", "token", Some("/a/b"))
        );
        assert_eq!(target("/api/apps/budget/v/token").unwrap().rest, None);
        assert!(target("/api/apps/budget/state").is_none());
    }
}

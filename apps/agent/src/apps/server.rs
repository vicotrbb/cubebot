use super::{Apps, report};
use cubebot_protocol::apps::{AppReport, AppSpec, Desired, LOCAL_PORT, VIEWER_ID};
use http_body_util::{BodyExt, Full, Limited, combinators::BoxBody};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Bytes, Incoming},
    header::{CACHE_CONTROL, CONTENT_TYPE, UPGRADE},
    server::conn::http1,
    service::service_fn,
    upgrade::OnUpgrade,
};
use hyper_util::rt::TokioIo;
use std::{
    convert::Infallible,
    io::Read,
    net::Ipv4Addr,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

type Body = BoxBody<Bytes, hyper::Error>;
type Failure = Box<dyn std::error::Error + Send + Sync>;

const KIT_CSS: &str = include_str!("../../../../modules/kit/kit.css");
const KIT_JS: &str = include_str!("../../../../modules/kit/src/kit.js");
const KIT_CACHE: &str = "public, max-age=86400";
const CALL_LIMIT: usize = 64 * 1024;
const FILE_LIMIT: u64 = 64 * 1024 * 1024;

pub async fn serve(apps: Arc<Apps>) {
    match TcpListener::bind((Ipv4Addr::LOCALHOST, LOCAL_PORT)).await {
        Ok(listener) => listen(listener, apps).await,
        Err(error) => eprintln!("Apps not served: {error}"),
    }
}

async fn listen(listener: TcpListener, apps: Arc<Apps>) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                eprintln!("App connection not accepted: {error}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let apps = apps.clone();
        tokio::spawn(async move {
            let service = service_fn(move |request| answer(apps.clone(), request));
            let connection = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades();
            let _ = connection.await;
        });
    }
}

async fn answer(apps: Arc<Apps>, request: Request<Incoming>) -> Result<Response<Body>, Infallible> {
    let path = request.uri().path().to_owned();
    let (slug, rest) = path
        .trim_start_matches('/')
        .split_once('/')
        .unwrap_or((path.trim_start_matches('/'), ""));
    let Some(spec) = apps.spec(slug) else {
        return Ok(plain(StatusCode::NOT_FOUND, "not found"));
    };
    if spec.desired == Desired::Stopped {
        return Ok(plain(StatusCode::SERVICE_UNAVAILABLE, "stopped"));
    }
    Ok(match rest {
        "_cubebot/kit.css" => kit(KIT_CSS, "text/css; charset=utf-8"),
        "_cubebot/kit.js" => kit(KIT_JS, "text/javascript; charset=utf-8"),
        "_cubebot/call" => call(&apps, &spec, request).await,
        _ => match (&spec.command, spec.port) {
            (Some(_), Some(port)) => proxy(port, rest, request).await,
            _ => file(&apps, &spec, rest).await,
        },
    })
}

fn full(status: StatusCode, content_type: &str, body: impl Into<Bytes>) -> Response<Body> {
    let mut response = Response::new(
        Full::new(body.into())
            .map_err(|never| match never {})
            .boxed(),
    );
    *response.status_mut() = status;
    if let Ok(value) = content_type.parse() {
        response.headers_mut().insert(CONTENT_TYPE, value);
    }
    response
}

fn plain(status: StatusCode, text: &'static str) -> Response<Body> {
    full(status, "text/plain; charset=utf-8", text)
}

fn json(status: StatusCode, value: serde_json::Value) -> Response<Body> {
    full(status, "application/json", value.to_string())
}

fn kit(text: &'static str, content_type: &str) -> Response<Body> {
    let mut response = full(StatusCode::OK, content_type, text);
    response.headers_mut().insert(
        CACHE_CONTROL,
        hyper::header::HeaderValue::from_static(KIT_CACHE),
    );
    response
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    tool: String,
    #[serde(default)]
    input: serde_json::Map<String, serde_json::Value>,
}

fn refusal(status: StatusCode, error: &str) -> Response<Body> {
    json(status, serde_json::json!({ "ok": false, "error": error }))
}

async fn call(apps: &Apps, spec: &AppSpec, request: Request<Incoming>) -> Response<Body> {
    if request.method() != Method::POST {
        return refusal(StatusCode::METHOD_NOT_ALLOWED, "calls are POST requests");
    }
    let viewer = request
        .headers()
        .get(VIEWER_ID)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok());
    let Ok(body) = Limited::new(request.into_body(), CALL_LIMIT)
        .collect()
        .await
    else {
        return refusal(StatusCode::BAD_REQUEST, "the call must be at most 64 KiB");
    };
    let Ok(call) = serde_json::from_slice::<Call>(&body.to_bytes()) else {
        return refusal(StatusCode::BAD_REQUEST, "the call must be { tool, input }");
    };
    if !spec.tools.contains(&call.tool) {
        return refusal(StatusCode::FORBIDDEN, "this app does not declare that tool");
    }
    let input = serde_json::Value::Object(call.input);
    let result = apps.connections.call_tool(&call.tool, &input).await;
    report(
        &apps.reports,
        AppReport::Called {
            slug: spec.slug.clone(),
            tool: call.tool,
            viewer,
            ok: result.is_ok(),
        },
    );
    match result {
        Ok(text) => json(
            StatusCode::OK,
            serde_json::json!({ "ok": true, "text": text }),
        ),
        Err(error) => refusal(StatusCode::OK, &error),
    }
}

async fn proxy(port: u16, rest: &str, request: Request<Incoming>) -> Response<Body> {
    match forward(port, rest, request).await {
        Ok(response) => response,
        Err(error) => {
            tracing::debug!(%error, "app did not answer");
            plain(StatusCode::BAD_GATEWAY, "unavailable")
        }
    }
}

async fn forward(
    port: u16,
    rest: &str,
    mut request: Request<Incoming>,
) -> Result<Response<Body>, Failure> {
    let target = match request.uri().query() {
        Some(query) => format!("/{rest}?{query}"),
        None => format!("/{rest}"),
    };
    *request.uri_mut() = target.parse()?;
    let upgrade = request
        .headers()
        .contains_key(UPGRADE)
        .then(|| hyper::upgrade::on(&mut request));
    let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await?;
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(connection.with_upgrades());
    let mut response = sender.send_request(request).await?;
    if response.status() == StatusCode::SWITCHING_PROTOCOLS
        && let Some(client) = upgrade
    {
        tokio::spawn(splice(client, hyper::upgrade::on(&mut response)));
    }
    Ok(response.map(BodyExt::boxed))
}

async fn splice(client: OnUpgrade, app: OnUpgrade) {
    if let (Ok(client), Ok(app)) = tokio::join!(client, app) {
        let _ =
            tokio::io::copy_bidirectional(&mut TokioIo::new(client), &mut TokioIo::new(app)).await;
    }
}

async fn file(apps: &Apps, spec: &AppSpec, rest: &str) -> Response<Body> {
    let Some(sandbox) = apps.sandbox.clone() else {
        return plain(StatusCode::NOT_FOUND, "not found");
    };
    let folder = spec.folder.clone();
    let rest = rest.to_owned();
    let read = tokio::task::spawn_blocking(move || {
        read(sandbox.workspace(), &folder, &rest, sandbox.uid())
    });
    match read.await.ok().flatten() {
        Some((bytes, kind)) => full(StatusCode::OK, kind, bytes),
        None => plain(StatusCode::NOT_FOUND, "not found"),
    }
}

fn read(workspace: &Path, folder: &str, rest: &str, owner: u32) -> Option<(Vec<u8>, &'static str)> {
    let workspace = workspace.canonicalize().ok()?;
    let root = workspace.join(folder).canonicalize().ok()?;
    let decoded = percent_encoding::percent_decode_str(rest)
        .decode_utf8()
        .ok()?;
    let mut path = root.join(&*decoded).canonicalize().ok()?;
    if !root.starts_with(&workspace) || !path.starts_with(&root) {
        return None;
    }
    if path.is_dir() {
        path.push("index.html");
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(&path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.uid() != owner || metadata.len() > FILE_LIMIT {
        return None;
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some((bytes, kind(&path)))
}

fn kind(path: &Path) -> &'static str {
    let extension = path.extension().and_then(|extension| extension.to_str());
    match extension.map(str::to_ascii_lowercase).as_deref() {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Sandbox;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn spec(slug: &str, desired: Desired, tools: &[&str]) -> AppSpec {
        AppSpec {
            slug: slug.into(),
            name: slug.into(),
            folder: "site".into(),
            command: None,
            port: None,
            tools: tools.iter().map(|tool| tool.to_string()).collect(),
            desired,
        }
    }

    async fn serving(specs: Vec<AppSpec>) -> (Arc<Apps>, Sandbox, std::net::SocketAddr) {
        let sandbox = crate::tools::tests::sandbox();
        let site = sandbox.workspace().join("site");
        std::fs::create_dir(&site).unwrap();
        std::fs::write(site.join("index.html"), "<p>home</p>").unwrap();
        std::fs::write(site.join("a.txt"), "inside").unwrap();
        std::fs::write(sandbox.workspace().join("secret.txt"), "outside").unwrap();
        std::os::unix::fs::symlink("/etc", site.join("etc")).unwrap();
        let apps = Arc::new(Apps::new(None, Some(sandbox.clone())));
        *apps.specs.write().unwrap() = specs;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(listen(listener, apps.clone()));
        (apps, sandbox, address)
    }

    async fn send(address: std::net::SocketAddr, head: &str, body: &str) -> (u16, String) {
        exchange(address, head, "", body).await
    }

    async fn exchange(
        address: std::net::SocketAddr,
        head: &str,
        headers: &str,
        body: &str,
    ) -> (u16, String) {
        let mut stream = TcpStream::connect(address).await.unwrap();
        let request = format!(
            "{head} HTTP/1.1\r\nhost: app\r\n{headers}connection: close\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        let status = response[9..12].parse().unwrap();
        let body = response.split_once("\r\n\r\n").unwrap().1.to_owned();
        (status, body)
    }

    #[tokio::test]
    async fn static_paths_never_escape_the_folder() {
        let (_apps, sandbox, address) = serving(vec![spec("app", Desired::Running, &[])]).await;
        assert_eq!(
            send(address, "GET /app/", "").await,
            (200, "<p>home</p>".into())
        );
        assert_eq!(
            send(address, "GET /app/a.txt", "").await,
            (200, "inside".into())
        );
        for escape in [
            "/app/../secret.txt",
            "/app/%2e%2e/secret.txt",
            "/app/%2E%2E%2Fsecret.txt",
            "/app//etc/passwd",
            "/app/%2Fetc%2Fpasswd",
            "/app/etc/passwd",
            "/app/etc/",
        ] {
            let (status, body) = send(address, &format!("GET {escape}"), "").await;
            assert_eq!(status, 404, "{escape}");
            assert!(
                !body.contains("outside") && !body.contains("root"),
                "{escape}"
            );
        }
        std::fs::remove_dir_all(sandbox.workspace()).unwrap();
    }

    #[tokio::test]
    async fn undeclared_tools_are_refused() {
        let (apps, sandbox, address) = serving(vec![spec(
            "app",
            Desired::Running,
            &["github__list_issues"],
        )])
        .await;
        let mut reports = apps.reports().unwrap();
        let call = r#"{"tool":"github__delete_repository","input":{}}"#;
        let (status, body) = send(address, "POST /app/_cubebot/call", call).await;
        assert_eq!(status, 403);
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["ok"], false);
        assert!(reports.try_recv().is_err());

        let viewer = Uuid::new_v4();
        let headers = format!("{VIEWER_ID}: {viewer}\r\n");
        let call = r#"{"tool":"github__list_issues","input":{}}"#;
        let (status, body) = exchange(address, "POST /app/_cubebot/call", &headers, call).await;
        assert_eq!(status, 200);
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"], "no orchestrator to ask");
        let called = reports.try_recv().unwrap();
        assert_eq!(
            called,
            AppReport::Called {
                slug: "app".into(),
                tool: "github__list_issues".into(),
                viewer: Some(viewer),
                ok: false,
            }
        );
        std::fs::remove_dir_all(sandbox.workspace()).unwrap();
    }

    #[tokio::test]
    async fn stopped_apps_answer_503() {
        let (_apps, sandbox, address) = serving(vec![spec("app", Desired::Stopped, &[])]).await;
        assert_eq!(
            send(address, "GET /app/", "").await,
            (503, "stopped".into())
        );
        assert_eq!(send(address, "GET /app/_cubebot/kit.js", "").await.0, 503);
        assert_eq!(send(address, "GET /other/", "").await.0, 404);
        std::fs::remove_dir_all(sandbox.workspace()).unwrap();
    }

    #[tokio::test]
    async fn command_apps_are_proxied_with_upgrades() {
        use axum::{
            Router,
            extract::{WebSocketUpgrade, ws::Message},
            routing::get,
        };
        use futures_util::{SinkExt, StreamExt};
        let echo = |upgrade: WebSocketUpgrade| async move {
            upgrade.on_upgrade(|mut socket| async move {
                while let Some(Ok(Message::Text(text))) = socket.recv().await {
                    let _ = socket.send(Message::Text(text)).await;
                }
            })
        };
        let seen = |uri: hyper::Uri| async move { uri.to_string() };
        let upstream = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = upstream.local_addr().unwrap().port();
        let router = Router::new()
            .route("/socket", get(echo))
            .route("/page", get(seen));
        tokio::spawn(async move { axum::serve(upstream, router).await });
        let mut app = spec("app", Desired::Running, &[]);
        app.command = Some("true".into());
        app.port = Some(port);
        let (_apps, sandbox, address) = serving(vec![app]).await;

        assert_eq!(
            send(address, "GET /app/page?q=1", "").await,
            (200, "/page?q=1".into())
        );
        let url = format!("ws://{address}/app/socket");
        let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text("hi".into()))
            .await
            .unwrap();
        let echoed = socket.next().await.unwrap().unwrap();
        assert_eq!(echoed.into_text().unwrap().as_str(), "hi");
        std::fs::remove_dir_all(sandbox.workspace()).unwrap();
    }
}

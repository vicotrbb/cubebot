//! Remote servers over Streamable HTTP: each message is a POST answered by JSON or by an SSE
//! stream carrying the response.
use super::{Failure, answer, result};
use reqwest::{
    Client, Response, Url,
    header::{ACCEPT, HeaderMap, HeaderName, HeaderValue},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
    time::Duration,
};

/// Remote output is held in memory before it is retained, so it is bounded here.
const RESPONSE_BYTES: usize = 16 * 1024 * 1024;

pub(in crate::tools) struct Http {
    client: Client,
    url: Url,
    headers: HeaderMap,
    session: Mutex<Option<HeaderValue>>,
    version: OnceLock<HeaderValue>,
}

impl Http {
    pub fn new(url: &str, headers: &BTreeMap<String, String>) -> Result<Self, String> {
        let url = Url::parse(url).map_err(|error| format!("invalid URL: {error}"))?;
        if !matches!(url.scheme(), "https" | "http") {
            return Err("URL must use https or http".into());
        }
        let headers = headers
            .iter()
            .map(|(name, value)| {
                Ok((
                    HeaderName::try_from(name.as_str()).map_err(|_| "invalid header name")?,
                    HeaderValue::try_from(value.as_str()).map_err(|_| "invalid header value")?,
                ))
            })
            .collect::<Result<HeaderMap, &str>>()?;
        let client = Client::builder()
            // Configured credentials go to the configured endpoint only.
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            client,
            url,
            headers,
            session: Mutex::new(None),
            version: OnceLock::new(),
        })
    }

    pub fn negotiated(&self, version: &str) {
        if let Ok(version) = HeaderValue::try_from(version) {
            let _ = self.version.set(version);
        }
    }

    #[tracing::instrument(skip_all, fields(method = method, id = id))]
    pub async fn request(&self, id: u64, method: &str, params: Value) -> Result<Value, Failure> {
        let response = self
            .post(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        let stream = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"));
        if stream {
            return self.stream(response, id).await;
        }
        let body = body(response).await?;
        result(serde_json::from_slice(&body).map_err(|_| Failure::Lost)?)
    }

    /// Reads SSE events as they arrive, answering server requests such as ping, until the
    /// response to `id`. A stream that ends first lost the response.
    async fn stream(&self, mut response: Response, id: u64) -> Result<Value, Failure> {
        let mut buffer = Vec::new();
        let mut scanned = 0;
        while let Some(chunk) = response.chunk().await.map_err(|_| Failure::Lost)? {
            // JSON never holds a raw carriage return, so CRLF events become LF events.
            buffer.extend(chunk.iter().filter(|&&byte| byte != b'\r'));
            if buffer.len() > RESPONSE_BYTES {
                return Err(Failure::Lost);
            }
            while let Some(end) = boundary(&buffer, scanned) {
                let message = event(&buffer.drain(..end + 2).collect::<Vec<u8>>());
                scanned = 0;
                match message {
                    Some(message) if message.get("method").is_some() => {
                        if !message["id"].is_null() {
                            let _ = self.post(&answer(&message)).await;
                        }
                    }
                    Some(message) if message["id"] == id => return result(message),
                    _ => (),
                }
            }
            scanned = buffer.len().saturating_sub(1);
        }
        Err(Failure::Lost)
    }

    #[tracing::instrument(skip_all, fields(method = message["method"].as_str()))]
    pub async fn notify(&self, message: &Value) -> Result<(), Failure> {
        self.post(message).await.map(drop)
    }

    async fn post(&self, message: &Value) -> Result<Response, Failure> {
        let mut request = self
            .client
            .post(self.url.clone())
            .headers(self.headers.clone())
            .header(ACCEPT, "application/json, text/event-stream")
            .json(message);
        let session = self.session.lock().ok().and_then(|session| session.clone());
        if let Some(session) = &session {
            request = request.header("mcp-session-id", session);
        }
        if let Some(version) = self.version.get() {
            request = request.header("mcp-protocol-version", version);
        }
        // Only a failed connection proves the request never arrived.
        let response = request
            .send()
            .await
            .map_err(|error| match error.is_connect() {
                true => Failure::NotSent("connection failed".into()),
                false => Failure::Lost,
            })?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND && session.is_some() {
            return Err(Failure::Expired);
        }
        if status.is_client_error() {
            return Err(Failure::Rejected(format!("HTTP {}", status.as_u16())));
        }
        if !status.is_success() {
            return Err(Failure::Lost);
        }
        if let Some(session) = response.headers().get("mcp-session-id")
            && let Ok(mut current) = self.session.lock()
        {
            *current = Some(session.clone());
        }
        Ok(response)
    }
}

/// A JSON response body, bounded.
async fn body(mut response: Response) -> Result<Vec<u8>, Failure> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Failure::Lost)? {
        if bytes.len() + chunk.len() > RESPONSE_BYTES {
            return Err(Failure::Lost);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// The end of the first complete event, searching from where the last search stopped.
fn boundary(buffer: &[u8], from: usize) -> Option<usize> {
    buffer[from..]
        .windows(2)
        .position(|pair| pair == b"\n\n")
        .map(|position| from + position)
}

/// The JSON message carried in one event's `data` lines.
fn event(bytes: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(bytes);
    let data: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|data| data.strip_prefix(' ').unwrap_or(data))
        .collect();
    serde_json::from_str(&data.join("\n")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router, body::Body, http::HeaderMap, http::StatusCode, response::IntoResponse,
        routing::post,
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    static SESSIONS: AtomicUsize = AtomicUsize::new(0);
    static PONGED: AtomicBool = AtomicBool::new(false);

    /// Expires the first session on its first call, pings mid-stream and never closes a stream.
    async fn server(headers: HeaderMap, Json(message): Json<Value>) -> axum::response::Response {
        let id = message["id"].clone();
        let session = headers
            .get("mcp-session-id")
            .map(|value| value.to_str().unwrap().to_owned());
        let Some(method) = message["method"].as_str() else {
            assert_eq!(message, json!({"jsonrpc":"2.0","id":"ping-1","result":{}}));
            PONGED.store(true, Ordering::SeqCst);
            return StatusCode::ACCEPTED.into_response();
        };
        match method {
            "initialize" => {
                assert_eq!(headers["authorization"], "Bearer secret");
                assert!(session.is_none(), "a new session starts without the old id");
                let session = format!("s-{}", SESSIONS.fetch_add(1, Ordering::SeqCst) + 1);
                let body =
                    json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2025-11-25"}});
                ([("mcp-session-id", session)], Json(body)).into_response()
            }
            "notifications/initialized" => StatusCode::ACCEPTED.into_response(),
            "tools/call" if session.as_deref() == Some("s-1") => {
                StatusCode::NOT_FOUND.into_response()
            }
            method => {
                assert!(session.is_some());
                assert_eq!(headers["mcp-protocol-version"], "2025-11-25");
                let result = match method {
                    "tools/list" => {
                        json!({"tools":[{"name":"lookup","inputSchema":{"type":"object"}}]})
                    }
                    _ => {
                        json!({"isError":true,"content":[{"type":"text","text":format!("no {}", message["params"]["arguments"]["q"])}]})
                    }
                };
                let events = format!(
                    ": comment\r\nevent: message\r\ndata: {{\"jsonrpc\":\"2.0\",\"method\":\"ping\",\"id\":\"ping-1\"}}\r\n\r\ndata: {}\n\n",
                    json!({"jsonrpc":"2.0","id":id,"result":result})
                );
                let open = futures_util::StreamExt::chain(
                    futures_util::stream::iter([Ok::<_, std::io::Error>(events)]),
                    futures_util::stream::pending(),
                );
                (
                    [("content-type", "text/event-stream")],
                    Body::from_stream(open),
                )
                    .into_response()
            }
        }
    }

    #[tokio::test]
    async fn streamable_http_renews_an_expired_session_and_answers_pings_mid_stream() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/mcp", post(server)))
                .await
                .unwrap()
        });
        let sandbox = crate::tools::tests::sandbox();
        let config = super::super::Config::Http {
            name: "remote".into(),
            url,
            headers: BTreeMap::from([("authorization".into(), "Bearer secret".into())]),
        };
        let tools = super::super::connect(&sandbox, config).await.unwrap();
        assert_eq!(tools[0].definition().name, "remote__lookup");
        let call = tools[0]
            .call(&sandbox, uuid::Uuid::new_v4(), &json!({"q":"x"}))
            .unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(5), call)
            .await
            .unwrap();
        let cubebot_protocol::terminal::Outcome::Exited { code, output } = outcome else {
            panic!("call did not finish");
        };
        assert_eq!(code, Some(1));
        assert!(output.starts_with("no \"x\""), "{output}");
        assert_eq!(SESSIONS.load(Ordering::SeqCst), 2);
        assert!(PONGED.load(Ordering::SeqCst));
        task.abort();
        std::fs::remove_dir_all(&sandbox.workspace).unwrap();
    }
}

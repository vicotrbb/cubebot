//! Loopback servers that replay recorded vendor traffic: HTTP exchanges, and line scripts for
//! IMAP and SMTP. Each records what the connector sent, so tests can check requests too.
// Connectors use different parts of it.
#![allow(dead_code)]
use serde::Deserialize;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};

/// One recorded request and the answer to it. A path with `?` must match the query too.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exchange {
    pub method: String,
    pub path: String,
    #[serde(default = "ok")]
    pub status: u16,
    #[serde(default)]
    pub body: Value,
    /// Sent as is instead of `body`, such as XML or an empty answer.
    #[serde(default)]
    pub raw: Option<String>,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
}

fn ok() -> u16 {
    200
}

impl Exchange {
    pub fn new(method: &str, path: &str, status: u16, body: Value) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            status,
            body,
            raw: None,
            headers: Vec::new(),
        }
    }

    pub fn raw(method: &str, path: &str, status: u16, body: &str) -> Self {
        Self {
            raw: Some(body.into()),
            ..Self::new(method, path, status, Value::Null)
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    fn matches(&self, method: &str, target: &str) -> bool {
        let target = match self.path.contains('?') {
            true => target,
            false => target.split('?').next().unwrap_or_default(),
        };
        self.method == method && self.path == target
    }

    fn answer(&self) -> Vec<u8> {
        let body = match &self.raw {
            Some(raw) => raw.clone().into_bytes(),
            None if self.body.is_null() => Vec::new(),
            None => self.body.to_string().into_bytes(),
        };
        let mut head = format!(
            "HTTP/1.1 {} Fixture\r\nconnection: close\r\ncontent-length: {}\r\n",
            self.status,
            body.len()
        );
        if self.raw.is_none() && !body.is_empty() {
            head.push_str("content-type: application/json\r\n");
        }
        for (name, value) in &self.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        [head.into_bytes(), body].concat()
    }
}

/// A request the fixture received.
#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    /// The path and query, as sent.
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

#[derive(Default)]
struct Log {
    waiting: Vec<Option<Exchange>>,
    seen: Vec<Seen>,
}

pub struct Fixture {
    origin: String,
    log: Arc<Mutex<Log>>,
    task: tokio::task::JoinHandle<()>,
}

impl Fixture {
    /// Answers each request with the first unused exchange of the same method and path; anything
    /// else gets a 599 and fails [`Fixture::finish`].
    pub async fn start(exchanges: Vec<Exchange>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Log {
            waiting: exchanges.into_iter().map(Some).collect(),
            seen: Vec::new(),
        }));
        let shared = log.clone();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream, shared.clone()));
            }
        });
        Self { origin, log, task }
    }

    /// Accepts and reads requests but never answers.
    pub async fn silent() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });
        Self {
            origin,
            log: Arc::default(),
            task,
        }
    }

    /// With a trailing slash.
    pub fn origin(&self) -> String {
        self.origin.clone()
    }

    /// Every request received, after checking each exchange was used exactly once.
    pub async fn finish(self) -> Vec<Seen> {
        self.task.abort();
        let log = self.log.lock().unwrap();
        let unused: Vec<String> = log
            .waiting
            .iter()
            .flatten()
            .map(|exchange| format!("{} {}", exchange.method, exchange.path))
            .collect();
        assert!(unused.is_empty(), "exchanges never requested: {unused:?}");
        log.seen.clone()
    }
}

async fn serve(stream: TcpStream, log: Arc<Mutex<Log>>) {
    let mut reader = BufReader::new(stream);
    let Some(seen) = request(&mut reader).await else {
        return;
    };
    let answer = {
        let mut log = log.lock().unwrap();
        let found = log.waiting.iter_mut().find(|slot| {
            slot.as_ref()
                .is_some_and(|exchange| exchange.matches(&seen.method, &seen.target))
        });
        let answer = match found.and_then(Option::take) {
            Some(exchange) => exchange.answer(),
            None => {
                let missing = format!("{} {}", seen.method, seen.target);
                log.waiting.push(Some(Exchange::new(
                    "UNEXPECTED",
                    &missing,
                    599,
                    Value::Null,
                )));
                Exchange::new("", "", 599, Value::Null).answer()
            }
        };
        log.seen.push(seen);
        answer
    };
    let _ = reader.get_mut().write_all(&answer).await;
    let _ = reader.get_mut().shutdown().await;
}

async fn request(reader: &mut BufReader<TcpStream>) -> Option<Seen> {
    let mut line = String::new();
    reader.read_line(&mut line).await.ok()?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next()?.to_owned(), parts.next()?.to_owned());
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).await.ok()?;
        let Some((name, value)) = header.trim_end().split_once(':') else {
            break;
        };
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await.ok()?;
    Some(Seen {
        method,
        target,
        headers,
        body,
    })
}

/// A line-based conversation such as IMAP or SMTP. `S: text` is sent with CRLF; `C: text` must be
/// the client's next line, where a trailing `*` matches any rest; `C~ n` takes `n` raw bytes,
/// such as an IMAP literal.
pub struct Script {
    port: u16,
    task: tokio::task::JoinHandle<Result<(), String>>,
}

impl Script {
    pub async fn start(lines: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.map_err(|error| error.to_string())?;
            converse(BufReader::new(stream), lines).await
        });
        Self { port, task }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Fails unless the whole conversation happened as scripted.
    pub async fn finish(self) {
        let ended = tokio::time::timeout(std::time::Duration::from_secs(5), self.task).await;
        match ended {
            Ok(Ok(Ok(()))) => (),
            Ok(Ok(Err(error))) => panic!("script: {error}"),
            other => panic!("script did not finish: {other:?}"),
        }
    }
}

async fn converse(mut stream: BufReader<TcpStream>, lines: Vec<String>) -> Result<(), String> {
    for (number, line) in lines.iter().enumerate() {
        if let Some(text) = line.strip_prefix("S: ") {
            let sent = format!("{text}\r\n");
            let written = stream.get_mut().write_all(sent.as_bytes()).await;
            written.map_err(|error| error.to_string())?;
        } else if let Some(count) = line.strip_prefix("C~ ") {
            let mut raw = vec![0; count.trim().parse().map_err(|_| "a byte count")?];
            stream
                .read_exact(&mut raw)
                .await
                .map_err(|error| error.to_string())?;
        } else if let Some(expected) = line.strip_prefix("C: ") {
            let mut got = String::new();
            stream
                .read_line(&mut got)
                .await
                .map_err(|error| error.to_string())?;
            expect(number, expected, got.trim_end_matches(['\r', '\n']))?;
        } else {
            return Err(format!("line {number}: not a script line: {line}"));
        }
    }
    Ok(())
}

fn expect(number: usize, expected: &str, got: &str) -> Result<(), String> {
    let matched = match expected.strip_suffix('*') {
        Some(prefix) => got.starts_with(prefix),
        None => got == expected,
    };
    matched
        .then_some(())
        .ok_or_else(|| format!("line {number}: expected {expected:?}, got {got:?}"))
}

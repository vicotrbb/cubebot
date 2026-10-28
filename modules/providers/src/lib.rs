//! Streaming inference adapters. Callers own context, retries and tool policy; adapters
//! translate requests, streams, usage, errors and capability differences. No provider-side
//! history and no automatic retries.
use futures_util::Stream;
use reqwest::{Client, RequestBuilder, Response, Url};
use serde_json::Value;
use std::{collections::VecDeque, time::Duration};

mod anthropic;
mod openai;
mod sse;
pub use anthropic::Anthropic;
pub use openai::OpenAi;

#[derive(Debug)]
pub enum Error {
    Configuration,
    Unavailable,
    /// HTTP 429 or 5xx, with the delay the provider asked for when it sent one.
    Transient(Option<Duration>),
    Rejected,
    InvalidResponse,
}
impl Error {
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Transient(_))
    }
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Transient(delay) => *delay,
            _ => None,
        }
    }
    pub fn code(&self) -> &'static str {
        match self {
            Self::Configuration | Self::Unavailable | Self::Transient(_) => "provider_unavailable",
            Self::Rejected => "provider_rejected",
            Self::InvalidResponse => "provider_invalid_response",
        }
    }
}

/// A function tool. `parameters` is a strict JSON schema.
#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FunctionCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

/// One completed model response.
#[derive(Clone, Debug)]
pub struct Reply {
    pub text: String,
    pub call: Option<FunctionCall>,
    /// The provider's own output, replayed verbatim to the same provider so reasoning state
    /// survives; `Null` for history that only has text.
    pub native: Value,
}

#[derive(Clone)]
pub enum Message {
    User(String),
    Assistant(Reply),
    ToolResult { call_id: String, output: String },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Usage {
    /// The whole request, including cached input.
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub enum Event {
    Text(String),
    /// Usage is `None` when the provider did not report it; it is never guessed as zero.
    Done {
        reply: Reply,
        usage: Option<Usage>,
    },
}

pub struct Request<'a> {
    pub model: &'a str,
    pub key: &'a str,
    pub system: &'a str,
    pub messages: &'a [Message],
    pub tools: &'a [Tool],
    /// The tools stay declared so the history remains valid, but the model may not call one.
    pub tools_disabled: bool,
    /// `None` leaves the model's own output limit in effect where the provider allows it.
    pub max_output_tokens: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
pub struct Capabilities {
    pub tool_calls: bool,
    /// `limits` asks the provider; otherwise the context window must be configured.
    pub model_limits: bool,
}

#[derive(Default, Debug, PartialEq)]
pub struct Limits {
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

pub trait Provider: Sync {
    fn capabilities(&self) -> Capabilities;
    fn limits(&self, model: &str, key: &str) -> impl Future<Output = Result<Limits, Error>> + Send;
    /// Text deltas as they arrive, then exactly one `Done`. A stream that ends early is an
    /// error, never a partial reply.
    fn stream<'a>(
        &'a self,
        request: &'a Request<'a>,
    ) -> impl Stream<Item = Result<Event, Error>> + Send + 'a;
}

/// Controlled-provider qualification accepts literal loopback only, never an arbitrary host.
fn loopback(origin: &str) -> Result<Url, Error> {
    let url = Url::parse(origin).map_err(|_| Error::Configuration)?;
    let loopback = url
        .host_str()
        .and_then(|host| {
            host.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .ok()
        })
        .is_some_and(|ip| ip.is_loopback());
    if !loopback
        || url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(Error::Configuration);
    }
    Ok(url)
}

fn client() -> Result<Client, Error> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .no_proxy()
        .referer(false)
        .connect_timeout(Duration::from_secs(5))
        // Long responses stream for minutes; only a stalled stream is a failure.
        .read_timeout(Duration::from_secs(120))
        .build()
        .map_err(|_| Error::Configuration)
}

async fn send(request: RequestBuilder) -> Result<Response, Error> {
    let response = request.send().await.map_err(unavailable)?;
    let status = response.status().as_u16();
    if response.status().is_success() {
        return Ok(response);
    }
    eprintln!("Provider HTTP failure: status={status}");
    Err(match status {
        429 | 500..=599 => Error::Transient(retry_after(&response)),
        _ => Error::Rejected,
    })
}

fn retry_after(response: &Response) -> Option<Duration> {
    let header = |name| {
        response
            .headers()
            .get(name)?
            .to_str()
            .ok()?
            .parse::<f64>()
            .ok()
    };
    // shortcut: the HTTP-date form of retry-after is ignored and falls back to backoff.
    let seconds = header("retry-after-ms")
        .map(|ms| ms / 1000.0)
        .or_else(|| header("retry-after"))?;
    Duration::try_from_secs_f64(seconds).ok()
}

/// A small non-streaming JSON body, such as model metadata.
async fn json(mut response: Response) -> Result<Value, Error> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(unavailable)? {
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err(Error::InvalidResponse);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| Error::InvalidResponse)
}

/// Turns one provider's stream payloads into events.
trait Decoder: Default + Send {
    /// Pushes the events in one payload; true once the provider's final event arrived.
    fn decode(&mut self, data: Value, events: &mut VecDeque<Event>) -> Result<bool, Error>;
}

struct Open<D> {
    reader: sse::Reader,
    decoder: D,
    pending: VecDeque<Event>,
    finished: bool,
}
enum Phase<F, D> {
    Opening(F),
    Open(Open<D>),
}

fn events<'a, D: Decoder + 'a>(
    open: impl Future<Output = Result<Response, Error>> + Send + 'a,
) -> impl Stream<Item = Result<Event, Error>> + Send + 'a {
    futures_util::stream::try_unfold(Phase::Opening(Box::pin(open)), |phase| async move {
        let mut open = match phase {
            Phase::Opening(future) => Open {
                reader: sse::Reader::new(future.await?),
                decoder: D::default(),
                pending: VecDeque::new(),
                finished: false,
            },
            Phase::Open(open) => open,
        };
        loop {
            if let Some(event) = open.pending.pop_front() {
                return Ok(Some((event, Phase::Open(open))));
            }
            if open.finished {
                return Ok(None);
            }
            // A stream that closes before its final event is as ambiguous as a lost response.
            let data = open.reader.next().await?.ok_or(Error::Unavailable)?;
            open.finished = open.decoder.decode(data, &mut open.pending)?;
        }
    })
}

fn reply(text: String, call: Option<FunctionCall>, native: Value) -> Result<Reply, Error> {
    if (call.is_none() && text.is_empty()) || text.contains('\0') {
        return Err(Error::InvalidResponse);
    }
    Ok(Reply { text, call, native })
}

fn call(call_id: &str, name: &str, arguments: String) -> Result<FunctionCall, Error> {
    if call_id.is_empty()
        || call_id.len() > 256
        || name.is_empty()
        || name.len() > 128
        || arguments.len() > 16384
    {
        return Err(Error::InvalidResponse);
    }
    Ok(FunctionCall {
        call_id: call_id.into(),
        name: name.into(),
        arguments,
    })
}

// Do not format reqwest errors: their URLs and request data are not diagnostics.
fn unavailable(error: reqwest::Error) -> Error {
    eprintln!(
        "Provider transport unavailable: timeout={} connect={} body={}",
        error.is_timeout(),
        error.is_connect(),
        error.is_body()
    );
    Error::Unavailable
}

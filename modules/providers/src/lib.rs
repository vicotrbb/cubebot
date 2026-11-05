use futures_util::Stream;
use reqwest::{Client, RequestBuilder, Response, Url};
use serde_json::Value;
use std::{collections::VecDeque, time::Duration};

mod anthropic;
mod completions;
mod gemini;
mod grok;
mod openai;
mod openrouter;
mod prices;
mod sse;
pub use anthropic::Anthropic;
pub use gemini::Gemini;
pub use grok::Grok;
pub use openai::OpenAi;
pub use openrouter::OpenRouter;
pub use prices::Price;

type Source = Box<dyn std::error::Error + Send + Sync>;

pub enum Error {
    Configuration(Option<Source>),
    Unavailable,
    Transient(Option<Duration>),
    Rejected,
    InvalidResponse(Option<Source>),
}
// Debug omits the cause: callers format it into reports as the variant name alone.
impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Configuration(_) => f.write_str("Configuration"),
            Self::Unavailable => f.write_str("Unavailable"),
            Self::Transient(delay) => f.debug_tuple("Transient").field(delay).finish(),
            Self::Rejected => f.write_str("Rejected"),
            Self::InvalidResponse(_) => f.write_str("InvalidResponse"),
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Configuration(Some(source)) | Self::InvalidResponse(Some(source)) => {
                Some(source.as_ref())
            }
            _ => None,
        }
    }
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
            Self::Configuration(_) | Self::Unavailable | Self::Transient(_) => {
                "provider_unavailable"
            }
            Self::Rejected => "provider_rejected",
            Self::InvalidResponse(_) => "provider_invalid_response",
        }
    }
}

#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    /// Strict schemas must close every object and require every property; external schemas rarely do, so they are sent as advisory.
    pub strict: bool,
    /// Anthropic's calls arrive as `{"action": member, ...input}`, OpenAI's as `{"actions": [...]}`; results go back in the provider's computer form.
    pub computer: bool,
}

impl Tool {
    pub fn strict(
        name: impl Into<String>,
        description: impl Into<String>,
        properties: Value,
    ) -> Self {
        let required: Vec<&String> = properties
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, _)| key)
            .collect();
        Self {
            name: name.into(),
            description: description.into(),
            parameters: serde_json::json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
            strict: true,
            computer: false,
        }
    }
}

fn computer_calls(messages: &[Message], native: impl Fn(&Value) -> Option<&str>) -> Vec<&str> {
    messages
        .iter()
        .filter_map(|message| match message {
            Message::Assistant(reply) => reply.native.as_array(),
            _ => None,
        })
        .flatten()
        .filter_map(native)
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct FunctionCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug)]
pub struct Reply {
    pub text: String,
    pub calls: Vec<FunctionCall>,
    /// Replayed verbatim to the same provider so reasoning state survives; `Null` for history that only has text.
    pub native: Value,
}

#[derive(Clone)]
pub enum Message {
    User(String),
    Assistant(Reply),
    ToolResult {
        call_id: String,
        output: String,
        image: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cost_usd: Option<f64>,
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
    pub max_output_tokens: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
pub struct Capabilities {
    pub tool_calls: bool,
    pub model_limits: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    pub id: String,
    pub context_window: Option<u64>,
    pub tools: bool,
}

#[derive(Default, Debug, PartialEq)]
pub struct Limits {
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

pub trait Provider: Sync {
    fn capabilities(&self) -> Capabilities;
    fn limits(&self, model: &str, key: &str) -> impl Future<Output = Result<Limits, Error>> + Send;
    fn models(&self, key: &str) -> impl Future<Output = Result<Vec<Model>, Error>> + Send;
    /// A stream that ends early is an error, never a partial reply.
    fn stream<'a>(
        &'a self,
        request: &'a Request<'a>,
    ) -> impl Stream<Item = Result<Event, Error>> + Send + 'a;
}

/// Controlled-provider qualification accepts literal loopback only, never an arbitrary host.
fn loopback(origin: &str) -> Result<Url, Error> {
    let url = Url::parse(origin).map_err(|error| Error::Configuration(Some(error.into())))?;
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
        return Err(Error::Configuration(None));
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
        .map_err(|error| Error::Configuration(Some(error.without_url().into())))
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

async fn json(response: Response) -> Result<Value, Error> {
    let bytes = body(response).await?;
    serde_json::from_slice(&bytes).map_err(|error| Error::InvalidResponse(Some(error.into())))
}

async fn body(mut response: Response) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(unavailable)? {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err(Error::InvalidResponse(None));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn models(body: &Value, window: impl Fn(&Value) -> Option<u64>) -> Result<Vec<Model>, Error> {
    body["data"]
        .as_array()
        .ok_or(Error::InvalidResponse(None))?
        .iter()
        .map(|entry| {
            Ok(Model {
                id: entry["id"]
                    .as_str()
                    .ok_or(Error::InvalidResponse(None))?
                    .to_owned(),
                context_window: window(entry),
                tools: true,
            })
        })
        .collect()
}

trait Decoder: Default + Send {
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
    Open(Box<Open<D>>),
}

fn events<'a, D: Decoder + 'a>(
    provider: &'static str,
    model: &str,
    open: impl Future<Output = Result<Response, Error>> + Send + 'a,
) -> impl Stream<Item = Result<Event, Error>> + Send + 'a {
    let span = tracing::info_span!("provider_request", provider, model);
    let open = Box::pin(tracing::Instrument::instrument(open, span));
    futures_util::stream::try_unfold(Phase::Opening(open), |phase| async move {
        let mut open = match phase {
            Phase::Opening(future) => Box::new(Open {
                reader: sse::Reader::new(future.await?),
                decoder: D::default(),
                pending: VecDeque::new(),
                finished: false,
            }),
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

fn reply(text: String, calls: Vec<FunctionCall>, native: Value) -> Result<Reply, Error> {
    if (calls.is_empty() && text.is_empty()) || text.contains('\0') {
        return Err(Error::InvalidResponse(None));
    }
    Ok(Reply {
        text,
        calls,
        native,
    })
}

const ARGUMENT_BYTES: usize = 1024 * 1024;

fn call(call_id: &str, name: &str, arguments: String) -> Result<FunctionCall, Error> {
    if call_id.is_empty()
        || call_id.len() > 256
        || name.is_empty()
        || name.len() > 128
        || arguments.len() > ARGUMENT_BYTES
    {
        return Err(Error::InvalidResponse(None));
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
    // A failed connection proves the request never arrived, so retrying cannot repeat it.
    if error.is_connect() {
        Error::Transient(None)
    } else {
        Error::Unavailable
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_strict_tool_closes_its_object_and_requires_every_property() {
        let tool = super::Tool::strict("t", "d", serde_json::json!({"a":{},"b":{}}));
        assert!(tool.strict && !tool.computer);
        assert_eq!(
            tool.parameters,
            serde_json::json!({"type":"object","properties":{"a":{},"b":{}},"required":["a","b"],"additionalProperties":false})
        );
    }

    #[test]
    fn tool_arguments_may_carry_a_large_file_but_stay_bounded() {
        assert!(super::call("c", "write_file", "x".repeat(super::ARGUMENT_BYTES)).is_ok());
        assert!(super::call("c", "write_file", "x".repeat(super::ARGUMENT_BYTES + 1)).is_err());
    }
}

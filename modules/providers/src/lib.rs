//! Native bounded OpenAI inference. No automatic retries or provider-side history.
use cubebot_protocol::chat::Message;
use reqwest::{Client, Url};
use serde_json::{Value, json};
use std::time::Duration;

pub struct OpenAi {
    http: Client,
    endpoint: Url,
}

#[derive(Debug)]
pub enum Error {
    Configuration,
    Unavailable,
    Transient,
    Rejected,
    InvalidResponse,
}
impl Error {
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Transient)
    }
    pub fn code(&self) -> &'static str {
        match self {
            Self::Configuration | Self::Unavailable | Self::Transient => "provider_unavailable",
            Self::Rejected => "provider_rejected",
            Self::InvalidResponse => "provider_invalid_response",
        }
    }
}

impl OpenAi {
    pub fn new() -> Result<Self, Error> {
        Self::endpoint(
            Url::parse("https://api.openai.com/v1/responses").map_err(|_| Error::Configuration)?,
        )
    }

    /// Controlled-provider qualification accepts literal loopback only, never an arbitrary host.
    pub fn for_loopback_tests(origin: &str) -> Result<Self, Error> {
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
        Self::endpoint(url.join("responses").map_err(|_| Error::Configuration)?)
    }

    fn endpoint(endpoint: Url) -> Result<Self, Error> {
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .referer(false)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(45))
            .build()
            .map_err(|_| Error::Configuration)?;
        Ok(Self { http, endpoint })
    }

    pub async fn complete(
        &self,
        model: &str,
        key: &str,
        messages: &[Message],
    ) -> Result<String, Error> {
        extract(
            self.send(
                key,
                json!({"model":model,"input":messages,"store":false,"max_output_tokens":2048}),
            )
            .await?,
        )
    }
    pub(crate) async fn send(&self, key: &str, body: Value) -> Result<Value, Error> {
        let mut response = self
            .http
            .post(self.endpoint.clone())
            .bearer_auth(key)
            .json(&body)
            .send()
            .await
            .map_err(unavailable)?;
        if !response.status().is_success() {
            eprintln!(
                "Provider HTTP failure: status={}",
                response.status().as_u16()
            );
            return Err(match response.status().as_u16() {
                429 | 500..=599 => Error::Transient,
                _ => Error::Rejected,
            });
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(unavailable)? {
            if bytes.len() + chunk.len() > 256 * 1024 {
                return Err(Error::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| Error::InvalidResponse)
    }
}

fn extract(value: Value) -> Result<String, Error> {
    if value["status"] != "completed" {
        return Err(Error::InvalidResponse);
    }
    let output = value["output"].as_array().ok_or(Error::InvalidResponse)?;
    let mut text = String::new();
    for item in output {
        if item["type"] == "reasoning" {
            continue;
        }
        if item["type"] != "message" {
            return Err(Error::InvalidResponse);
        }
        append_content(&mut text, &item["content"])?;
    }
    if text.is_empty() || text.len() > 32768 || text.contains('\0') {
        return Err(Error::InvalidResponse);
    }
    Ok(text)
}
fn append_content(text: &mut String, value: &Value) -> Result<(), Error> {
    for part in value.as_array().ok_or(Error::InvalidResponse)? {
        if part["type"] != "output_text" {
            return Err(Error::InvalidResponse);
        }
        text.push_str(part["text"].as_str().ok_or(Error::InvalidResponse)?);
    }
    Ok(())
}

pub mod tools;

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

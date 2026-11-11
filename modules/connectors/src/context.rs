//! What a call gets: the connection's settings, its short-lived credential and an HTTP client
//! whose failures never carry a URL, which can hold a token or a search.
use crate::Error;
use base64::Engine;
use reqwest::{Method, RequestBuilder, Response, StatusCode, Url};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::LazyLock, time::Duration};

/// A tool result holding this key is a file: the agent saves it in its workspace and answers
/// with the path, so the model never reads raw bytes.
pub const FILE: &str = "$file";
/// A tool input property naming a workspace file to upload. The agent replaces the path with an
/// [`Attachment`] holding its bytes, read as the tool user, so a call reaches only what `bash` can.
pub const ATTACH: &str = "workspace_file";
/// The most a JSON answer, a download or an upload may hold.
pub const LARGEST: usize = 25 * 1024 * 1024;
const RETRIES: u32 = 2;
const LONGEST_WAIT: Duration = Duration::from_secs(10);

/// A workspace file the agent attached under [`ATTACH`].
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    /// The file's name, without its folder.
    pub name: String,
    base64: String,
}

impl Attachment {
    pub fn new(name: &str, bytes: &[u8]) -> Self {
        Self {
            name: name.into(),
            base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        }
    }

    pub fn bytes(&self) -> Result<Vec<u8>, Error> {
        base64::engine::general_purpose::STANDARD
            .decode(&self.base64)
            .map_err(|_| Error::Invalid(format!("{ATTACH} must be a path in the workspace")))
    }
}

/// One connection's credential as an agent holds it; never logged, journaled or shown to the model.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Credential {
    /// An OAuth access token or a minted installation token, sent as a bearer token.
    Token { token: String },
    /// Pasted secrets by field key, such as `token` or `password`.
    Fields { fields: BTreeMap<String, String> },
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credential { .. }")
    }
}

impl Credential {
    /// Strikes every secret from `text`, so even a vendor echoing one cannot leak it to the model.
    pub fn redact(&self, text: &str) -> String {
        let secrets: Vec<&str> = match self {
            Self::Token { token } => vec![token],
            Self::Fields { fields } => fields.values().map(String::as_str).collect(),
        };
        // Short values such as a port would strike ordinary text; secrets are longer.
        secrets
            .into_iter()
            .filter(|secret| secret.len() >= 6)
            .fold(text.to_owned(), |text, secret| {
                text.replace(secret, "[credential]")
            })
    }
}

pub struct Context {
    config: Value,
    credential: Credential,
    http: reqwest::Client,
    /// Tests send every vendor URL here, with the vendor's host as the first path segment.
    origin: Option<Url>,
}

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    // Without a usable TLS stack nothing can be reached; each call reports that.
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .user_agent(concat!("cubebot/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
});

impl Context {
    pub fn new(config: Value, credential: Credential) -> Self {
        Self {
            config,
            credential,
            http: HTTP.clone(),
            origin: None,
        }
    }

    /// Sends vendor URLs to a loopback fixture server instead; addresses already on loopback,
    /// such as a CalDAV server in a test, are left alone.
    pub fn with_origin(mut self, origin: &str) -> Self {
        let origin = Url::parse(origin)
            .ok()
            .filter(|url| url.host_str() == Some("127.0.0.1"));
        assert!(origin.is_some(), "a loopback origin");
        self.origin = origin;
        self
    }

    pub fn config(&self) -> &Value {
        &self.config
    }

    /// A non-empty text setting.
    pub fn setting(&self, key: &str) -> Option<&str> {
        self.config[key]
            .as_str()
            .filter(|value| !value.trim().is_empty())
    }

    pub fn credential(&self) -> &Credential {
        &self.credential
    }

    pub fn token(&self) -> Result<&str, Error> {
        match &self.credential {
            Credential::Token { token } => Ok(token),
            Credential::Fields { fields } => fields
                .get("token")
                .map(String::as_str)
                .ok_or(Error::Unauthorized),
        }
    }

    pub fn secret(&self, key: &str) -> Result<&str, Error> {
        match &self.credential {
            Credential::Fields { fields } => fields.get(key).map(String::as_str),
            Credential::Token { .. } => None,
        }
        .ok_or(Error::Unauthorized)
    }

    pub fn url(&self, url: &str) -> Result<Url, Error> {
        let parsed = Url::parse(url).map_err(|_| Error::Invalid("not a valid address".into()))?;
        let Some(origin) = self.origin.as_ref() else {
            return Ok(parsed);
        };
        if parsed.host_str() == Some("127.0.0.1") {
            return Ok(parsed);
        }
        let mut rerooted = origin.clone();
        let host = parsed.host_str().unwrap_or_default();
        rerooted.set_path(&format!("/{host}{}", parsed.path()));
        rerooted.set_query(parsed.query());
        Ok(rerooted)
    }

    /// A request to `url`, with the bearer token when the credential is one.
    pub fn request(&self, method: Method, url: &str) -> Result<RequestBuilder, Error> {
        let request = self.http.request(method, self.url(url)?);
        Ok(match &self.credential {
            Credential::Token { token } => request.bearer_auth(token),
            Credential::Fields { .. } => request,
        })
    }

    pub fn get(&self, url: &str) -> Result<RequestBuilder, Error> {
        self.request(Method::GET, url)
    }

    pub fn post(&self, url: &str) -> Result<RequestBuilder, Error> {
        self.request(Method::POST, url)
    }

    /// Sends, waiting out short rate limits, and maps a refusal to an [`Error`].
    pub async fn send(&self, request: RequestBuilder) -> Result<Response, Error> {
        let (response, reading) = self.answer(request).await?;
        refused(response, reading).await
    }

    /// [`Context::send`] for a write made conditional on what was read: a 412 is `changed`.
    pub(crate) async fn send_unchanged(
        &self,
        request: RequestBuilder,
        changed: &str,
    ) -> Result<Response, Error> {
        let (response, reading) = self.answer(request).await?;
        if response.status() == StatusCode::PRECONDITION_FAILED {
            return Err(Error::Invalid(changed.into()));
        }
        refused(response, reading).await
    }

    /// The answer after waiting out short rate limits, and whether the request was a read.
    async fn answer(&self, request: RequestBuilder) -> Result<(Response, bool), Error> {
        let request = request
            .build()
            .map_err(|_| Error::Invalid("the request could not be built".into()))?;
        let reading = request.method() == Method::GET;
        let mut current = request;
        for attempt in 0.. {
            let retry = current.try_clone().filter(|_| attempt < RETRIES);
            let response = self
                .http
                .execute(current)
                .await
                .map_err(|error| failed(&error, reading))?;
            match (retry, wait(&response)) {
                (Some(next), Some(delay)) => {
                    tokio::time::sleep(delay).await;
                    current = next;
                }
                _ => return Ok((response, reading)),
            }
        }
        unreachable!("the attempts are unbounded until one returns")
    }

    /// The JSON answer; an empty body is `null`.
    pub async fn json(&self, request: RequestBuilder) -> Result<Value, Error> {
        let (body, _) = self.bytes(request).await?;
        if body.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&body)
            .map_err(|_| Error::Unavailable("the answer was not valid JSON".into()))
    }

    /// The raw answer and its content type, bounded.
    pub async fn bytes(&self, request: RequestBuilder) -> Result<(Vec<u8>, Option<String>), Error> {
        let mut response = self.send(request).await?;
        let kind = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|kind| kind.to_str().ok())
            .map(str::to_owned);
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Uncertain)? {
            if body.len() + chunk.len() > LARGEST {
                return Err(Error::Rejected("the answer is larger than 25 MiB".into()));
            }
            body.extend_from_slice(&chunk);
        }
        Ok((body, kind))
    }
}

/// Typed tool input; the message names what is wrong for the model to correct.
pub fn input<T: DeserializeOwned>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value).map_err(|error| Error::Invalid(error.to_string()))
}

/// A file as a tool result; see [`FILE`].
pub fn file(name: &str, content_type: Option<&str>, bytes: &[u8]) -> Value {
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    json!({FILE: {"name": name, "content_type": content_type, "base64": data}})
}

/// A request that failed without an answer. One that was never sent did nothing; a read did
/// nothing either way; anything else may have taken effect.
fn failed(error: &reqwest::Error, reading: bool) -> Error {
    if error.is_connect() || error.is_builder() || reading {
        let what = if error.is_timeout() {
            "timed out"
        } else {
            "could not connect"
        };
        return Error::Unavailable(what.into());
    }
    Error::Uncertain
}

/// How long a rate limit or brief outage asks to wait, when short enough to wait here.
fn wait(response: &Response) -> Option<Duration> {
    if !matches!(
        response.status(),
        StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
    ) {
        return None;
    }
    let seconds = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1);
    Some(Duration::from_secs(seconds)).filter(|delay| *delay <= LONGEST_WAIT)
}

async fn refused(response: Response, reading: bool) -> Result<Response, Error> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let message = message(&response.bytes().await.unwrap_or_default());
    Err(match status {
        StatusCode::UNAUTHORIZED => Error::Unauthorized,
        StatusCode::FORBIDDEN => Error::Forbidden(message),
        StatusCode::NOT_FOUND | StatusCode::GONE => Error::NotFound(message),
        StatusCode::TOO_MANY_REQUESTS => Error::Unavailable(format!("rate limited: {message}")),
        StatusCode::GATEWAY_TIMEOUT if !reading => Error::Uncertain,
        status if status.is_server_error() => Error::Unavailable(message),
        _ => Error::Rejected(message),
    })
}

/// The vendor's own words from an error body, bounded and on one line.
fn message(body: &[u8]) -> String {
    let parsed: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let candidates = [
        &parsed["error"]["message"],
        &parsed["message"],
        &parsed["error_description"],
        &parsed["description"],
        // Django REST framework's, such as Readwise's.
        &parsed["detail"],
        // Gotify's, whose `error` is only the status's name.
        &parsed["errorDescription"],
        &parsed["error"],
        // GraphQL and Atlassian error bodies.
        &parsed["errors"][0]["message"],
        &parsed["errorMessages"][0],
        // Pushover's.
        &parsed["errors"][0],
    ];
    let text = candidates
        .into_iter()
        .find_map(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| String::from_utf8_lossy(&body[..body.len().min(300)]).into_owned());
    let mut text: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(300)
        .collect();
    if text.trim().is_empty() {
        text = "no reason given".into();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Exchange, Fixture};

    fn token() -> Credential {
        Credential::Token {
            token: "secret-token".into(),
        }
    }

    #[test]
    fn redaction_strikes_every_secret_but_not_short_values() {
        let fields = Credential::Fields {
            fields: [
                ("password".into(), "hunter22".into()),
                ("pin".into(), "42".into()),
            ]
            .into(),
        };
        assert_eq!(
            fields.redact("pw hunter22 pin 42"),
            "pw [credential] pin 42"
        );
        assert_eq!(token().redact("Bearer secret-token"), "Bearer [credential]");
        assert_eq!(format!("{:?}", token()), "Credential { .. }");
    }

    #[test]
    fn an_attachment_carries_its_bytes_and_a_bare_path_is_refused() {
        let attached = serde_json::to_value(Attachment::new("a.pdf", b"%PDF")).unwrap();
        let read: Attachment = input(attached).unwrap();
        assert_eq!(
            (read.name.as_str(), read.bytes().unwrap()),
            ("a.pdf", b"%PDF".to_vec())
        );
        assert!(input::<Attachment>(json!("notes/a.pdf")).is_err());
    }

    #[test]
    fn tests_reroot_vendor_urls_but_not_loopback_ones() {
        let cx = Context::new(json!({}), token()).with_origin("http://127.0.0.1:9/");
        let url = cx.url("https://api.example.com/v1/items?q=a").unwrap();
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:9/api.example.com/v1/items?q=a"
        );
        let local = cx.url("http://127.0.0.1:5232/dav/").unwrap();
        assert_eq!(local.as_str(), "http://127.0.0.1:5232/dav/");
    }

    #[tokio::test]
    async fn refusals_map_to_errors_with_the_vendors_words_and_a_short_rate_limit_is_waited_out() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api.example.com/a",
                401,
                json!({"message": "bad token"}),
            ),
            Exchange::new(
                "GET",
                "/api.example.com/b",
                404,
                json!({"error": {"message": "no such item"}}),
            ),
            Exchange::new("GET", "/api.example.com/c", 429, json!({})).header("retry-after", "0"),
            Exchange::new("GET", "/api.example.com/c", 200, json!({"ok": true})),
            Exchange::new(
                "POST",
                "/api.example.com/d",
                422,
                json!({"error_description": "line\nbreak"}),
            ),
        ])
        .await;
        let cx = Context::new(json!({}), token()).with_origin(&fixture.origin());
        let get = |path: &str| cx.get(&format!("https://api.example.com/{path}")).unwrap();
        assert_eq!(cx.json(get("a")).await, Err(Error::Unauthorized));
        assert_eq!(
            cx.json(get("b")).await,
            Err(Error::NotFound("no such item".into()))
        );
        assert_eq!(cx.json(get("c")).await, Ok(json!({"ok": true})));
        let post = cx.post("https://api.example.com/d").unwrap();
        assert_eq!(
            cx.json(post).await,
            Err(Error::Rejected("line break".into()))
        );
        let seen = fixture.finish().await;
        assert_eq!(seen[0].header("authorization"), Some("Bearer secret-token"));
    }

    #[tokio::test]
    async fn a_write_that_got_no_answer_is_uncertain_but_an_unreachable_one_did_nothing() {
        let cx = Context::new(json!({}), token());
        let unreachable = cx.post("http://127.0.0.1:9/never").unwrap();
        assert!(matches!(
            cx.json(unreachable).await,
            Err(Error::Unavailable(_))
        ));
        let fixture = Fixture::silent().await;
        let cx = Context::new(json!({}), token());
        let request = cx
            .post(&format!("{}hang", fixture.origin()))
            .unwrap()
            .timeout(Duration::from_millis(200));
        assert_eq!(cx.json(request).await, Err(Error::Uncertain));
    }
}

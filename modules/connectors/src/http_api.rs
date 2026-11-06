//! Any other HTTP API: a base URL, a header carrying the token, and the paths the agent may call.
//! The person chose the API but the model chooses paths, so every path is checked here before a
//! request leaves; redirects are never followed, so an answer cannot lead the token elsewhere.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Credential, Error, Field, FieldKind,
    Level, Manifest, Risk, ToolSpec,
};
use reqwest::{Method, RequestBuilder, Url};
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// The most characters of an answer the model gets.
const LONGEST: usize = 60_000;

pub struct HttpApi;

static MANIFEST: Manifest = Manifest {
    id: "http-api",
    name: "HTTP API",
    description: "Call any HTTP API with a token, limited to the paths you allow.",
    category: Category::Dev,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "Token",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "",
            help: "Sent in the header below with every request.",
        }],
    }],
    config: &[
        Field {
            key: "base_url",
            label: "Base URL",
            kind: FieldKind::Url,
            required: true,
            placeholder: "https://api.example.com/v1/",
            help: "Every request stays on this address and under its path.",
        },
        Field {
            key: "auth_header",
            label: "Header",
            kind: FieldKind::Text,
            required: false,
            placeholder: "Authorization",
            help: "The header that carries the token.",
        },
        Field {
            key: "auth_scheme",
            label: "Scheme",
            kind: FieldKind::Text,
            required: false,
            placeholder: "Bearer",
            help: "Written before the token; clear it to send the token alone.",
        },
        Field {
            key: "allowed_paths",
            label: "Allowed paths",
            kind: FieldKind::Lines,
            required: false,
            placeholder: "/",
            help: "Path prefixes under the base URL the agent may call, one per line. All when empty.",
        },
        Field {
            key: "check_path",
            label: "Test path",
            kind: FieldKind::Text,
            required: false,
            placeholder: "/me",
            help: "A path to GET when testing the connection; the base URL itself when empty.",
        },
    ],
    products: &[],
    tools: &[
        ToolSpec {
            name: "get",
            description: "GET a path under the API's base URL. JSON and text come back as such; anything else is saved as a file.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"path":{"type":"string","description":"Starts with /, relative to the base URL, without a query."},"query":{"type":"object","additionalProperties":{"type":["string","number","boolean"]}}},"required":["path"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "request",
            description: "Send a POST, PUT, PATCH or DELETE with an optional JSON body to a path under the API's base URL.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"method":{"type":"string","enum":["POST","PUT","PATCH","DELETE"]},"path":{"type":"string","description":"Starts with /, relative to the base URL, without a query."},"body":{"description":"Sent as JSON."},"query":{"type":"object","additionalProperties":{"type":["string","number","boolean"]}}},"required":["method","path"],"additionalProperties":false}"#,
        },
    ],
    events: &[],
    level: Level::Write,
    guide: &[],
    warning: Some(
        "The agent can call every allowed path with this token, so give the token only the access the agent should have.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Get {
    path: String,
    #[serde(default)]
    query: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    method: String,
    path: String,
    body: Option<Value>,
    #[serde(default)]
    query: Map<String, Value>,
}

impl Connector for HttpApi {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let url = match cx.setting("check_path") {
                Some(path) => resolve(cx, path.trim())?,
                None => base(cx)?,
            };
            cx.send(prepare(cx, Method::GET, &url, &Map::new())?)
                .await?;
            let host = url.host_str().unwrap_or("HTTP API");
            Ok(Account { name: host.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "get" => {
                    let get: Get = crate::input(input)?;
                    let url = permitted(cx, &get.path)?;
                    let (body, kind) = cx
                        .bytes(prepare(cx, Method::GET, &url, &get.query)?)
                        .await?;
                    Ok(answer(&url, body, kind))
                }
                "request" => request(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }
}

async fn request(cx: &Context, input: Value) -> Result<Value, Error> {
    let call: Call = crate::input(input)?;
    let method = match call.method.as_str() {
        "POST" => Method::POST,
        "PUT" => Method::PUT,
        "PATCH" => Method::PATCH,
        "DELETE" => Method::DELETE,
        _ => {
            return Err(Error::Invalid(
                "the method is POST, PUT, PATCH or DELETE".into(),
            ));
        }
    };
    let url = permitted(cx, &call.path)?;
    let mut request = prepare(cx, method, &url, &call.query)?;
    if let Some(body) = &call.body {
        request = request.json(body);
    }
    let (body, kind) = cx.bytes(request).await?;
    Ok(answer(&url, body, kind))
}

/// The base URL, which must be an http or https address.
fn base(cx: &Context) -> Result<Url, Error> {
    cx.setting("base_url")
        .and_then(|url| Url::parse(url.trim()).ok())
        .filter(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some())
        .ok_or_else(|| {
            Error::Invalid("the connection's base URL is not an http or https address".into())
        })
}

/// `path` under the base URL, refused unless it stays on the base's address and under its path.
fn resolve(cx: &Context, path: &str) -> Result<Url, Error> {
    let base = base(cx)?;
    let lower = path.to_ascii_lowercase();
    // Encoded dots and slashes too: a server may decode them into a traversal.
    let tricks = ["..", "//", "\\", "@", "?", "#", "%2e", "%2f", "%5c"];
    if !path.starts_with('/')
        || path.chars().any(char::is_control)
        || tricks.iter().any(|trick| lower.contains(trick))
    {
        return Err(Error::Invalid(format!(
            "{path:?} is not a plain path such as /v1/items; pass the query separately"
        )));
    }
    let root = base.path().trim_end_matches('/');
    let mut url = base.clone();
    url.set_query(None);
    url.set_fragment(None);
    url.set_path(&format!("{root}{path}"));
    let same = url.scheme() == base.scheme()
        && url.host_str() == base.host_str()
        && url.port_or_known_default() == base.port_or_known_default()
        && url.path().starts_with(root);
    same.then_some(url)
        .ok_or_else(|| Error::Invalid(format!("{path:?} leaves the base URL")))
}

/// [`resolve`], and within one of the connection's allowed path prefixes.
fn permitted(cx: &Context, path: &str) -> Result<Url, Error> {
    let url = resolve(cx, path)?;
    let listed: Vec<&str> = cx.config()["allowed_paths"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|prefix| !prefix.is_empty())
        .collect();
    let prefixes = if listed.is_empty() { vec!["/"] } else { listed };
    // `/v1/users` allows `/v1/users/1` but not `/v1/users-admin`.
    let within = |prefix: &&str| {
        path.strip_prefix(*prefix)
            .is_some_and(|rest| rest.is_empty() || prefix.ends_with('/') || rest.starts_with('/'))
    };
    if prefixes.iter().any(within) {
        return Ok(url);
    }
    Err(Error::Forbidden(format!(
        "{path} is outside the paths this connection allows: {}",
        prefixes.join(", ")
    )))
}

/// A request with the token in the connection's header.
fn prepare(
    cx: &Context,
    method: Method,
    url: &Url,
    query: &Map<String, Value>,
) -> Result<RequestBuilder, Error> {
    let pairs: Vec<(&str, String)> = query
        .iter()
        .map(|(key, value)| {
            let value = value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned);
            (key.as_str(), value)
        })
        .collect();
    let request = cx.request(method, url.as_str())?.query(&pairs);
    // Context already sends a token credential as a bearer.
    let Credential::Fields { .. } = cx.credential() else {
        return Ok(request);
    };
    let header = cx.setting("auth_header").unwrap_or("Authorization").trim();
    let scheme = cx.config()["auth_scheme"]
        .as_str()
        .unwrap_or("Bearer")
        .trim();
    let token = cx.token()?;
    let value = match scheme {
        "" => token.to_owned(),
        scheme => format!("{scheme} {token}"),
    };
    Ok(request.header(header, value))
}

/// JSON as JSON, text as text, both bounded; anything else as a file.
fn answer(url: &Url, body: Vec<u8>, kind: Option<String>) -> Value {
    let kind = kind.unwrap_or_default().to_ascii_lowercase();
    let textual = kind.is_empty()
        || kind.starts_with("text/")
        || ["json", "xml", "javascript"]
            .iter()
            .any(|text| kind.contains(text));
    let body = match String::from_utf8(body) {
        Ok(text) if textual => return text_answer(text, &kind),
        Ok(text) => text.into_bytes(),
        Err(binary) => binary.into_bytes(),
    };
    let name = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|name| !name.is_empty())
        .unwrap_or("download");
    crate::file(
        name,
        Some(kind.as_str()).filter(|kind| !kind.is_empty()),
        &body,
    )
}

fn text_answer(text: String, kind: &str) -> Value {
    if kind.contains("json")
        && text.len() <= LONGEST
        && let Ok(parsed) = serde_json::from_str::<Value>(&text)
    {
        return json!({"json": parsed});
    }
    match text.char_indices().nth(LONGEST) {
        Some((end, _)) => json!({"text": &text[..end], "cut_short": true}),
        None => json!({"text": text}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Exchange, Fixture};

    fn cx(fixture: &Fixture, mut config: Value) -> Context {
        config["base_url"] = json!(format!("{}api/v1/", fixture.origin()));
        let credential = Credential::Fields {
            fields: [("token".into(), "api-secret".into())].into(),
        };
        Context::new(config, credential).with_origin(&fixture.origin())
    }

    #[tokio::test]
    async fn paths_that_could_leave_the_api_or_the_allowed_prefixes_are_refused_before_sending() {
        let fixture = Fixture::start(Vec::new()).await;
        let cx = cx(&fixture, json!({"allowed_paths": ["/items", " /search/ "]}));
        let invalid = [
            "/../admin",
            "/items/../../admin",
            "//evil.com/items",
            "/a@b",
            "http://x/items",
            "items",
            "/items\\x",
            "/items/%2E%2E/admin",
            "/items?admin=1",
            "/items\n",
        ];
        for path in invalid {
            let refused = HttpApi.call(&cx, "get", json!({"path": path})).await;
            assert!(
                matches!(refused, Err(Error::Invalid(_))),
                "{path:?}: {refused:?}"
            );
        }
        for path in ["/admin", "/items-admin", "/search"] {
            let refused = HttpApi.call(&cx, "get", json!({"path": path})).await;
            assert!(
                matches!(refused, Err(Error::Forbidden(_))),
                "{path:?}: {refused:?}"
            );
        }
        assert!(fixture.finish().await.is_empty());
    }

    #[tokio::test]
    async fn the_token_goes_in_the_configured_header_and_answers_keep_their_kind() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api/v1/items?limit=2&tag=a",
                200,
                json!([{"id": 1}]),
            ),
            Exchange::raw("GET", "/api/v1/items/1/notes", 200, "plain words")
                .header("content-type", "text/plain"),
            Exchange::raw("GET", "/api/v1/items/1/photo.png", 200, "PNG")
                .header("content-type", "image/png"),
            Exchange::raw("GET", "/api/v1/items/2", 302, "")
                .header("location", "https://evil.example/"),
        ])
        .await;
        let cx = cx(
            &fixture,
            json!({"auth_header": "X-Api-Key", "auth_scheme": ""}),
        );
        let query = json!({"path": "/items", "query": {"tag": "a", "limit": 2}});
        let json = HttpApi.call(&cx, "get", query).await.unwrap();
        assert_eq!(json, json!({"json": [{"id": 1}]}));
        let text = HttpApi
            .call(&cx, "get", json!({"path": "/items/1/notes"}))
            .await;
        assert_eq!(text, Ok(json!({"text": "plain words"})));
        let photo = HttpApi
            .call(&cx, "get", json!({"path": "/items/1/photo.png"}))
            .await;
        let photo = photo.unwrap();
        assert_eq!(photo[crate::FILE]["name"], "photo.png");
        assert_eq!(photo[crate::FILE]["content_type"], "image/png");
        let redirected = HttpApi.call(&cx, "get", json!({"path": "/items/2"})).await;
        assert!(
            matches!(redirected, Err(Error::Rejected(_))),
            "{redirected:?}"
        );
        let seen = fixture.finish().await;
        assert_eq!(seen[0].header("x-api-key"), Some("api-secret"));
        assert_eq!(seen[0].header("authorization"), None);
    }

    #[tokio::test]
    async fn request_sends_its_json_body_with_its_method_and_a_bearer_by_default() {
        let fixture = Fixture::start(vec![Exchange::new(
            "PATCH",
            "/api/v1/items/1?dry=true",
            200,
            json!({"id": 1, "name": "new"}),
        )])
        .await;
        let cx = cx(&fixture, json!({}));
        let input = json!({"method": "PATCH", "path": "/items/1", "query": {"dry": true}, "body": {"name": "new"}});
        let changed = HttpApi.call(&cx, "request", input).await.unwrap();
        assert_eq!(changed["json"]["name"], "new");
        let bad = json!({"method": "GET", "path": "/items"});
        let refused = HttpApi.call(&cx, "request", bad).await;
        assert!(matches!(refused, Err(Error::Invalid(_))));
        let seen = fixture.finish().await;
        assert_eq!(seen[0].header("authorization"), Some("Bearer api-secret"));
        assert_eq!(seen[0].json(), json!({"name": "new"}));
    }
}

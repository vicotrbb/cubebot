//! ntfy push notifications (https://docs.ntfy.sh/publish/, https://docs.ntfy.sh/subscribe/api/),
//! on ntfy.sh or a self-hosted server. Topics are names in the URL path, so they are checked before
//! any request is built.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Field, FieldKind, Level, Manifest,
    PRIVATE_ADDRESS, Risk, Step, ToolSpec,
};
use reqwest::{Method, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const SERVER: &str = "https://ntfy.sh";
const READ: usize = 50;
const MOST_READ: usize = 200;

pub struct Ntfy;

static MANIFEST: Manifest = Manifest {
    id: "ntfy",
    name: "ntfy",
    description: "Push notifications to your phone or desktop through ntfy.sh or your own ntfy server.",
    category: Category::Notifications,
    auth: &[
        Auth::Public,
        Auth::ApiKey {
            fields: &[Field {
                key: "token",
                label: "Access token",
                kind: FieldKind::Secret,
                required: true,
                placeholder: "tk_…",
                help: "From Account, Access tokens in the ntfy web app, or ntfy token add on your server.",
            }],
        },
        Auth::Password {
            fields: &[
                Field {
                    key: "username",
                    label: "Username",
                    kind: FieldKind::Text,
                    required: true,
                    placeholder: "",
                    help: "",
                },
                Field {
                    key: "password",
                    label: "Password",
                    kind: FieldKind::Secret,
                    required: true,
                    placeholder: "",
                    help: "",
                },
            ],
        },
    ],
    config: &[Field {
        key: "url",
        label: "Server address",
        kind: FieldKind::Url,
        required: false,
        placeholder: SERVER,
        help: "Leave empty for ntfy.sh.",
    }],
    products: &[],
    tools: &[
        ToolSpec {
            name: "publish",
            description: "Publish a notification to a topic; everyone subscribed to it gets a push.",
            risk: Risk::Send,
            product: None,
            input: r#"{"type":"object","properties":{"topic":{"type":"string","description":"Letters, digits, underscores and hyphens, up to 64."},"message":{"type":"string"},"title":{"type":"string"},"priority":{"type":"integer","minimum":1,"maximum":5,"description":"1 min, 2 low, 3 default, 4 high, 5 urgent."},"tags":{"type":"array","items":{"type":"string"},"description":"Tags; names of emoji such as warning or white_check_mark show as icons."},"click":{"type":"string","description":"A URL opened when the notification is tapped."},"markdown":{"type":"boolean","description":"Render the message as Markdown in the web and desktop apps."}},"required":["topic","message"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_messages",
            description: "Messages a topic's server still holds (ntfy.sh keeps them 12 hours), oldest first. One published seconds ago may not be there yet.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"topic":{"type":"string"},"since":{"type":"string","description":"A duration such as 30m, a Unix time, a message id, or all (the default)."},"limit":{"type":"integer","minimum":1,"maximum":200,"description":"The newest this many; 50 unless given."}},"required":["topic"],"additionalProperties":false}"#,
        },
    ],
    events: &[],
    level: Level::Full,
    guide: &[
        Step {
            title: "Choose a server",
            body: "Leave the address empty to use the free public server, ntfy.sh, or enter the address of your own ntfy server.",
            link: Some("https://docs.ntfy.sh/"),
            copy: None,
        },
        Step {
            title: "Choose how to sign in",
            body: "On ntfy.sh any topic you have not reserved is open: sign in with nothing and pick a long topic name nobody could guess. For reserved topics or a server with access control, make an access token under Account, Access tokens in the ntfy web app (or run ntfy token add on your server), or use a username and password.",
            link: None,
            copy: None,
        },
        Step {
            title: "Subscribe on your phone",
            body: "Install the ntfy app and subscribe to the same topic on the same server. Tell your agents the topic's name, for example in their instructions.",
            link: Some("https://ntfy.sh/app"),
            copy: None,
        },
        PRIVATE_ADDRESS,
    ],
    warning: Some(
        "On ntfy.sh a topic nobody reserved is public: anyone who knows its name can read what agents publish there and post to it.",
    ),
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Publish {
    topic: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    priority: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    click: Option<String>,
    #[serde(default)]
    markdown: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    topic: String,
    since: Option<String>,
    limit: Option<usize>,
}

impl Connector for Ntfy {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let account = cx.json(api(cx, Method::GET, "v1/account")?).await?;
            let user = match account["username"].as_str() {
                Some("*") | None => "anonymous",
                Some(name) => name,
            };
            let server = server(cx);
            let host = server.split("://").nth(1).unwrap_or(server);
            Ok(Account {
                name: format!("{user} on {}", host.trim_end_matches('/')),
            })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "publish" => publish(cx, input).await,
                "read_messages" => read(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }
}

fn server(cx: &Context) -> &str {
    cx.setting("url").unwrap_or(SERVER)
}

/// A request to `<server>/<path>`, signed with whichever credential the connection has.
fn api(cx: &Context, method: Method, path: &str) -> Result<RequestBuilder, Error> {
    let url = format!("{}/{path}", server(cx).trim_end_matches('/'));
    let request = cx.request(method, &url)?;
    Ok(
        match (
            cx.secret("token"),
            cx.secret("username"),
            cx.secret("password"),
        ) {
            (Ok(token), _, _) => request.bearer_auth(token),
            (_, Ok(user), Ok(password)) => request.basic_auth(user, Some(password)),
            _ => request,
        },
    )
}

/// ntfy's own rule for topic names, which also keeps them a single path segment.
fn topic(name: &str) -> Result<&str, Error> {
    let fine = (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    fine.then_some(name).ok_or_else(|| {
        Error::Invalid("a topic is up to 64 letters, digits, underscores and hyphens".into())
    })
}

async fn publish(cx: &Context, input: Value) -> Result<Value, Error> {
    let publish: Publish = crate::input(input)?;
    topic(&publish.topic)?;
    if publish.message.trim().is_empty() {
        return Err(Error::Invalid("message is empty".into()));
    }
    let sent = cx.json(api(cx, Method::POST, "")?.json(&publish)).await?;
    Ok(json!({"id": sent["id"], "time": sent["time"], "topic": sent["topic"]}))
}

async fn read(cx: &Context, input: Value) -> Result<Value, Error> {
    let read: Read = crate::input(input)?;
    let since = read.since.unwrap_or_else(|| "all".into());
    if since.is_empty() || !since.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(Error::Invalid(
            "since is a duration such as 30m, a Unix time, a message id or all".into(),
        ));
    }
    let path = format!("{}/json", topic(&read.topic)?);
    let request = api(cx, Method::GET, &path)?.query(&[("poll", "1"), ("since", &since)]);
    let (body, _) = cx.bytes(request).await?;
    // One JSON object per line; only `message` events are messages.
    let messages: Vec<Value> = String::from_utf8_lossy(&body)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["event"] == "message")
        .map(|message| {
            json!({
                "id": message["id"],
                "time": message["time"],
                "title": message["title"],
                "message": message["message"],
                "priority": message["priority"],
                "tags": message["tags"],
                "click": message["click"],
            })
        })
        .collect();
    let limit = read.limit.unwrap_or(READ).clamp(1, MOST_READ);
    let newest = messages[messages.len().saturating_sub(limit)..].to_vec();
    Ok(json!({"messages": newest, "held": messages.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    fn cx(fixture: &Fixture, fields: &[(&str, &str)]) -> Context {
        let credential = Credential::Fields {
            fields: fields
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into()))
                .collect(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    /// Recorded from ntfy.sh on 2026-11-06.
    fn message(id: &str, text: &str) -> String {
        json!({"id": id, "time": 1793995648, "expires": 1794038848, "event": "message", "topic": "cubebot-alerts", "title": "Backup", "message": text, "priority": 4, "tags": ["warning"], "click": "https://example.com", "content_type": "text/markdown"}).to_string()
    }

    #[tokio::test]
    async fn publish_sends_the_notification_as_json_with_the_token() {
        let fixture = Fixture::start(vec![Exchange::new(
            "POST",
            "/ntfy.sh/",
            200,
            serde_json::from_str(&message("RjnZI0iPSIiX", "done")).unwrap(),
        )])
        .await;
        let input = json!({"topic": "cubebot-alerts", "message": "done", "title": "Backup", "priority": 4, "tags": ["warning"], "markdown": true});
        let cx = cx(&fixture, &[("token", "tk_secret")]);
        let sent = Ntfy.call(&cx, "publish", input).await.unwrap();
        assert_eq!(
            sent,
            json!({"id": "RjnZI0iPSIiX", "time": 1793995648, "topic": "cubebot-alerts"})
        );
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].json(),
            json!({"topic": "cubebot-alerts", "message": "done", "title": "Backup", "priority": 4, "tags": ["warning"], "markdown": true})
        );
        assert_eq!(seen[0].header("authorization"), Some("Bearer tk_secret"));
    }

    #[tokio::test]
    async fn read_messages_polls_the_cache_and_keeps_only_the_newest_messages() {
        let open = json!({"id": "x", "time": 1, "event": "open", "topic": "cubebot-alerts"});
        let body = format!(
            "{open}\n{}\n{}\n",
            message("a1", "first"),
            message("b2", "second")
        );
        let fixture = Fixture::start(vec![Exchange::raw(
            "GET",
            "/ntfy.sh/cubebot-alerts/json",
            200,
            &body,
        )])
        .await;
        let cx = cx(&fixture, &[("username", "ana"), ("password", "hunter22")]);
        let input = json!({"topic": "cubebot-alerts", "since": "10m", "limit": 1});
        let read = Ntfy.call(&cx, "read_messages", input).await.unwrap();
        assert_eq!(read["held"], 2);
        assert_eq!(read["messages"][0]["id"], "b2");
        assert_eq!(read["messages"][0]["message"], "second");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].target,
            "/ntfy.sh/cubebot-alerts/json?poll=1&since=10m"
        );
        assert_eq!(
            seen[0].header("authorization"),
            Some("Basic YW5hOmh1bnRlcjIy")
        );
    }

    #[tokio::test]
    async fn a_topic_or_since_that_could_leave_the_path_never_reaches_the_server() {
        let fixture = Fixture::start(Vec::new()).await;
        let cx = cx(&fixture, &[]);
        let bad = [
            (
                "publish",
                json!({"topic": "../v1/account", "message": "hi"}),
            ),
            ("publish", json!({"topic": "alerts", "message": " "})),
            ("read_messages", json!({"topic": "a/b"})),
            (
                "read_messages",
                json!({"topic": "alerts", "since": "1&x=2"}),
            ),
        ];
        for (tool, input) in bad {
            let refused = Ntfy.call(&cx, tool, input).await;
            assert!(
                matches!(refused, Err(Error::Invalid(_))),
                "{tool}: {refused:?}"
            );
        }
        assert!(fixture.finish().await.is_empty());
    }

    #[tokio::test]
    async fn a_refused_token_is_unauthorized_and_a_forbidden_topic_says_so() {
        let fixture = Fixture::start(vec![
            Exchange::new("GET", "/ntfy.sh/v1/account", 401, json!({"code": 40101, "http": 401, "error": "unauthorized", "link": "https://ntfy.sh/docs/publish/#authentication"})),
            Exchange::new("POST", "/ntfy.sh/", 403, json!({"code": 40301, "http": 403, "error": "forbidden", "link": "https://ntfy.sh/docs/publish/#authentication"})),
        ])
        .await;
        let cx = cx(&fixture, &[("token", "tk_secret")]);
        assert_eq!(Ntfy.check(&cx).await, Err(Error::Unauthorized));
        let input = json!({"topic": "reserved", "message": "hi"});
        let refused = Ntfy.call(&cx, "publish", input).await;
        assert_eq!(refused, Err(Error::Forbidden("forbidden".into())));
        fixture.finish().await;
    }

    /// Publishes and reads back on a real server: set `CUBEBOT_LIVE_NTFY_TOPIC` (a long random
    /// name), optionally `CUBEBOT_LIVE_NTFY_URL` and `CUBEBOT_LIVE_NTFY_TOKEN`, and run
    /// `cargo test -p cubebot-connectors live_ntfy -- --ignored`.
    #[tokio::test]
    #[ignore = "needs an ntfy server"]
    async fn live_ntfy() {
        let env = |key: &str| std::env::var(format!("CUBEBOT_LIVE_NTFY_{key}")).ok();
        let fields = env("TOKEN").map(|token| ("token".to_owned(), token));
        let credential = Credential::Fields {
            fields: fields.into_iter().collect(),
        };
        let cx = Context::new(json!({"url": env("URL")}), credential);
        println!("check: {:?}", Ntfy.check(&cx).await.unwrap());
        let topic = env("TOPIC").unwrap();
        let text = format!("Live check {}", crate::dav::now_nanos());
        let input = json!({"topic": topic, "message": text, "title": "Cubebot", "priority": 2, "tags": ["robot"], "markdown": true});
        let sent = Ntfy.call(&cx, "publish", input).await.unwrap();
        println!("publish: {sent}");
        // ntfy.sh writes its message cache in batches, so a new message shows up a moment later.
        for _ in 0..10 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let input = json!({"topic": topic, "since": "5m"});
            let read = Ntfy.call(&cx, "read_messages", input).await.unwrap();
            println!("read_messages: {read}");
            let messages = read["messages"].as_array().unwrap();
            if messages
                .iter()
                .any(|message| message["id"] == sent["id"] && message["message"] == text)
            {
                return;
            }
        }
        panic!("the published message was never read back");
    }
}

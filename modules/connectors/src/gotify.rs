//! Gotify, a self-hosted notification server (https://gotify.net/api-docs). An application token
//! only sends; reading needs a client token. Both go in the `X-Gotify-Key` header.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Field, FieldKind, Level, Manifest,
    PRIVATE_ADDRESS, Risk, Step, ToolSpec,
};
use reqwest::{Method, RequestBuilder};
use serde::Deserialize;
use serde_json::{Value, json};

const LISTED: u32 = 25;
const MOST_LISTED: u32 = 200;

pub struct Gotify;

static MANIFEST: Manifest = Manifest {
    id: "gotify",
    name: "Gotify",
    description: "Send and read notifications on your own Gotify server.",
    category: Category::Notifications,
    auth: &[Auth::ApiKey {
        fields: &[
            Field {
                key: "app_token",
                label: "Application token",
                kind: FieldKind::Secret,
                required: true,
                placeholder: "",
                help: "From the application you create for Cubebot under Apps; it sends.",
            },
            Field {
                key: "client_token",
                label: "Client token",
                kind: FieldKind::Secret,
                required: false,
                placeholder: "",
                help: "Optional, from Clients: lets agents read messages. It can read every application's messages.",
            },
        ],
    }],
    config: &[Field {
        key: "url",
        label: "Server address",
        kind: FieldKind::Url,
        required: true,
        placeholder: "http://gotify.local",
        help: "The address you open Gotify at.",
    }],
    products: &[],
    tools: &[
        ToolSpec {
            name: "send_message",
            description: "Send a notification through the connection's application.",
            risk: Risk::Send,
            product: None,
            input: r#"{"type":"object","properties":{"message":{"type":"string"},"title":{"type":"string"},"priority":{"type":"integer","minimum":0,"maximum":10,"description":"0 silent in the Android app, 1-3 quiet, 4-7 sound, 8-10 pop up; the application's default when absent."},"markdown":{"type":"boolean","description":"Render the message as Markdown."},"click_url":{"type":"string","description":"A URL opened when the notification is tapped on Android."}},"required":["message"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_messages",
            description: "Messages from every application, newest first. Needs the connection's client token.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"limit":{"type":"integer","minimum":1,"maximum":200},"before":{"type":"integer","description":"Only messages with a smaller id, from the previous answer's next_before."}},"additionalProperties":false}"#,
        },
    ],
    events: &[],
    level: Level::Full,
    guide: &[
        Step {
            title: "Create an application",
            body: "In Gotify open Apps, choose Create Application and name it Cubebot. Copy its token: agents send with it, and their messages appear under Cubebot.",
            link: None,
            copy: Some("Cubebot"),
        },
        Step {
            title: "Optionally create a client",
            body: "To let agents read messages, open Clients, create one named Cubebot and copy its token. A client token reads every application's messages, so leave it empty if agents only need to send.",
            link: None,
            copy: None,
        },
        Step {
            title: "Enter the server's address",
            body: "Use the address you open Gotify at, such as http://gotify.local or http://192.168.1.20:8080, then paste the tokens.",
            link: None,
            copy: None,
        },
        PRIVATE_ADDRESS,
    ],
    warning: None,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Send {
    message: String,
    title: Option<String>,
    priority: Option<u8>,
    #[serde(default)]
    markdown: bool,
    click_url: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    limit: Option<u32>,
    before: Option<u64>,
}

impl Connector for Gotify {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    /// Proves the application token without sending: a message whose priority is not a number
    /// passes Gotify's token check and is then refused as a bad request.
    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let version = cx.json(api(cx, Method::GET, "version", None)?).await?;
            let version = version["version"].as_str().unwrap_or("unknown version");
            let probe = api(cx, Method::POST, "message", Some("app_token"))?
                .json(&json!({"priority": "not a number"}));
            match cx.json(probe).await {
                Err(Error::Rejected(_)) => (),
                Err(error) => return Err(error),
                Ok(_) => {
                    return Err(Error::Rejected(
                        "the server accepted a malformed message".into(),
                    ));
                }
            }
            let name = match cx.secret("client_token") {
                Ok(_) => {
                    let user = cx.json(api(cx, Method::GET, "current/user", Some("client_token"))?);
                    let user = user.await?;
                    let user = user["name"].as_str().unwrap_or("user").to_owned();
                    format!("{user} on Gotify {version}")
                }
                Err(_) => format!("Gotify {version}, sending only"),
            };
            Ok(Account { name })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "send_message" => send(cx, input).await,
                "list_messages" => list(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }
}

/// A request to `<server>/<path>` carrying the named token.
fn api(
    cx: &Context,
    method: Method,
    path: &str,
    token: Option<&str>,
) -> Result<RequestBuilder, Error> {
    let server = cx
        .setting("url")
        .ok_or_else(|| Error::Invalid("the connection has no server address".into()))?;
    let request = cx.request(method, &format!("{}/{path}", server.trim_end_matches('/')))?;
    Ok(match token {
        Some(key) => request.header("x-gotify-key", cx.secret(key)?),
        None => request,
    })
}

async fn send(cx: &Context, input: Value) -> Result<Value, Error> {
    let send: Send = crate::input(input)?;
    if send.message.trim().is_empty() {
        return Err(Error::Invalid("message is empty".into()));
    }
    let mut body = json!({"message": send.message});
    if let Some(title) = send.title {
        body["title"] = json!(title);
    }
    if let Some(priority) = send.priority {
        body["priority"] = json!(priority);
    }
    if send.markdown {
        body["extras"]["client::display"] = json!({"contentType": "text/markdown"});
    }
    if let Some(url) = send.click_url {
        body["extras"]["client::notification"] = json!({"click": {"url": url}});
    }
    let sent = cx
        .json(api(cx, Method::POST, "message", Some("app_token"))?.json(&body))
        .await?;
    Ok(json!({"id": sent["id"], "date": sent["date"]}))
}

async fn list(cx: &Context, input: Value) -> Result<Value, Error> {
    let list: List = crate::input(input)?;
    if cx.secret("client_token").is_err() {
        return Err(Error::Forbidden(
            "reading messages needs a client token; an administrator can add one to this connection".into(),
        ));
    }
    let limit = list.limit.unwrap_or(LISTED).clamp(1, MOST_LISTED);
    let mut query = vec![("limit", limit.to_string())];
    query.extend(list.before.map(|id| ("since", id.to_string())));
    let request = api(cx, Method::GET, "message", Some("client_token"))?.query(&query);
    let found = cx.json(request).await?;
    let messages: Vec<Value> = found["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|message| {
            json!({
                "id": message["id"],
                "app": message["appid"],
                "title": message["title"],
                "message": message["message"],
                "priority": message["priority"],
                "date": message["date"],
            })
        })
        .collect();
    // Gotify's `since` pages backwards: the next page holds ids below the last one shown.
    let next = found["paging"]["next"]
        .is_string()
        .then(|| messages.last().map(|message| message["id"].clone()))
        .flatten();
    Ok(json!({"messages": messages, "next_before": next}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    const APP: &str = "gtfya.-mxrJv0Tr1glsErM3scFC8sdkdJjPSPebYExxsGZ2kI";
    const CLIENT: &str = "gtfyc.7jrATKDQ4TfED5Gyn-O1Xji_d1qhczdbSjMu5muj64U";

    fn cx(fixture: &Fixture, client: bool) -> Context {
        let mut fields = vec![("app_token".to_owned(), APP.to_owned())];
        if client {
            fields.push(("client_token".into(), CLIENT.into()));
        }
        let credential = Credential::Fields {
            fields: fields.into_iter().collect(),
        };
        Context::new(json!({"url": fixture.origin()}), credential).with_origin(&fixture.origin())
    }

    /// Recorded from Gotify 3.1.1.
    fn message(id: u64) -> Value {
        json!({"id": id, "appid": 1, "message": "**hi**", "title": "t", "priority": 5, "extras": {"client::display": {"contentType": "text/markdown"}}, "date": "2026-11-06T20:12:23.155567725Z"})
    }

    #[tokio::test]
    async fn send_message_uses_the_app_token_and_extras_for_markdown_and_a_link() {
        let fixture =
            Fixture::start(vec![Exchange::new("POST", "/message", 200, message(1))]).await;
        let input = json!({"message": "**hi**", "title": "t", "priority": 5, "markdown": true, "click_url": "https://example.com"});
        let sent = Gotify
            .call(&cx(&fixture, false), "send_message", input)
            .await
            .unwrap();
        assert_eq!(
            sent,
            json!({"id": 1, "date": "2026-11-06T20:12:23.155567725Z"})
        );
        let seen = fixture.finish().await;
        assert_eq!(seen[0].header("x-gotify-key"), Some(APP));
        assert_eq!(
            seen[0].json(),
            json!({"message": "**hi**", "title": "t", "priority": 5, "extras": {"client::display": {"contentType": "text/markdown"}, "client::notification": {"click": {"url": "https://example.com"}}}})
        );
    }

    #[tokio::test]
    async fn list_messages_reads_with_the_client_token_and_pages_backwards() {
        let page = json!({"paging": {"size": 2, "since": 0, "limit": 2, "next": "http://gotify.local/message?limit=2&since=4"}, "messages": [message(5), message(4)]});
        let fixture = Fixture::start(vec![Exchange::new("GET", "/message", 200, page)]).await;
        let listed = Gotify
            .call(
                &cx(&fixture, true),
                "list_messages",
                json!({"limit": 2, "before": 6}),
            )
            .await
            .unwrap();
        assert_eq!(listed["messages"][1]["id"], 4);
        assert_eq!(listed["next_before"], 4);
        let seen = fixture.finish().await;
        assert_eq!(seen[0].target, "/message?limit=2&since=6");
        assert_eq!(seen[0].header("x-gotify-key"), Some(CLIENT));
    }

    #[tokio::test]
    async fn without_a_client_token_reading_is_refused_and_check_says_sending_only() {
        let fixture = Fixture::start(vec![
            Exchange::new("GET", "/version", 200, json!({"version": "3.1.1", "commit": "e6361ca", "buildDate": "2026-09-15-18:26:53"})),
            Exchange::new("POST", "/message", 400, json!({"error": "Bad Request", "errorCode": 400, "errorDescription": "json: cannot unmarshal string into Go struct field CreateMessage.priority of type int"})),
        ])
        .await;
        let cx = cx(&fixture, false);
        let refused = Gotify.call(&cx, "list_messages", json!({})).await;
        assert!(matches!(refused, Err(Error::Forbidden(_))), "{refused:?}");
        let account = Gotify.check(&cx).await.unwrap();
        assert_eq!(account.name, "Gotify 3.1.1, sending only");
        let seen = fixture.finish().await;
        assert_eq!(seen[1].json(), json!({"priority": "not a number"}));
    }

    #[tokio::test]
    async fn a_refused_app_token_fails_the_check_and_gotifys_reason_is_kept() {
        let unauthorized = json!({"error": "Unauthorized", "errorCode": 401, "errorDescription": "you need to provide a valid access token or user credentials to access this api"});
        let fixture = Fixture::start(vec![
            Exchange::new("GET", "/version", 200, json!({"version": "3.1.1"})),
            Exchange::new("POST", "/message", 401, unauthorized),
            Exchange::new("POST", "/message", 400, json!({"error": "Bad Request", "errorCode": 400, "errorDescription": "Field 'message' is required"})),
        ])
        .await;
        let cx = cx(&fixture, false);
        assert_eq!(Gotify.check(&cx).await, Err(Error::Unauthorized));
        let refused = Gotify
            .call(&cx, "send_message", json!({"message": "x"}))
            .await;
        assert_eq!(
            refused,
            Err(Error::Rejected("Field 'message' is required".into()))
        );
        fixture.finish().await;
    }

    /// Every tool against a real server: set `CUBEBOT_LIVE_GOTIFY_URL`,
    /// `CUBEBOT_LIVE_GOTIFY_APP_TOKEN` and `CUBEBOT_LIVE_GOTIFY_CLIENT_TOKEN`, and run
    /// `cargo test -p cubebot-connectors live_gotify -- --ignored`. It sends one message.
    #[tokio::test]
    #[ignore = "needs a Gotify server"]
    async fn live_gotify() {
        let env = |key: &str| std::env::var(format!("CUBEBOT_LIVE_GOTIFY_{key}")).unwrap();
        let fields = [("app_token".into(), env("APP_TOKEN"))];
        let sender = Context::new(
            json!({"url": env("URL")}),
            Credential::Fields {
                fields: fields.clone().into(),
            },
        );
        println!(
            "check, app token only: {:?}",
            Gotify.check(&sender).await.unwrap()
        );
        let both = fields
            .into_iter()
            .chain([("client_token".into(), env("CLIENT_TOKEN"))]);
        let cx = Context::new(
            json!({"url": env("URL")}),
            Credential::Fields {
                fields: both.collect(),
            },
        );
        println!("check, both tokens: {:?}", Gotify.check(&cx).await.unwrap());
        let text = format!("Live check {}", crate::dav::now_nanos());
        let input = json!({"message": text, "title": "Cubebot", "priority": 4, "markdown": true});
        let sent = Gotify.call(&sender, "send_message", input).await.unwrap();
        println!("send_message: {sent}");
        let listed = Gotify
            .call(&cx, "list_messages", json!({"limit": 5}))
            .await
            .unwrap();
        println!("list_messages: {listed}");
        assert_eq!(listed["messages"][0]["id"], sent["id"]);
        assert_eq!(listed["messages"][0]["message"], text);
        let refused = Gotify.call(&sender, "list_messages", json!({})).await;
        assert!(matches!(refused, Err(Error::Forbidden(_))));
    }
}

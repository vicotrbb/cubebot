//! Slack through an app the person creates in their workspace from a manifest
//! (https://api.slack.com/methods). A bot token sees the channels the bot was invited to; a user
//! token acts as the person and can search.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Event, EventSpec, Field, FieldKind,
    Level, Manifest, Poll, Risk, Step, ToolSpec,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

const API: &str = "https://slack.com/api";
/// The app's manifest; the guide's link carries it URL-encoded.
const APP: &str = r#"{"display_information":{"name":"Cubebot"},"features":{"bot_user":{"display_name":"Cubebot","always_online":true}},"oauth_config":{"scopes":{"bot":["channels:history","channels:read","groups:history","groups:read","im:history","im:read","mpim:history","chat:write","reactions:write","users:read","users:read.email"],"user":["channels:history","channels:read","groups:history","groups:read","im:history","im:read","mpim:history","chat:write","reactions:write","users:read","users:read.email","search:read"]}},"settings":{"org_deploy_enabled":false,"socket_mode_enabled":false,"token_rotation_enabled":false}}"#;
/// Authors looked up per call to name them.
const NAMES: usize = 25;
/// Direct-message conversations read per poll.
const CHATS: usize = 20;
/// The longest message text passed on.
const LONGEST: usize = 4000;
/// Slack's `ts` is ten digits, a dot and six, so such strings order as their times do.
const BEGINNING: &str = "0000000000.000000";

pub struct Slack;

static MANIFEST: Manifest = Manifest {
    id: "slack",
    name: "Slack",
    description: "Read channels and threads, search, post messages and react in a Slack workspace.",
    category: Category::Chat,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "Bot User OAuth Token (xoxb-…) or User OAuth Token (xoxp-…)",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "xoxb-…",
            help: "From the app's OAuth & Permissions page. Searching needs the User OAuth Token.",
        }],
    }],
    config: &[],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_channels",
            description: "Public and private channels the token can see, with their ids.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"limit":{"type":"integer","minimum":1,"maximum":200},"cursor":{"type":"string","description":"The next cursor from a previous answer."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_channel",
            description: "Recent messages of a channel or direct message, newest first.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"channel":{"type":"string","description":"The channel's id, such as C024BE91L."},"limit":{"type":"integer","minimum":1,"maximum":100},"cursor":{"type":"string"}},"required":["channel"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_thread",
            description: "A message and its replies, oldest first.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"channel":{"type":"string"},"ts":{"type":"string","description":"The ts of the thread's first message."},"limit":{"type":"integer","minimum":1,"maximum":100},"cursor":{"type":"string"}},"required":["channel","ts"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "search_messages",
            description: "Search messages with Slack's syntax, such as `in:#general from:@ana budget`. Needs a User OAuth Token.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["query"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_users",
            description: "People in the workspace, with their ids and emails.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"limit":{"type":"integer","minimum":1,"maximum":200},"cursor":{"type":"string"}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "find_user",
            description: "The person in the workspace with this email address.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"email":{"type":"string"}},"required":["email"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "post_message",
            description: "Post a message to a channel, a direct message or, with thread_ts, a thread.",
            risk: Risk::Send,
            product: None,
            input: r#"{"type":"object","properties":{"channel":{"type":"string","description":"A channel or conversation id, or a user id for a direct message."},"text":{"type":"string","description":"Slack mrkdwn."},"thread_ts":{"type":"string","description":"Reply in the thread of the message with this ts."}},"required":["channel","text"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "add_reaction",
            description: "React to a message with an emoji, such as thumbsup.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"channel":{"type":"string"},"ts":{"type":"string"},"name":{"type":"string","description":"The emoji's name without colons."}},"required":["channel","ts","name"],"additionalProperties":false}"#,
        },
    ],
    events: &[EventSpec {
        kind: "message.received",
        description: "A direct message to the bot, or to the person with a user token.",
        product: None,
    }],
    level: Level::Write,
    guide: &[
        Step {
            title: "Create the app from a manifest",
            body: "Open Slack's new-app page, pick the workspace and create the app; the manifest is filled in. If it is not, choose From a manifest and paste the text below.",
            link: Some(
                "https://api.slack.com/apps?new_app=1&manifest_json=%7B%22display_information%22%3A%7B%22name%22%3A%22Cubebot%22%7D%2C%22features%22%3A%7B%22bot_user%22%3A%7B%22display_name%22%3A%22Cubebot%22%2C%22always_online%22%3Atrue%7D%7D%2C%22oauth_config%22%3A%7B%22scopes%22%3A%7B%22bot%22%3A%5B%22channels%3Ahistory%22%2C%22channels%3Aread%22%2C%22groups%3Ahistory%22%2C%22groups%3Aread%22%2C%22im%3Ahistory%22%2C%22im%3Aread%22%2C%22mpim%3Ahistory%22%2C%22chat%3Awrite%22%2C%22reactions%3Awrite%22%2C%22users%3Aread%22%2C%22users%3Aread.email%22%5D%2C%22user%22%3A%5B%22channels%3Ahistory%22%2C%22channels%3Aread%22%2C%22groups%3Ahistory%22%2C%22groups%3Aread%22%2C%22im%3Ahistory%22%2C%22im%3Aread%22%2C%22mpim%3Ahistory%22%2C%22chat%3Awrite%22%2C%22reactions%3Awrite%22%2C%22users%3Aread%22%2C%22users%3Aread.email%22%2C%22search%3Aread%22%5D%7D%7D%2C%22settings%22%3A%7B%22org_deploy_enabled%22%3Afalse%2C%22socket_mode_enabled%22%3Afalse%2C%22token_rotation_enabled%22%3Afalse%7D%7D",
            ),
            copy: Some(APP),
        },
        Step {
            title: "Install it and copy a token",
            body: "Under OAuth & Permissions choose Install to Workspace and allow it. Copy the Bot User OAuth Token to act as the Cubebot bot, or the User OAuth Token to act as yourself and to search messages, and paste it below.",
            link: None,
            copy: None,
        },
        Step {
            title: "Invite the bot to channels",
            body: "A bot token reads only the channels the bot is in: in each one, send /invite @Cubebot.",
            link: None,
            copy: None,
        },
    ],
    warning: None,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    limit: Option<u16>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct History {
    channel: String,
    ts: Option<String>,
    limit: Option<u16>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: String,
    limit: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Lookup {
    email: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Post {
    channel: String,
    text: String,
    thread_ts: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct React {
    channel: String,
    ts: String,
    name: String,
}

type Params<'a> = Vec<(&'a str, String)>;

impl Connector for Slack {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let me = api(cx, "auth.test", Vec::new(), false).await?;
            let team = me["team"].as_str().unwrap_or("Slack workspace");
            let user = me["user"].as_str().unwrap_or("unknown");
            Ok(Account {
                name: format!("{team} (@{user})"),
            })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "list_channels" => channels(cx, input).await,
                "read_channel" => read(cx, input, false).await,
                "read_thread" => read(cx, input, true).await,
                "search_messages" => search(cx, input).await,
                "list_users" => users(cx, input).await,
                "find_user" => find(cx, input).await,
                "post_message" => post(cx, input).await,
                "add_reaction" => react(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(poll(cx, cursor))
    }
}

/// Every call goes through here: Slack answers most refusals with HTTP 200 and `ok: false`.
/// Reads are GETs, so one that got no answer is known to have done nothing.
async fn api(cx: &Context, method: &str, params: Params<'_>, write: bool) -> Result<Value, Error> {
    let url = format!("{API}/{method}");
    let request = match write {
        true => cx.post(&url)?.form(&params),
        false => cx.get(&url)?.query(&params),
    };
    let answer = cx.json(request.bearer_auth(cx.token()?)).await?;
    match answer["ok"].as_bool() {
        Some(true) => Ok(answer),
        _ => Err(refusal(&answer)),
    }
}

fn refusal(answer: &Value) -> Error {
    let code = word(&answer["error"]);
    match code.as_str() {
        "invalid_auth" | "not_authed" | "account_inactive" | "token_revoked" | "token_expired" => {
            Error::Unauthorized
        }
        "missing_scope" => Error::Forbidden(format!(
            "the token lacks the {} scope; add it to the app and reinstall it",
            word(&answer["needed"])
        )),
        "not_allowed_token_type" => Error::Forbidden(
            "this needs a User OAuth Token (xoxp-…); a bot token cannot do it".into(),
        ),
        "not_in_channel" => {
            Error::Forbidden("the bot is not in that channel; invite it with /invite".into())
        }
        "channel_not_found" | "user_not_found" | "users_not_found" | "thread_not_found" => {
            Error::NotFound(code)
        }
        "ratelimited" => Error::Unavailable("rate limited".into()),
        _ => Error::Rejected(code),
    }
}

/// An error code or scope from Slack, kept to the characters such names use.
fn word(value: &Value) -> String {
    let code = value.as_str().unwrap_or("unknown_error");
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric() || "_:.,".contains(*c))
        .take(80)
        .collect()
}

fn paged(mut params: Params<'_>, limit: Option<u16>, cursor: Option<String>) -> Params<'_> {
    params.push(("limit", limit.unwrap_or(50).min(200).to_string()));
    params.extend(cursor.map(|cursor| ("cursor", cursor)));
    params
}

fn next(answer: &Value) -> Value {
    let cursor = answer["response_metadata"]["next_cursor"].as_str();
    json!(cursor.filter(|cursor| !cursor.is_empty()))
}

async fn channels(cx: &Context, input: Value) -> Result<Value, Error> {
    let page: Page = crate::input(input)?;
    let params = vec![
        ("types", "public_channel,private_channel".to_owned()),
        ("exclude_archived", "true".to_owned()),
    ];
    let params = paged(params, page.limit, page.cursor);
    let found = api(cx, "conversations.list", params, false).await?;
    let channels: Vec<Value> = list(&found["channels"])
        .map(|channel| {
            json!({
                "id": channel["id"],
                "name": channel["name"],
                "private": channel["is_private"],
                "member": channel["is_member"],
                "topic": channel["topic"]["value"],
                "members": channel["num_members"],
            })
        })
        .collect();
    Ok(json!({"channels": channels, "next": next(&found)}))
}

async fn read(cx: &Context, input: Value, thread: bool) -> Result<Value, Error> {
    let history: History = crate::input(input)?;
    let mut params = vec![("channel", history.channel)];
    let method = match (thread, history.ts) {
        (true, Some(ts)) => {
            params.push(("ts", ts));
            "conversations.replies"
        }
        (true, None) => return Err(Error::Invalid("a thread needs its ts".into())),
        (false, _) => "conversations.history",
    };
    let limit = history.limit.unwrap_or(20).min(100);
    let found = api(
        cx,
        method,
        paged(params, Some(limit), history.cursor),
        false,
    )
    .await?;
    let messages: Vec<Value> = list(&found["messages"]).cloned().collect();
    let names = names(cx, &messages).await;
    let messages: Vec<Value> = messages.iter().map(|m| shown(m, &names)).collect();
    Ok(json!({"messages": messages, "next": next(&found)}))
}

async fn search(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: Search = crate::input(input)?;
    let count = search.limit.unwrap_or(20).min(100).to_string();
    let params = vec![("query", search.query), ("count", count)];
    let found = api(cx, "search.messages", params, false).await?;
    let matches: Vec<Value> = list(&found["messages"]["matches"])
        .map(|found| {
            json!({
                "channel": found["channel"]["id"],
                "channel_name": found["channel"]["name"],
                "user": found["username"],
                "ts": found["ts"],
                "text": text(found),
                "permalink": found["permalink"],
            })
        })
        .collect();
    Ok(json!({"matches": matches, "total": found["messages"]["total"]}))
}

async fn users(cx: &Context, input: Value) -> Result<Value, Error> {
    let page: Page = crate::input(input)?;
    let params = paged(Vec::new(), page.limit, page.cursor);
    let found = api(cx, "users.list", params, false).await?;
    let users: Vec<Value> = list(&found["members"])
        .filter(|user| user["deleted"] != true)
        .map(person)
        .collect();
    Ok(json!({"users": users, "next": next(&found)}))
}

async fn find(cx: &Context, input: Value) -> Result<Value, Error> {
    let lookup: Lookup = crate::input(input)?;
    let params = vec![("email", lookup.email)];
    let found = api(cx, "users.lookupByEmail", params, false).await?;
    Ok(person(&found["user"]))
}

async fn post(cx: &Context, input: Value) -> Result<Value, Error> {
    let post: Post = crate::input(input)?;
    let mut params = vec![("channel", post.channel), ("text", post.text)];
    params.extend(post.thread_ts.map(|ts| ("thread_ts", ts)));
    let posted = api(cx, "chat.postMessage", params, true).await?;
    Ok(json!({"channel": posted["channel"], "ts": posted["ts"]}))
}

async fn react(cx: &Context, input: Value) -> Result<Value, Error> {
    let react: React = crate::input(input)?;
    let name = react.name.trim_matches(':').to_owned();
    let params = vec![
        ("channel", react.channel),
        ("timestamp", react.ts),
        ("name", name),
    ];
    api(cx, "reactions.add", params, true).await?;
    Ok(json!({"reacted": true}))
}

/// Direct messages newer than the cursor, the newest `ts` seen. Without one, it only finds that
/// newest `ts`.
async fn poll(cx: &Context, cursor: Option<&str>) -> Result<Poll, Error> {
    let me = api(cx, "auth.test", Vec::new(), false).await?;
    let me = me["user_id"].as_str().unwrap_or_default().to_owned();
    let params = vec![("types", "im".to_owned()), ("limit", "100".to_owned())];
    let chats = api(cx, "conversations.list", params, false).await?;
    let chats: Vec<String> = list(&chats["channels"])
        .filter_map(|chat| chat["id"].as_str().map(str::to_owned))
        .take(CHATS)
        .collect();
    let since = cursor.unwrap_or(BEGINNING);
    let mut newest = since.to_owned();
    let mut received = Vec::new();
    for chat in chats {
        let limit = if cursor.is_some() { "50" } else { "1" };
        let params = vec![
            ("channel", chat.clone()),
            ("oldest", since.to_owned()),
            ("limit", limit.to_owned()),
        ];
        let found = api(cx, "conversations.history", params, false).await?;
        for message in list(&found["messages"]) {
            let ts = message["ts"].as_str().unwrap_or_default();
            newest = newest.max(ts.to_owned());
            // Edits, joins and the like carry a subtype; only plain messages from others count.
            let theirs = message["user"].as_str().is_some_and(|user| user != me);
            if cursor.is_some() && ts > since && theirs && message["subtype"].is_null() {
                received.push((chat.clone(), message.clone()));
            }
        }
    }
    let messages: Vec<Value> = received.iter().map(|(_, m)| m.clone()).collect();
    let names = names(cx, &messages).await;
    let events = received
        .iter()
        .map(|(chat, m)| received_event(chat, &shown(m, &names)))
        .collect();
    Ok(Poll {
        events,
        cursor: Some(newest),
    })
}

fn received_event(chat: &str, message: &Value) -> Event {
    let from = message["user"].as_str().unwrap_or("someone");
    let text = message["text"].as_str().unwrap_or_default();
    let preview: String = text.chars().take(200).collect();
    Event {
        kind: "message.received",
        id: format!("{chat}:{}", message["ts"].as_str().unwrap_or_default()),
        summary: format!("Direct message from {from}: {preview}"),
        data: json!({"channel": chat, "user": from, "user_id": message["user_id"], "ts": message["ts"], "text": text}),
    }
}

fn list(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

fn text(message: &Value) -> String {
    let text = message["text"].as_str().unwrap_or_default();
    text.chars().take(LONGEST).collect()
}

/// Names for the authors of `messages`, each looked up once and at most [`NAMES`] per call; a
/// failed lookup leaves the id.
async fn names(cx: &Context, messages: &[Value]) -> HashMap<String, String> {
    let mut ids: Vec<&str> = messages
        .iter()
        .filter_map(|message| message["user"].as_str())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let mut names = HashMap::new();
    for id in ids.into_iter().take(NAMES) {
        let params = vec![("user", id.to_owned())];
        if let Ok(found) = api(cx, "users.info", params, false).await {
            names.insert(id.to_owned(), display(&found["user"]));
        }
    }
    names
}

fn display(user: &Value) -> String {
    let names = [
        &user["profile"]["display_name"],
        &user["real_name"],
        &user["name"],
    ];
    let name = names
        .into_iter()
        .filter_map(Value::as_str)
        .find(|name| !name.is_empty());
    name.unwrap_or("unknown").to_owned()
}

fn shown(message: &Value, names: &HashMap<String, String>) -> Value {
    let id = message["user"].as_str();
    let user = id
        .and_then(|id| names.get(id).map(String::as_str))
        .or(id)
        .or(message["bot_profile"]["name"].as_str())
        .or(message["username"].as_str());
    json!({
        "ts": message["ts"],
        "user": user,
        "user_id": id,
        "text": text(message),
        "thread_ts": message["thread_ts"],
        "replies": message["reply_count"],
    })
}

fn person(user: &Value) -> Value {
    json!({
        "id": user["id"],
        "name": display(user),
        "real_name": user["real_name"],
        "email": user["profile"]["email"],
        "bot": user["is_bot"],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    fn cx(fixture: &Fixture) -> Context {
        let credential = Credential::Fields {
            fields: [("token".into(), "xoxb-secret".into())].into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    /// Example answers from Slack's method reference pages.
    fn recorded(name: &str) -> Value {
        let path = format!("{}/fixtures/slack/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn at(method: &str) -> String {
        format!("/slack.com/api/{method}")
    }

    fn form(seen: &crate::testing::Seen) -> HashMap<String, String> {
        let url = reqwest::Url::parse(&format!("http://form/?{}", seen.text())).unwrap();
        url.query_pairs().into_owned().collect()
    }

    #[test]
    fn ok_false_codes_map_to_errors() {
        let cases = [
            ("invalid_auth", Error::Unauthorized),
            ("not_authed", Error::Unauthorized),
            ("account_inactive", Error::Unauthorized),
            ("token_revoked", Error::Unauthorized),
            ("token_expired", Error::Unauthorized),
            (
                "channel_not_found",
                Error::NotFound("channel_not_found".into()),
            ),
            ("user_not_found", Error::NotFound("user_not_found".into())),
            ("ratelimited", Error::Unavailable("rate limited".into())),
            ("is_archived", Error::Rejected("is_archived".into())),
            ("<script>\n", Error::Rejected("script".into())),
        ];
        for (code, error) in cases {
            assert_eq!(
                refusal(&json!({"ok": false, "error": code})),
                error,
                "{code}"
            );
        }
        let scope =
            refusal(&json!({"ok": false, "error": "missing_scope", "needed": "chat:write"}));
        assert!(matches!(scope, Error::Forbidden(m) if m.contains("chat:write")));
        let bot = refusal(&json!({"ok": false, "error": "not_allowed_token_type"}));
        assert!(matches!(bot, Error::Forbidden(m) if m.contains("User OAuth Token")));
    }

    #[test]
    fn the_guide_link_carries_the_manifest() {
        let link = reqwest::Url::parse(MANIFEST.guide[0].link.unwrap()).unwrap();
        let (_, encoded) = link
            .query_pairs()
            .find(|(key, _)| key == "manifest_json")
            .unwrap();
        assert_eq!(encoded, APP);
        let app: Value = serde_json::from_str(APP).unwrap();
        assert_eq!(app["oauth_config"]["scopes"]["user"][11], "search:read");
    }

    #[tokio::test]
    async fn a_refusal_in_a_200_is_an_error_and_search_explains_bot_tokens() {
        let fixture = Fixture::start(vec![Exchange::new(
            "GET",
            &at("search.messages"),
            200,
            json!({"ok": false, "error": "not_allowed_token_type"}),
        )])
        .await;
        let found = Slack
            .call(&cx(&fixture), "search_messages", json!({"query": "budget"}))
            .await;
        assert!(matches!(found, Err(Error::Forbidden(m)) if m.contains("bot token")));
        let seen = fixture.finish().await;
        assert_eq!(seen[0].header("authorization"), Some("Bearer xoxb-secret"));
        assert!(seen[0].target.contains("query=budget"));
    }

    #[tokio::test]
    async fn read_channel_names_authors_and_keeps_an_id_it_cannot_name() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                &at("conversations.history"),
                200,
                recorded("history"),
            ),
            Exchange::new(
                "GET",
                &format!("{}?user=U123ABC456", at("users.info")),
                200,
                recorded("user"),
            ),
            Exchange::new(
                "GET",
                &format!("{}?user=U222BBB222", at("users.info")),
                200,
                json!({"ok": false, "error": "user_not_found"}),
            ),
        ])
        .await;
        let read = Slack
            .call(
                &cx(&fixture),
                "read_channel",
                json!({"channel": "C123ABC456", "limit": 2}),
            )
            .await
            .unwrap();
        assert_eq!(read["messages"][0]["user"], "spengler");
        assert_eq!(read["messages"][0]["user_id"], "U123ABC456");
        assert_eq!(read["messages"][1]["user"], "U222BBB222");
        assert_eq!(read["next"], "bmV4dF90czoxNTEyMDg1ODYxMDAwNTQz");
        let seen = fixture.finish().await;
        assert!(seen[0].target.ends_with("?channel=C123ABC456&limit=2"));
    }

    #[tokio::test]
    async fn post_message_sends_a_form_with_the_thread() {
        let fixture = Fixture::start(vec![Exchange::new(
            "POST",
            &at("chat.postMessage"),
            200,
            json!({"ok": true, "channel": "C123ABC456", "ts": "1503435956.000247"}),
        )])
        .await;
        let input = json!({"channel": "C123ABC456", "text": "Done & shipped", "thread_ts": "1503435956.000100"});
        let posted = Slack
            .call(&cx(&fixture), "post_message", input)
            .await
            .unwrap();
        assert_eq!(
            posted,
            json!({"channel": "C123ABC456", "ts": "1503435956.000247"})
        );
        let seen = fixture.finish().await;
        let sent = form(&seen[0]);
        assert_eq!(sent["text"], "Done & shipped");
        assert_eq!(sent["thread_ts"], "1503435956.000100");
        assert_eq!(
            seen[0].header("content-type"),
            Some("application/x-www-form-urlencoded")
        );
    }

    fn polled(history: Value) -> Vec<Exchange> {
        vec![
            Exchange::new(
                "GET",
                &at("auth.test"),
                200,
                recorded("check")["exchanges"][0]["body"].clone(),
            ),
            Exchange::new("GET", &at("conversations.list"), 200, recorded("ims")),
            Exchange::new("GET", &at("conversations.history"), 200, history),
        ]
    }

    #[tokio::test]
    async fn a_first_poll_sets_a_baseline_and_the_next_reports_new_direct_messages_but_not_its_own()
    {
        let latest = json!({"ok": true, "messages": [{"type": "message", "user": "U0BS9U4SV", "text": "old", "ts": "1512085950.000216"}]});
        let fixture = Fixture::start(polled(latest)).await;
        let baseline = Slack.poll(&cx(&fixture), None).await.unwrap();
        assert!(baseline.events.is_empty());
        assert_eq!(baseline.cursor.as_deref(), Some("1512085950.000216"));
        let seen = fixture.finish().await;
        assert!(seen[2].target.contains("limit=1"));

        let newer = json!({"ok": true, "messages": [
            {"type": "message", "user": "W12345678", "text": "my own reply", "ts": "1512104500.000100"},
            {"type": "message", "user": "U0BS9U4SV", "text": "Can you send the report?", "ts": "1512104434.000490"},
            {"type": "message", "subtype": "message_changed", "ts": "1512104440.000001"}
        ]});
        let mut exchanges = polled(newer);
        exchanges.push(Exchange::new(
            "GET",
            &at("users.info"),
            200,
            recorded("user"),
        ));
        let fixture = Fixture::start(exchanges).await;
        let cursor = baseline.cursor.as_deref();
        let next = Slack.poll(&cx(&fixture), cursor).await.unwrap();
        assert_eq!(next.cursor.as_deref(), Some("1512104500.000100"));
        assert_eq!(next.events.len(), 1, "{:?}", next.events);
        let event = &next.events[0];
        assert_eq!(event.id, "D0C0F7S8Y:1512104434.000490");
        assert_eq!(
            event.summary,
            "Direct message from spengler: Can you send the report?"
        );
        assert_eq!(event.data["user_id"], "U0BS9U4SV");
        let seen = fixture.finish().await;
        assert!(seen[2].target.contains("oldest=1512085950.000216"));
    }
}

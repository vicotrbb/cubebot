//! A Discord bot the person creates in their own application (REST API v10,
//! https://docs.discord.com/developers/reference). It sees the servers it was invited to; without
//! the Message Content intent it reads other people's words only where it is mentioned
//! (https://docs.discord.com/developers/events/gateway#message-content-intent). Setup follows
//! https://docs.discord.com/developers/quick-start/getting-started.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Event, EventSpec, Field, FieldKind,
    Level, Manifest, Poll, Risk, Step, ToolSpec,
};
use reqwest::{Method, RequestBuilder, header::AUTHORIZATION};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const API: &str = "https://discord.com/api/v10";
/// Discord's limit for one message.
const LONGEST_MESSAGE: usize = 2000;
/// The most characters of a message's text the model gets.
const LONGEST: usize = 4000;
/// Servers whose channels one call lists.
const SERVERS: usize = 10;
/// Watched channels read per poll, keeping each poll's work bounded.
const CHANNELS: usize = 20;

pub struct Discord;

static MANIFEST: Manifest = Manifest {
    id: "discord",
    name: "Discord bot",
    description: "Read and post in Discord servers through your own bot, and hear when it is mentioned.",
    category: Category::Chat,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "Bot token",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "MTA…",
            help: "From the application's Bot page, under Token.",
        }],
    }],
    config: &[Field {
        key: "channels",
        label: "Watched channel ids",
        kind: FieldKind::Lines,
        required: false,
        placeholder: "1234567890123456789",
        help: "One channel id per line; a message there that mentions the bot becomes an event. The first 20 are watched.",
    }],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_channels",
            description: "The servers the bot is in and their text channels, with ids.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"server":{"type":"string","description":"Only this server's id."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_channel",
            description: "Recent messages of a channel, newest first. Page with before or after a message id.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"channel":{"type":"string","description":"The channel's id."},"limit":{"type":"integer","minimum":1,"maximum":100},"before":{"type":"string","description":"Only messages older than this message id."},"after":{"type":"string","description":"Only messages newer than this message id."}},"required":["channel"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "send_message",
            description: "Post a message of up to 2000 characters to a channel. It can mention people but not roles or @everyone.",
            risk: Risk::Send,
            product: None,
            input: r#"{"type":"object","properties":{"channel":{"type":"string"},"text":{"type":"string","maxLength":2000}},"required":["channel","text"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "reply_to_message",
            description: "Reply to a message in its channel, quoting it.",
            risk: Risk::Send,
            product: None,
            input: r#"{"type":"object","properties":{"channel":{"type":"string"},"message":{"type":"string","description":"The id of the message to reply to."},"text":{"type":"string","maxLength":2000}},"required":["channel","message","text"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "add_reaction",
            description: "React to a message with an emoji, such as 👍, or a custom one as name:id.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"channel":{"type":"string"},"message":{"type":"string"},"emoji":{"type":"string"}},"required":["channel","message","emoji"],"additionalProperties":false}"#,
        },
    ],
    events: &[EventSpec {
        kind: "message.mentioned",
        description: "Someone mentioned the bot in a watched channel.",
        product: None,
    }],
    level: Level::Full,
    guide: &[
        Step {
            title: "Create an application",
            body: "In the Discord Developer Portal choose New Application, name it and create it.",
            link: Some("https://discord.com/developers/applications?new_application=true"),
            copy: None,
        },
        Step {
            title: "Copy the bot token",
            body: "Open the Bot page, choose Reset Token under Token, and paste the token below. Discord shows it once.",
            link: None,
            copy: None,
        },
        Step {
            title: "Allow reading messages",
            body: "On the same Bot page, under Privileged Gateway Intents, switch on Message Content Intent and save. Without it the bot still sees messages that mention it, but read_channel shows other messages as empty.",
            link: None,
            copy: None,
        },
        Step {
            title: "Invite the bot to your server",
            body: "On the Installation page, under Default Install Settings for Guild Install, add the bot scope and the permissions View Channels, Send Messages, Read Message History and Add Reactions (the permissions number below). Open the Install Link, choose Add to server and pick your server.",
            link: None,
            copy: Some("68672"),
        },
        Step {
            title: "Choose channels to watch",
            body: "To hear mentions, switch on Developer Mode under User Settings › Advanced in Discord, right-click each channel, choose Copy Channel ID, and list the ids below.",
            link: None,
            copy: None,
        },
    ],
    warning: None,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Servers {
    server: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct History {
    channel: String,
    limit: Option<u8>,
    before: Option<String>,
    after: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Post {
    channel: String,
    text: String,
    /// Only for replies.
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct React {
    channel: String,
    message: String,
    emoji: String,
}

impl Connector for Discord {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let me = api(cx, Method::GET, "/users/@me")?;
            let me = cx.json(me).await?;
            let name = me["username"].as_str().unwrap_or("Discord bot");
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "list_channels" => channels(cx, input).await,
                "read_channel" => read(cx, input).await,
                "send_message" => post(cx, input, false).await,
                "reply_to_message" => post(cx, input, true).await,
                "add_reaction" => react(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(poll(cx, cursor))
    }
}

fn api(cx: &Context, method: Method, path: &str) -> Result<RequestBuilder, Error> {
    let token = format!("Bot {}", cx.token()?);
    Ok(cx
        .request(method, &format!("{API}{path}"))?
        .header(AUTHORIZATION, token))
}

/// A snowflake id: digits only, so it can sit in a path.
fn id<'a>(what: &str, text: &'a str) -> Result<&'a str, Error> {
    let valid = (1..=20).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit());
    valid
        .then_some(text)
        .ok_or_else(|| Error::Invalid(format!("{what} must be a Discord id, digits only")))
}

fn list(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

async fn channels(cx: &Context, input: Value) -> Result<Value, Error> {
    let servers: Servers = crate::input(input)?;
    let (servers, more) = match servers.server {
        Some(server) => (vec![json!({"id": id("server", &server)?})], false),
        None => {
            let found = cx.json(api(cx, Method::GET, "/users/@me/guilds")?).await?;
            let all: Vec<Value> = list(&found).cloned().collect();
            (
                all.iter().take(SERVERS).cloned().collect(),
                all.len() > SERVERS,
            )
        }
    };
    let mut shown = Vec::new();
    for server in servers {
        let server_id = server["id"].as_str().unwrap_or_default();
        let path = format!("/guilds/{server_id}/channels");
        let found = cx.json(api(cx, Method::GET, &path)?).await?;
        // Text (0) and announcement (5) channels hold messages the bot can read.
        let channels: Vec<Value> = list(&found)
            .filter(|channel| matches!(channel["type"].as_u64(), Some(0 | 5)))
            .map(|channel| json!({"id": channel["id"], "name": channel["name"], "topic": channel["topic"]}))
            .collect();
        shown.push(json!({"id": server_id, "name": server["name"], "channels": channels}));
    }
    Ok(json!({"servers": shown, "more": more}))
}

async fn read(cx: &Context, input: Value) -> Result<Value, Error> {
    let history: History = crate::input(input)?;
    let channel = id("channel", &history.channel)?;
    let limit = history.limit.unwrap_or(25).clamp(1, 100);
    let mut query = vec![("limit", limit.to_string())];
    match (&history.before, &history.after) {
        (Some(_), Some(_)) => return Err(Error::Invalid("give before or after, not both".into())),
        (Some(before), None) => query.push(("before", id("before", before)?.to_owned())),
        (None, Some(after)) => query.push(("after", id("after", after)?.to_owned())),
        (None, None) => (),
    }
    let path = format!("/channels/{channel}/messages");
    let found = cx.json(api(cx, Method::GET, &path)?.query(&query)).await?;
    let messages: Vec<Value> = list(&found).map(message).collect();
    Ok(json!({"messages": messages}))
}

/// A message as the model needs it; its text is untrusted.
fn message(message: &Value) -> Value {
    let content = message["content"].as_str().unwrap_or_default();
    let attachments: Vec<Value> = list(&message["attachments"])
        .map(|file| json!({"name": file["filename"], "url": file["url"]}))
        .collect();
    json!({
        "id": message["id"],
        "author": message["author"]["username"],
        "author_id": message["author"]["id"],
        "bot": message["author"]["bot"].as_bool().unwrap_or(false),
        "text": content.chars().take(LONGEST).collect::<String>(),
        "time": message["timestamp"],
        "reply_to": message["message_reference"]["message_id"],
        "attachments": attachments,
    })
}

async fn post(cx: &Context, input: Value, reply: bool) -> Result<Value, Error> {
    let post: Post = crate::input(input)?;
    let channel = id("channel", &post.channel)?;
    if post.text.trim().is_empty() {
        return Err(Error::Invalid("text is empty".into()));
    }
    if post.text.chars().count() > LONGEST_MESSAGE {
        return Err(Error::Invalid(format!(
            "text is longer than Discord's {LONGEST_MESSAGE} characters; send it in parts"
        )));
    }
    // People can be mentioned, but never roles or @everyone, which would reach a whole server.
    let mut body = json!({"content": post.text, "allowed_mentions": {"parse": ["users"]}});
    match (reply, &post.message) {
        (true, Some(message)) => {
            body["message_reference"] = json!({"message_id": id("message", message)?});
        }
        (true, None) => return Err(Error::Invalid("message is required".into())),
        (false, Some(_)) => {
            return Err(Error::Invalid(
                "unknown field `message`; use reply_to_message".into(),
            ));
        }
        (false, None) => (),
    }
    let path = format!("/channels/{channel}/messages");
    let sent = cx.json(api(cx, Method::POST, &path)?.json(&body)).await?;
    Ok(json!({"id": sent["id"], "channel": sent["channel_id"]}))
}

async fn react(cx: &Context, input: Value) -> Result<Value, Error> {
    let react: React = crate::input(input)?;
    let (channel, message) = (
        id("channel", &react.channel)?,
        id("message", &react.message)?,
    );
    let emoji = react
        .emoji
        .trim()
        .trim_start_matches(':')
        .trim_end_matches(':');
    if emoji.is_empty() {
        return Err(Error::Invalid("emoji is empty".into()));
    }
    let path = format!(
        "/channels/{channel}/messages/{message}/reactions/{}/@me",
        encoded(emoji)
    );
    cx.json(api(cx, Method::PUT, &path)?).await?;
    Ok(json!({"reacted": true}))
}

/// Percent-encodes everything but unreserved characters, as Discord wants emoji in paths.
fn encoded(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The newer of two snowflakes, which order as numbers.
fn newer(a: &str, b: &str) -> bool {
    a.parse::<u64>().unwrap_or(0) > b.parse::<u64>().unwrap_or(0)
}

/// The cursor maps each watched channel to the newest message id seen there. A channel missing
/// from it, such as one just added, only records its newest message.
async fn poll(cx: &Context, cursor: Option<&str>) -> Result<Poll, Error> {
    let before: Map<String, Value> = cursor
        .and_then(|cursor| serde_json::from_str(cursor).ok())
        .unwrap_or_default();
    let watched: Vec<&str> = list(&cx.config()["channels"])
        .filter_map(Value::as_str)
        .filter(|channel| id("channel", channel).is_ok())
        .take(CHANNELS)
        .collect();
    let me = cx.json(api(cx, Method::GET, "/users/@me")?).await?;
    let me = me["id"].as_str().unwrap_or_default().to_owned();
    let mut after = Map::new();
    let mut events = Vec::new();
    for channel in watched {
        let since = before.get(channel).and_then(Value::as_str);
        let query = match since {
            Some(since) => vec![("after", since), ("limit", "100")],
            None => vec![("limit", "1")],
        };
        let path = format!("/channels/{channel}/messages");
        // One unreadable channel, such as one the bot left, must not silence the others.
        let found = match cx.json(api(cx, Method::GET, &path)?.query(&query)).await {
            Ok(found) => found,
            Err(Error::Unauthorized) => return Err(Error::Unauthorized),
            Err(error) => {
                tracing::warn!(%error, "a watched Discord channel could not be read");
                after.extend(since.map(|since| (channel.to_owned(), json!(since))));
                continue;
            }
        };
        let mut newest = since.unwrap_or("0").to_owned();
        for message in list(&found) {
            let id = message["id"].as_str().unwrap_or_default();
            if newer(id, &newest) {
                newest = id.to_owned();
            }
            if since.is_some_and(|since| newer(id, since)) && mentions(message, &me) {
                events.push(mentioned(channel, message));
            }
        }
        after.insert(channel.to_owned(), json!(newest));
    }
    // Discord answers newest first; events read better oldest first.
    events.reverse();
    Ok(Poll {
        events,
        cursor: Some(Value::Object(after).to_string()),
    })
}

fn mentions(message: &Value, me: &str) -> bool {
    let own = message["author"]["id"] == me;
    !own && list(&message["mentions"]).any(|user| user["id"] == me)
}

fn mentioned(channel: &str, found: &Value) -> Event {
    let shown = message(found);
    let author = shown["author"].as_str().unwrap_or("someone");
    let preview: String = shown["text"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect();
    Event {
        kind: "message.mentioned",
        id: format!("{channel}:{}", found["id"].as_str().unwrap_or_default()),
        summary: format!("{author} mentioned the bot: {preview}"),
        data: json!({"channel": channel, "message": shown}),
    }
}

#[cfg(test)]
mod tests {
    //! Answers are trimmed from the examples in Discord's API reference.
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    const TOKEN: &str = "MTA5ODc2NTQzMjEwOTg3NjU0Mw.GaBcDe.test-token";
    const BOT: &str = "1098765432109876543";
    const CHANNEL: &str = "1100000000000000001";

    fn cx(fixture: &Fixture, config: Value) -> Context {
        let credential = Credential::Fields {
            fields: [("token".into(), TOKEN.into())].into(),
        };
        Context::new(config, credential).with_origin(&fixture.origin())
    }

    fn at(path: &str) -> String {
        format!("/discord.com/api/v10{path}")
    }

    fn me() -> Exchange {
        Exchange::new(
            "GET",
            &at("/users/@me"),
            200,
            json!({"id": BOT, "username": "cubebot", "discriminator": "0", "bot": true}),
        )
    }

    fn sent(id: &str, author: &str, text: &str, mentions: &[&str]) -> Value {
        let mentions: Vec<Value> = mentions
            .iter()
            .map(|id| json!({"id": id, "username": "cubebot", "bot": true}))
            .collect();
        json!({
            "id": id, "channel_id": CHANNEL, "type": 0, "content": text,
            "author": {"id": author, "username": if author == BOT { "cubebot" } else { "ana" }},
            "timestamp": "2026-11-05T12:00:00.000000+00:00", "mentions": mentions,
            "attachments": [], "embeds": [], "pinned": false, "tts": false, "mention_everyone": false,
        })
    }

    #[tokio::test]
    async fn list_channels_shows_each_servers_text_channels() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                &at("/users/@me/guilds"),
                200,
                json!([{"id": "80351110224678912", "name": "Home lab", "icon": null, "owner": false, "permissions": "68672", "features": []}]),
            ),
            Exchange::new(
                "GET",
                &at("/guilds/80351110224678912/channels"),
                200,
                json!([
                    {"id": "41771983423143937", "type": 0, "guild_id": "80351110224678912", "name": "general", "topic": "Say hi", "position": 0},
                    {"id": "41771983423143938", "type": 2, "guild_id": "80351110224678912", "name": "Voice", "position": 1},
                    {"id": "41771983423143939", "type": 4, "guild_id": "80351110224678912", "name": "Text Channels", "position": 0}
                ]),
            ),
        ])
        .await;
        let listed = Discord
            .call(&cx(&fixture, json!({})), "list_channels", json!({}))
            .await
            .unwrap();
        assert_eq!(
            listed,
            json!({"servers": [{"id": "80351110224678912", "name": "Home lab", "channels": [{"id": "41771983423143937", "name": "general", "topic": "Say hi"}]}], "more": false})
        );
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].header("authorization"),
            Some(&*format!("Bot {TOKEN}"))
        );
    }

    #[tokio::test]
    async fn read_channel_pages_before_a_message_and_shapes_messages() {
        let mut reply = sent(
            "1100000000000000012",
            "53908232506183680",
            "see attached",
            &[],
        );
        reply["message_reference"] =
            json!({"message_id": "1100000000000000011", "channel_id": CHANNEL});
        reply["attachments"] = json!([{"id": "1", "filename": "plan.pdf", "size": 1024, "url": "https://cdn.discordapp.com/attachments/1/plan.pdf", "proxy_url": "https://media.discordapp.net/attachments/1/plan.pdf"}]);
        let fixture = Fixture::start(vec![Exchange::new(
            "GET",
            &at(&format!(
                "/channels/{CHANNEL}/messages?limit=2&before=1100000000000000020"
            )),
            200,
            json!([reply]),
        )])
        .await;
        let input = json!({"channel": CHANNEL, "limit": 2, "before": "1100000000000000020"});
        let read = Discord
            .call(&cx(&fixture, json!({})), "read_channel", input)
            .await
            .unwrap();
        assert_eq!(
            read["messages"][0],
            json!({
                "id": "1100000000000000012", "author": "ana", "author_id": "53908232506183680",
                "bot": false, "text": "see attached", "time": "2026-11-05T12:00:00.000000+00:00",
                "reply_to": "1100000000000000011",
                "attachments": [{"name": "plan.pdf", "url": "https://cdn.discordapp.com/attachments/1/plan.pdf"}],
            })
        );
        fixture.finish().await;
    }

    #[tokio::test]
    async fn sending_and_replying_never_ping_roles_or_everyone() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "POST",
                &at(&format!("/channels/{CHANNEL}/messages")),
                200,
                sent("1100000000000000030", BOT, "Done.", &[]),
            ),
            Exchange::new(
                "POST",
                &at(&format!("/channels/{CHANNEL}/messages")),
                200,
                sent("1100000000000000031", BOT, "On it.", &[]),
            ),
        ])
        .await;
        let cx = cx(&fixture, json!({}));
        let posted = Discord
            .call(
                &cx,
                "send_message",
                json!({"channel": CHANNEL, "text": "Done."}),
            )
            .await
            .unwrap();
        assert_eq!(
            posted,
            json!({"id": "1100000000000000030", "channel": CHANNEL})
        );
        let input = json!({"channel": CHANNEL, "message": "1100000000000000012", "text": "On it."});
        Discord.call(&cx, "reply_to_message", input).await.unwrap();
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].json(),
            json!({"content": "Done.", "allowed_mentions": {"parse": ["users"]}})
        );
        assert_eq!(
            seen[1].json()["message_reference"],
            json!({"message_id": "1100000000000000012"})
        );
    }

    #[tokio::test]
    async fn add_reaction_encodes_the_emoji_in_the_path() {
        let fixture = Fixture::start(vec![Exchange::new(
            "PUT",
            &at(&format!(
                "/channels/{CHANNEL}/messages/1100000000000000012/reactions/%F0%9F%91%8D/@me"
            )),
            204,
            Value::Null,
        )])
        .await;
        let input = json!({"channel": CHANNEL, "message": "1100000000000000012", "emoji": "👍"});
        let reacted = Discord
            .call(&cx(&fixture, json!({})), "add_reaction", input)
            .await
            .unwrap();
        assert_eq!(reacted, json!({"reacted": true}));
        fixture.finish().await;
        assert_eq!(
            encoded("party:41771983429993937"),
            "party%3A41771983429993937"
        );
    }

    #[tokio::test]
    async fn bad_input_is_refused_before_reaching_discord() {
        let fixture = Fixture::start(Vec::new()).await;
        let cx = cx(&fixture, json!({}));
        let long = "a".repeat(LONGEST_MESSAGE + 1);
        let refused = [
            ("read_channel", json!({"channel": "../users/@me"})),
            (
                "read_channel",
                json!({"channel": CHANNEL, "before": "1", "after": "2"}),
            ),
            ("read_channel", json!({"channel": CHANNEL, "around": "1"})),
            ("send_message", json!({"channel": CHANNEL, "text": long})),
            ("send_message", json!({"channel": CHANNEL, "text": " "})),
            (
                "send_message",
                json!({"channel": CHANNEL, "text": "hi", "message": "1"}),
            ),
            (
                "reply_to_message",
                json!({"channel": CHANNEL, "text": "hi"}),
            ),
            (
                "add_reaction",
                json!({"channel": CHANNEL, "message": "1", "emoji": "::"}),
            ),
            ("list_channels", json!({"server": "home lab"})),
        ];
        for (tool, input) in refused {
            let answer = Discord.call(&cx, tool, input.clone()).await;
            assert!(
                matches!(answer, Err(Error::Invalid(_))),
                "{tool} {input}: {answer:?}"
            );
        }
        assert!(fixture.finish().await.is_empty());
    }

    #[tokio::test]
    async fn refusals_map_to_errors() {
        let cases = [
            (
                401,
                json!({"message": "401: Unauthorized", "code": 0}),
                Error::Unauthorized,
            ),
            (
                403,
                json!({"message": "Missing Access", "code": 50001}),
                Error::Forbidden("Missing Access".into()),
            ),
            (
                404,
                json!({"message": "Unknown Channel", "code": 10003}),
                Error::NotFound("Unknown Channel".into()),
            ),
            (
                429,
                json!({"message": "You are being rate limited.", "retry_after": 3600.0, "global": false}),
                Error::Unavailable("rate limited: You are being rate limited.".into()),
            ),
            (
                400,
                json!({"message": "Invalid Form Body", "code": 50035}),
                Error::Rejected("Invalid Form Body".into()),
            ),
        ];
        for (status, body, expected) in cases {
            let answer = Exchange::new(
                "GET",
                &at(&format!("/channels/{CHANNEL}/messages")),
                status,
                body,
            )
            .header("retry-after", "3600");
            let fixture = Fixture::start(vec![answer]).await;
            let read = Discord
                .call(
                    &cx(&fixture, json!({})),
                    "read_channel",
                    json!({"channel": CHANNEL}),
                )
                .await;
            assert_eq!(read, Err(expected), "{status}");
            fixture.finish().await;
        }
    }

    #[tokio::test]
    async fn poll_records_a_baseline_then_reports_mentions_once_but_not_its_own() {
        let messages = format!("/channels/{CHANNEL}/messages");
        let fixture = Fixture::start(vec![
            me(),
            Exchange::new(
                "GET",
                &at(&format!("{messages}?limit=1")),
                200,
                json!([sent(
                    "1100000000000000010",
                    "53908232506183680",
                    "old <@1098765432109876543>",
                    &[BOT]
                )]),
            ),
            me(),
            Exchange::new(
                "GET",
                &at(&format!("{messages}?after=1100000000000000010&limit=100")),
                200,
                json!([
                    sent(
                        "1100000000000000014",
                        BOT,
                        "<@1098765432109876543> talking to myself",
                        &[BOT]
                    ),
                    sent(
                        "1100000000000000013",
                        "53908232506183680",
                        "<@1098765432109876543> can you check the build?",
                        &[BOT]
                    ),
                    sent("1100000000000000012", "53908232506183680", "lunch?", &[]),
                ]),
            ),
            me(),
            Exchange::new(
                "GET",
                &at(&format!("{messages}?after=1100000000000000014&limit=100")),
                200,
                json!([]),
            ),
        ])
        .await;
        let cx = cx(&fixture, json!({"channels": [CHANNEL, "not-an-id"]}));
        let baseline = Discord.poll(&cx, None).await.unwrap();
        assert!(baseline.events.is_empty());
        let cursor = baseline.cursor.unwrap();
        assert_eq!(cursor, json!({CHANNEL: "1100000000000000010"}).to_string());
        let next = Discord.poll(&cx, Some(&cursor)).await.unwrap();
        assert_eq!(next.events.len(), 1, "{:?}", next.events);
        let event = &next.events[0];
        assert_eq!(event.kind, "message.mentioned");
        assert_eq!(event.id, format!("{CHANNEL}:1100000000000000013"));
        assert_eq!(
            event.summary,
            "ana mentioned the bot: <@1098765432109876543> can you check the build?"
        );
        assert_eq!(event.data["channel"], CHANNEL);
        let again = Discord.poll(&cx, next.cursor.as_deref()).await.unwrap();
        assert!(again.events.is_empty());
        assert_eq!(again.cursor, next.cursor);
        fixture.finish().await;
    }

    #[tokio::test]
    async fn an_unreadable_channel_keeps_its_place_and_a_refused_token_fails_the_poll() {
        let fixture = Fixture::start(vec![
            me(),
            Exchange::new(
                "GET",
                &at(&format!("/channels/{CHANNEL}/messages")),
                403,
                json!({"message": "Missing Access", "code": 50001}),
            ),
            Exchange::new(
                "GET",
                &at("/users/@me"),
                401,
                json!({"message": "401: Unauthorized", "code": 0}),
            ),
        ])
        .await;
        let cx = cx(&fixture, json!({"channels": [CHANNEL]}));
        let cursor = json!({CHANNEL: "1100000000000000010"}).to_string();
        let polled = Discord.poll(&cx, Some(&cursor)).await.unwrap();
        assert_eq!(polled.cursor, Some(cursor.clone()));
        assert_eq!(
            Discord.poll(&cx, Some(&cursor)).await.unwrap_err(),
            Error::Unauthorized
        );
        fixture.finish().await;
    }
}

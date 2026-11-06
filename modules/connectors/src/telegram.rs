//! A Telegram bot (https://core.telegram.org/bots/api), so a person can reach their agents from
//! their phone. The token sits in every request's path, so no URL reaches an error or a result.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Event, EventSpec, Field, FieldKind,
    Level, Manifest, Poll, Risk, Step, ToolSpec,
};
use serde::Deserialize;
use serde_json::{Value, json};

const API: &str = "https://api.telegram.org";
/// Telegram's limit for one message, counted in UTF-16 code units.
const LONGEST_MESSAGE: usize = 4096;

pub struct Telegram;

static MANIFEST: Manifest = Manifest {
    id: "telegram",
    name: "Telegram bot",
    description: "Message people through your own Telegram bot and hear what they send it.",
    category: Category::Chat,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "Bot token",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "123456789:AA…",
            help: "The token @BotFather gave you.",
        }],
    }],
    config: &[],
    products: &[],
    tools: &[
        ToolSpec {
            name: "send_message",
            description: "Send a text message to a chat; long text is split into several messages.",
            risk: Risk::Send,
            product: None,
            input: r#"{"type":"object","properties":{"chat_id":{"type":["integer","string"],"description":"The chat's id, or @channelusername."},"text":{"type":"string"},"reply_to_message_id":{"type":"integer"}},"required":["chat_id","text"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "get_chat",
            description: "A chat's type, title and username.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"chat_id":{"type":["integer","string"]}},"required":["chat_id"],"additionalProperties":false}"#,
        },
    ],
    events: &[EventSpec {
        kind: "message.received",
        description: "Someone sent the bot a message.",
        product: None,
    }],
    level: Level::Full,
    guide: &[
        Step {
            title: "Create a bot",
            body: "Open @BotFather in Telegram, send /newbot and choose a name and a username ending in bot.",
            link: Some("https://t.me/BotFather"),
            copy: Some("/newbot"),
        },
        Step {
            title: "Copy its token",
            body: "BotFather answers with a token such as 123456789:AA…; paste it below.",
            link: None,
            copy: None,
        },
        Step {
            title: "Message your bot",
            body: "Open your bot and send it a message. Bots cannot write to someone first, so it can reply only to people who have messaged it.",
            link: None,
            copy: None,
        },
    ],
    warning: None,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Send {
    chat_id: Value,
    text: String,
    reply_to_message_id: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Chat {
    chat_id: Value,
}

impl Connector for Telegram {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let me = method(cx, "getMe", json!({})).await?;
            let name = me["username"].as_str().unwrap_or("bot");
            Ok(Account {
                name: format!("@{name}"),
            })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "send_message" => send(cx, input).await,
                "get_chat" => chat(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(async move {
            // A negative offset answers only the newest update and forgets the older ones.
            let offset = cursor.and_then(|cursor| cursor.parse::<i64>().ok());
            let body = json!({"offset": offset.unwrap_or(-1), "timeout": 0, "allowed_updates": ["message"]});
            let updates = method(cx, "getUpdates", body).await?;
            let updates = updates.as_array().cloned().unwrap_or_default();
            let next = updates
                .iter()
                .filter_map(|update| update["update_id"].as_i64())
                .max()
                .map_or(offset.unwrap_or(0), |newest| newest + 1);
            let events = match offset {
                Some(_) => updates.iter().filter_map(event).collect(),
                None => Vec::new(),
            };
            Ok(Poll {
                events,
                cursor: Some(next.to_string()),
            })
        })
    }
}

/// Calls a Bot API method and answers its `result`. A malformed token gets a 404, which is a
/// refused credential too.
async fn method(cx: &Context, name: &str, body: Value) -> Result<Value, Error> {
    let url = format!("{API}/bot{}/{name}", cx.secret("token")?);
    let answer = cx.json(cx.post(&url)?.json(&body)).await;
    let answer = answer.map_err(|error| redacted(cx, error))?;
    if answer["ok"] != true {
        let reason = answer["description"].as_str().unwrap_or("no reason given");
        return Err(Error::Rejected(cx.credential().redact(reason)));
    }
    Ok(answer["result"].clone())
}

/// Vendor words with the token struck, in case Telegram echoes it.
fn redacted(cx: &Context, error: Error) -> Error {
    let strike = |message: String| cx.credential().redact(&message);
    match error {
        Error::NotFound(_) => Error::Unauthorized,
        Error::Forbidden(message) => Error::Forbidden(strike(message)),
        Error::Rejected(message) => Error::Rejected(strike(message)),
        Error::Unavailable(message) => Error::Unavailable(strike(message)),
        other => other,
    }
}

/// Pieces within Telegram's limit, broken after a newline when there is one.
fn pieces(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let mut units = 0;
        let end = rest
            .char_indices()
            .take_while(|(_, c)| {
                units += c.len_utf16();
                units <= LONGEST_MESSAGE
            })
            .last()
            .map_or(rest.len(), |(at, c)| at + c.len_utf8());
        let end = match end < rest.len() {
            true => rest[..end].rfind('\n').map_or(end, |newline| newline + 1),
            false => end,
        };
        found.push(&rest[..end]);
        rest = &rest[end..];
    }
    found
}

async fn send(cx: &Context, input: Value) -> Result<Value, Error> {
    let send: Send = crate::input(input)?;
    if send.text.trim().is_empty() {
        return Err(Error::Invalid("text is empty".into()));
    }
    let pieces = pieces(&send.text);
    let mut sent = Vec::new();
    for (index, piece) in pieces.iter().enumerate() {
        let mut body = json!({"chat_id": send.chat_id, "text": piece});
        if let (0, Some(id)) = (index, send.reply_to_message_id) {
            body["reply_parameters"] = json!({"message_id": id});
        }
        match method(cx, "sendMessage", body).await {
            Ok(message) => sent.push(message["message_id"].clone()),
            // Say what already went out, so a retry does not repeat it.
            Err(error) if !sent.is_empty() => {
                let total = pieces.len();
                let done = sent.len();
                return Err(Error::Rejected(format!(
                    "sent {done} of {total} parts, then: {error}"
                )));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(json!({"message_ids": sent}))
}

async fn chat(cx: &Context, input: Value) -> Result<Value, Error> {
    let chat: Chat = crate::input(input)?;
    let found = method(cx, "getChat", json!({"chat_id": chat.chat_id})).await?;
    Ok(json!({
        "id": found["id"],
        "type": found["type"],
        "title": found["title"],
        "username": found["username"],
        "first_name": found["first_name"],
    }))
}

/// A message update as an event; its text is untrusted.
fn event(update: &Value) -> Option<Event> {
    let message = update.get("message")?;
    let text = message["text"].as_str().or(message["caption"].as_str());
    let text: String = text
        .unwrap_or_default()
        .chars()
        .take(LONGEST_MESSAGE)
        .collect();
    let from = message["from"]["first_name"].as_str().unwrap_or("someone");
    Some(Event {
        kind: "message.received",
        id: update["update_id"].to_string(),
        summary: format!("Telegram message from {from}"),
        data: json!({
            "chat": {"id": message["chat"]["id"], "type": message["chat"]["type"], "title": message["chat"]["title"]},
            "from": {"username": message["from"]["username"], "first_name": message["from"]["first_name"]},
            "message_id": message["message_id"],
            "text": text,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    const TOKEN: &str = "123456:TEST-token";

    fn cx(fixture: &Fixture) -> Context {
        let credential = Credential::Fields {
            fields: [("token".into(), TOKEN.into())].into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    fn bot(name: &str, status: u16, body: Value) -> Exchange {
        Exchange::new(
            "POST",
            &format!("/api.telegram.org/bot{TOKEN}/{name}"),
            status,
            body,
        )
    }

    /// Shapes from the Bot API reference's Message and Update objects.
    fn update(id: i64, text: &str) -> Value {
        json!({"update_id": id, "message": {
            "message_id": id - 100,
            "from": {"id": 42, "is_bot": false, "first_name": "Ana", "username": "ana"},
            "chat": {"id": 42, "first_name": "Ana", "username": "ana", "type": "private"},
            "date": 1762336800,
            "text": text,
        }})
    }

    #[tokio::test]
    async fn send_message_splits_long_text_and_replies_with_the_first_piece() {
        let sent = |id: i64| {
            bot(
                "sendMessage",
                200,
                json!({"ok": true, "result": {"message_id": id, "chat": {"id": 42, "type": "private"}, "date": 1762336800, "text": "…"}}),
            )
        };
        let fixture = Fixture::start(vec![sent(7), sent(8)]).await;
        let text = format!("{}\n{}", "a".repeat(3000), "b".repeat(3000));
        let input = json!({"chat_id": 42, "text": text, "reply_to_message_id": 5});
        let answer = Telegram
            .call(&cx(&fixture), "send_message", input)
            .await
            .unwrap();
        assert_eq!(answer, json!({"message_ids": [7, 8]}));
        let seen = fixture.finish().await;
        let (first, second) = (seen[0].json(), seen[1].json());
        assert_eq!(first["text"], format!("{}\n", "a".repeat(3000)));
        assert_eq!(first["reply_parameters"], json!({"message_id": 5}));
        assert_eq!(second["text"], "b".repeat(3000));
        assert!(second.get("reply_parameters").is_none());
        assert!(!answer.to_string().contains(TOKEN));
    }

    #[test]
    fn pieces_without_a_newline_break_at_the_limit_in_utf16_units() {
        let text = "😀".repeat(3000);
        let split = pieces(&text);
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].encode_utf16().count(), LONGEST_MESSAGE);
        assert_eq!(split.concat(), text);
    }

    #[tokio::test]
    async fn the_first_poll_confirms_a_baseline_and_later_ones_report_messages() {
        let fixture = Fixture::start(vec![
            bot(
                "getUpdates",
                200,
                json!({"ok": true, "result": [update(900, "old")]}),
            ),
            bot(
                "getUpdates",
                200,
                json!({"ok": true, "result": [update(901, "hello"), update(902, "there")]}),
            ),
            bot("getUpdates", 200, json!({"ok": true, "result": []})),
        ])
        .await;
        let cx = cx(&fixture);
        let first = Telegram.poll(&cx, None).await.unwrap();
        assert!(first.events.is_empty());
        assert_eq!(first.cursor.as_deref(), Some("901"));
        let second = Telegram.poll(&cx, Some("901")).await.unwrap();
        assert_eq!(second.cursor.as_deref(), Some("903"));
        assert_eq!(second.events.len(), 2);
        assert_eq!(second.events[0].id, "901");
        assert_eq!(second.events[0].data["text"], "hello");
        assert_eq!(second.events[0].data["chat"]["id"], 42);
        assert_eq!(second.events[0].data["from"]["username"], "ana");
        let third = Telegram.poll(&cx, Some("903")).await.unwrap();
        assert_eq!(third.cursor.as_deref(), Some("903"));
        let seen = fixture.finish().await;
        assert_eq!(seen[0].json()["offset"], -1);
        assert_eq!(seen[1].json()["offset"], 901);
        assert_eq!(seen[1].json()["allowed_updates"], json!(["message"]));
    }

    #[tokio::test]
    async fn a_refused_token_is_unauthorized_and_no_error_carries_it() {
        let fixture = Fixture::start(vec![
            bot("getMe", 401, json!({"ok": false, "error_code": 401, "description": "Unauthorized"})),
            bot("getChat", 404, json!({"ok": false, "error_code": 404, "description": "Not Found"})),
            bot("sendMessage", 400, json!({"ok": false, "error_code": 400, "description": format!("Bad Request: chat not found for {TOKEN}")})),
        ])
        .await;
        let cx = cx(&fixture);
        assert_eq!(Telegram.check(&cx).await, Err(Error::Unauthorized));
        let chat = Telegram.call(&cx, "get_chat", json!({"chat_id": 1})).await;
        assert_eq!(chat, Err(Error::Unauthorized));
        let input = json!({"chat_id": 1, "text": "hi"});
        let refused = Telegram.call(&cx, "send_message", input).await.unwrap_err();
        let shown = format!("{refused} {refused:?}");
        assert!(shown.contains("chat not found"), "{shown}");
        assert!(!shown.contains(TOKEN), "{shown}");
        fixture.finish().await;
    }
}

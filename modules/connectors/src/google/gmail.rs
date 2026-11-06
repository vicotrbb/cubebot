//! Gmail (https://developers.google.com/gmail/api/reference/rest).
use super::{BASE64URL, LONGEST_TEXT, at, clip, decode};
use crate::{Context, Error, Event};
use base64::Engine;
use futures_util::future::try_join_all;
use lettre::message::{Mailbox, Message, MessageBuilder};
use serde::Deserialize;
use serde_json::{Value, json};

const GMAIL: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
/// The newest messages of a long thread that are returned.
const THREAD_MESSAGES: usize = 25;
/// New messages one poll reports; the rest are skipped rather than flood the routines.
const POLLED_MESSAGES: usize = 20;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    #[serde(default)]
    query: String,
    limit: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Thread {
    thread: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Compose {
    to: Vec<String>,
    #[serde(default)]
    cc: Vec<String>,
    #[serde(default)]
    bcc: Vec<String>,
    subject: Option<String>,
    text: String,
    reply_to: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Modify {
    message: String,
    #[serde(default)]
    add: Vec<String>,
    #[serde(default)]
    remove: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attachment {
    message: String,
    attachment: String,
    filename: Option<String>,
}

pub(super) async fn call(cx: &Context, name: &str, input: Value) -> Result<Value, Error> {
    match name {
        "search" => search(cx, input).await,
        "read_thread" => read_thread(cx, input).await,
        "list_labels" => labels(cx).await,
        "send" => {
            let raw = compose(cx, input).await?;
            let sent = cx.json(cx.post(&format!("{GMAIL}/messages/send"))?.json(&raw));
            Ok(json!({"sent": summary(&sent.await?)}))
        }
        "draft" => {
            let raw = compose(cx, input).await?;
            let body = json!({"message": raw});
            let made = cx.json(cx.post(&format!("{GMAIL}/drafts"))?.json(&body));
            let made = made.await?;
            Ok(json!({"draft": made["id"], "message": summary(&made["message"])}))
        }
        "modify" => modify(cx, input).await,
        "save_attachment" => attachment(cx, input).await,
        _ => Err(Error::UnknownTool),
    }
}

async fn search(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: Search = crate::input(input)?;
    let limit = search.limit.unwrap_or(10).clamp(1, 25).to_string();
    let request = cx
        .get(&format!("{GMAIL}/messages"))?
        .query(&[("q", search.query.as_str()), ("maxResults", &limit)]);
    let found = cx.json(request).await?;
    let ids = found["messages"].as_array().into_iter().flatten();
    let messages = try_join_all(
        ids.filter_map(|m| m["id"].as_str())
            .map(|id| headers(cx, id)),
    );
    Ok(json!({"messages": messages.await?}))
}

/// A message's id, thread, sender, subject, date and snippet.
async fn headers(cx: &Context, id: &str) -> Result<Value, Error> {
    let request = cx.get(&at(GMAIL, &["messages", id]))?.query(&[
        ("format", "metadata"),
        ("metadataHeaders", "From"),
        ("metadataHeaders", "Subject"),
        ("metadataHeaders", "Date"),
    ]);
    let message = cx.json(request).await?;
    let get = |name| header(&message["payload"], name);
    Ok(json!({
        "id": message["id"],
        "thread": message["threadId"],
        "from": get("From"),
        "subject": get("Subject"),
        "date": get("Date"),
        "snippet": message["snippet"],
        "labels": message["labelIds"],
    }))
}

fn header<'a>(payload: &'a Value, name: &str) -> &'a str {
    payload["headers"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|header| {
            header["name"]
                .as_str()
                .is_some_and(|found| found.eq_ignore_ascii_case(name))
        })
        .and_then(|header| header["value"].as_str())
        .unwrap_or_default()
}

async fn read_thread(cx: &Context, input: Value) -> Result<Value, Error> {
    let thread: Thread = crate::input(input)?;
    let request = cx
        .get(&at(GMAIL, &["threads", &thread.thread]))?
        .query(&[("format", "full")]);
    let found = cx.json(request).await?;
    let all = found["messages"].as_array().map_or(&[][..], Vec::as_slice);
    let newest = &all[all.len().saturating_sub(THREAD_MESSAGES)..];
    let messages: Result<Vec<Value>, Error> = newest.iter().map(message).collect();
    Ok(
        json!({"thread": found["id"], "messages": messages?, "earlier_messages_left_out": all.len() - newest.len()}),
    )
}

/// A message with its plain text, the HTML stripped when there is no plain part.
fn message(message: &Value) -> Result<Value, Error> {
    let payload = &message["payload"];
    let mut parts = Parts::default();
    parts.walk(payload)?;
    let text = match parts.plain.is_empty() {
        true => strip(&parts.html.join("\n")),
        false => parts.plain.join("\n"),
    };
    Ok(json!({
        "id": message["id"],
        "from": header(payload, "From"),
        "to": header(payload, "To"),
        "cc": header(payload, "Cc"),
        "subject": header(payload, "Subject"),
        "date": header(payload, "Date"),
        "labels": message["labelIds"],
        "text": clip(&text, LONGEST_TEXT),
        "attachments": parts.files,
    }))
}

#[derive(Default)]
struct Parts {
    plain: Vec<String>,
    html: Vec<String>,
    files: Vec<Value>,
}

impl Parts {
    /// Collects the text bodies and attachments of a MIME tree.
    fn walk(&mut self, part: &Value) -> Result<(), Error> {
        let kind = part["mimeType"].as_str().unwrap_or_default();
        let filename = part["filename"].as_str().unwrap_or_default();
        let data = part["body"]["data"].as_str();
        if !filename.is_empty() {
            self.files.push(json!({
                "attachment": part["body"]["attachmentId"],
                "filename": filename,
                "type": kind,
                "size": part["body"]["size"],
            }));
        } else if let Some(data) = data.filter(|_| kind.starts_with("text/")) {
            let text = String::from_utf8_lossy(&decode(data)?).into_owned();
            match kind {
                "text/html" => self.html.push(text),
                _ => self.plain.push(text),
            }
        }
        for child in part["parts"].as_array().into_iter().flatten() {
            self.walk(child)?;
        }
        Ok(())
    }
}

/// Readable text from HTML mail.
// shortcut: drops tags and a few entities but keeps style and script text; use an HTML parser
// if HTML-only mail reads badly.
fn strip(html: &str) -> String {
    let mut text = String::new();
    let mut inside = false;
    for c in html.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => inside = false,
            c if !inside => text.push(c),
            _ => (),
        }
    }
    let text = text
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    lines.join("\n")
}

async fn labels(cx: &Context) -> Result<Value, Error> {
    let found = cx.json(cx.get(&format!("{GMAIL}/labels"))?).await?;
    let labels: Vec<Value> = found["labels"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|label| json!({"id": label["id"], "name": label["name"], "type": label["type"]}))
        .collect();
    Ok(json!({"labels": labels}))
}

/// The `raw` RFC 2822 message Gmail sends or drafts, in the replied-to thread when there is one.
async fn compose(cx: &Context, input: Value) -> Result<Value, Error> {
    let compose: Compose = crate::input(input)?;
    let profile = cx.json(cx.get(&format!("{GMAIL}/profile"))?).await?;
    let from = profile["emailAddress"].as_str().unwrap_or_default();
    let mut builder = Message::builder().from(mailbox(from)?).keep_bcc();
    for to in &compose.to {
        builder = builder.to(mailbox(to)?);
    }
    for cc in &compose.cc {
        builder = builder.cc(mailbox(cc)?);
    }
    for bcc in &compose.bcc {
        builder = builder.bcc(mailbox(bcc)?);
    }
    let (builder, subject, thread) = match &compose.reply_to {
        Some(id) => reply(cx, builder, id, compose.subject).await?,
        None => (builder, compose.subject.unwrap_or_default(), None),
    };
    let message = builder
        .subject(subject)
        .body(compose.text)
        .map_err(|error| Error::Invalid(error.to_string()))?;
    let raw = BASE64URL.encode(message.formatted());
    Ok(json!({"raw": raw, "threadId": thread}))
}

/// Threads the message under `id`: Gmail keeps it in the thread only when the subject matches.
async fn reply(
    cx: &Context,
    builder: MessageBuilder,
    id: &str,
    subject: Option<String>,
) -> Result<(MessageBuilder, String, Option<String>), Error> {
    let request = cx.get(&at(GMAIL, &["messages", id]))?.query(&[
        ("format", "metadata"),
        ("metadataHeaders", "Subject"),
        ("metadataHeaders", "Message-ID"),
        ("metadataHeaders", "References"),
    ]);
    let original = cx.json(request).await?;
    let payload = &original["payload"];
    let message_id = header(payload, "Message-ID").to_owned();
    let references = format!("{} {message_id}", header(payload, "References"));
    let subject = subject.unwrap_or_else(|| match header(payload, "Subject") {
        old if old.to_ascii_lowercase().starts_with("re:") => old.to_owned(),
        old => format!("Re: {old}"),
    });
    let builder = builder
        .in_reply_to(message_id)
        .references(references.trim().to_owned());
    let thread = original["threadId"].as_str().map(str::to_owned);
    Ok((builder, subject, thread))
}

fn mailbox(address: &str) -> Result<Mailbox, Error> {
    address
        .trim()
        .parse()
        .map_err(|_| Error::Invalid(format!("{address:?} is not an email address")))
}

fn summary(message: &Value) -> Value {
    json!({"id": message["id"], "thread": message["threadId"], "labels": message["labelIds"]})
}

async fn modify(cx: &Context, input: Value) -> Result<Value, Error> {
    let modify: Modify = crate::input(input)?;
    let body = json!({"addLabelIds": modify.add, "removeLabelIds": modify.remove});
    let path = at(GMAIL, &["messages", &modify.message, "modify"]);
    let changed = cx.json(cx.post(&path)?.json(&body)).await?;
    Ok(summary(&changed))
}

async fn attachment(cx: &Context, input: Value) -> Result<Value, Error> {
    let wanted: Attachment = crate::input(input)?;
    let path = at(
        GMAIL,
        &[
            "messages",
            &wanted.message,
            "attachments",
            &wanted.attachment,
        ],
    );
    let found = cx.json(cx.get(&path)?).await?;
    let bytes = decode(found["data"].as_str().unwrap_or_default())?;
    let name = wanted.filename.as_deref().unwrap_or("attachment");
    Ok(crate::file(name, None, &bytes))
}

/// New inbox messages since the history id in `cursor`; answers the next history id. Without
/// one, or when Google no longer keeps that much history, it starts from now.
pub(super) async fn poll(
    cx: &Context,
    cursor: Option<&str>,
    events: &mut Vec<Event>,
) -> Result<Value, Error> {
    let Some(start) = cursor else {
        return latest(cx).await;
    };
    let request = cx.get(&format!("{GMAIL}/history"))?.query(&[
        ("startHistoryId", start),
        ("historyTypes", "messageAdded"),
        ("labelId", "INBOX"),
        ("maxResults", "500"),
    ]);
    // shortcut: reads one page of history (500 changes); follow nextPageToken if polls lag.
    let history = match cx.json(request).await {
        Err(Error::NotFound(_)) => return latest(cx).await,
        found => found?,
    };
    let mut ids: Vec<&str> = history["history"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|change| change["messagesAdded"].as_array().into_iter().flatten())
        .filter_map(|added| added["message"]["id"].as_str())
        .collect();
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(*id));
    for id in ids.into_iter().take(POLLED_MESSAGES) {
        match headers(cx, id).await {
            Ok(message) => events.push(received(message)),
            // Deleted again before this poll read it.
            Err(Error::NotFound(_)) => (),
            Err(error) => return Err(error),
        }
    }
    Ok(history["historyId"]
        .as_str()
        .map_or(json!(start), |id| json!(id)))
}

async fn latest(cx: &Context) -> Result<Value, Error> {
    let profile = cx.json(cx.get(&format!("{GMAIL}/profile"))?).await?;
    Ok(profile["historyId"].clone())
}

fn received(message: Value) -> Event {
    let from = message["from"].as_str().unwrap_or_default();
    let subject = message["subject"].as_str().unwrap_or_default();
    Event {
        kind: "mail.received",
        id: message["id"].as_str().unwrap_or_default().to_owned(),
        summary: clip(&format!("{from}: {subject}"), 200),
        data: message,
    }
}

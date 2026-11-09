//! Microsoft 365 through Microsoft Graph (https://learn.microsoft.com/graph/api/overview): Outlook
//! mail, Calendar, OneDrive and To Do, signed in with the person's own public client and a device
//! code, so no secret or redirect address is needed.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Credential, Error, Event, EventSpec,
    Field, FieldKind, Level, Manifest, OAuth2, Poll, Product, Risk, Scope, Step, ToolSpec,
};
use base64::Engine;
use chrono::{NaiveDateTime, TimeZone};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

const API: &str = "https://graph.microsoft.com/v1.0";
const INBOX: &str = "/me/mailFolders/inbox/messages";
/// The most text a tool hands the model at once.
const LONGEST_TEXT: usize = 50_000;
/// Graph's limit for a single-request upload.
const LARGEST_UPLOAD: usize = 4 * 1024 * 1024;
/// Before the first message: a cursor for an empty Inbox.
const EPOCH: &str = "1970-01-01T00:00:00Z";

pub struct Microsoft;

const MAIL: Option<&str> = Some("mail");
const CALENDAR: Option<&str> = Some("calendar");
const FILES: Option<&str> = Some("files");
const TODO: Option<&str> = Some("todo");

static MANIFEST: Manifest = Manifest {
    id: "microsoft",
    name: "Microsoft 365",
    description: "Outlook mail, Calendar, OneDrive and To Do through Microsoft Graph.",
    category: Category::MailCalendar,
    auth: &[Auth::OAuth2(OAuth2 {
        authorize: "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize",
        token: "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token",
        device: Some("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/devicecode"),
        revoke: None,
        scopes: &[
            Scope {
                scope: "offline_access",
                product: None,
            },
            Scope {
                scope: "User.Read",
                product: None,
            },
            Scope {
                scope: "Mail.ReadWrite",
                product: MAIL,
            },
            Scope {
                scope: "Mail.Send",
                product: MAIL,
            },
            Scope {
                scope: "Calendars.ReadWrite",
                product: CALENDAR,
            },
            Scope {
                scope: "Files.ReadWrite",
                product: FILES,
            },
            Scope {
                scope: "Tasks.ReadWrite",
                product: TODO,
            },
        ],
        params: &[],
        fields: &[Field {
            key: "client_id",
            label: "Application (client) ID",
            kind: FieldKind::Text,
            required: true,
            placeholder: "00000000-0000-0000-0000-000000000000",
            help: "From the app registration's Overview page.",
        }],
    })],
    config: &[Field {
        key: "tenant",
        label: "Tenant",
        kind: FieldKind::Text,
        required: true,
        placeholder: "common",
        help: "`common` for any account, `consumers` for personal accounts only, `organizations` for work or school accounts only, or your tenant's id.",
    }],
    products: &[
        Product {
            id: "mail",
            name: "Outlook mail",
        },
        Product {
            id: "calendar",
            name: "Calendar",
        },
        Product {
            id: "files",
            name: "OneDrive",
        },
        Product {
            id: "todo",
            name: "To Do",
        },
    ],
    tools: &[
        ToolSpec {
            name: "mail_search",
            description: "Find messages by search words (Outlook search syntax) or an OData filter, newest first.",
            risk: Risk::Read,
            product: MAIL,
            input: r#"{"type":"object","properties":{"query":{"type":"string","description":"Search words, such as from:ana invoice."},"filter":{"type":"string","description":"An OData filter, such as isRead eq false. Not with query."},"folder":{"type":"string","description":"A folder id or well-known name such as inbox, sentitems or archive."},"limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "mail_read",
            description: "A message's headers, its body as text and the list of its attachments.",
            risk: Risk::Read,
            product: MAIL,
            input: r#"{"type":"object","properties":{"message":{"type":"string","description":"The message id."}},"required":["message"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "mail_send",
            description: "Send an email; it is saved in Sent Items.",
            risk: Risk::Send,
            product: MAIL,
            input: MAIL_INPUT,
        },
        ToolSpec {
            name: "mail_reply",
            description: "Reply to a message, or to everyone on it with all.",
            risk: Risk::Send,
            product: MAIL,
            input: r#"{"type":"object","properties":{"message":{"type":"string"},"comment":{"type":"string","description":"The reply's text; the original is quoted below it."},"all":{"type":"boolean"}},"required":["message","comment"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "mail_draft",
            description: "Save an email in Drafts without sending it.",
            risk: Risk::Write,
            product: MAIL,
            input: MAIL_INPUT,
        },
        ToolSpec {
            name: "mail_move",
            description: "Move a message to a folder, by id or well-known name such as archive or deleteditems.",
            risk: Risk::Write,
            product: MAIL,
            input: r#"{"type":"object","properties":{"message":{"type":"string"},"destination":{"type":"string"}},"required":["message","destination"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "mail_save_attachment",
            description: "Save a message's file attachment to the workspace.",
            risk: Risk::Read,
            product: MAIL,
            input: r#"{"type":"object","properties":{"message":{"type":"string"},"attachment":{"type":"string","description":"The attachment id from mail_read."}},"required":["message","attachment"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "calendar_list",
            description: "Your calendars, with their ids and whether you can edit them.",
            risk: Risk::Read,
            product: CALENDAR,
            input: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "calendar_events",
            description: "Events between two times, including each occurrence of recurring ones, in UTC.",
            risk: Risk::Read,
            product: CALENDAR,
            input: r#"{"type":"object","properties":{"start":{"type":"string","description":"ISO 8601, such as 2026-11-05T00:00:00Z."},"end":{"type":"string"},"calendar":{"type":"string","description":"A calendar id from calendar_list; your default calendar when absent."},"limit":{"type":"integer","minimum":1,"maximum":500}},"required":["start","end"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "calendar_create_event",
            description: "Create an event on your own calendar, without inviting anyone.",
            risk: Risk::Write,
            product: CALENDAR,
            input: r#"{"type":"object","properties":{"subject":{"type":"string"},"start":{"type":"string","description":"Local time without offset, such as 2026-11-05T09:00:00."},"end":{"type":"string"},"time_zone":{"type":"string","description":"Such as Europe/Lisbon; UTC when absent."},"body":{"type":"string"},"location":{"type":"string"}},"required":["subject","start","end"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "calendar_update_event",
            description: "Change an event or one occurrence of a repeating one, or with all the whole series.",
            risk: Risk::Write,
            product: CALENDAR,
            input: r#"{"type":"object","properties":{"event":{"type":"string","description":"An event or occurrence id from calendar_events."},"all":{"type":"boolean","description":"Change the whole series an occurrence belongs to."},"subject":{"type":"string"},"start":{"type":"string","description":"Local time without offset, such as 2026-11-05T09:00:00, or a date for an all-day event; given with end."},"end":{"type":"string"},"all_day":{"type":"boolean","description":"True when start and end are dates; false to make an all-day event timed."},"time_zone":{"type":"string","description":"Such as Europe/Lisbon; UTC when absent."},"body":{"type":"string"},"location":{"type":"string"}},"required":["event"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "calendar_delete_event",
            description: "Delete an event or one occurrence of a repeating one, or with all the whole series.",
            risk: Risk::Write,
            product: CALENDAR,
            input: r#"{"type":"object","properties":{"event":{"type":"string"},"all":{"type":"boolean","description":"Delete the whole series an occurrence belongs to."}},"required":["event"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "calendar_invite",
            description: "Create a meeting and email invitations to its attendees.",
            risk: Risk::Send,
            product: CALENDAR,
            input: r#"{"type":"object","properties":{"subject":{"type":"string"},"start":{"type":"string","description":"Local time without offset, such as 2026-11-05T09:00:00."},"end":{"type":"string"},"time_zone":{"type":"string","description":"Such as Europe/Lisbon; UTC when absent."},"body":{"type":"string"},"location":{"type":"string"},"attendees":{"type":"array","items":{"type":"string"},"minItems":1,"description":"Email addresses."}},"required":["subject","start","end","attendees"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "calendar_respond",
            description: "Accept, decline or tentatively accept an invitation; the organizer is told.",
            risk: Risk::Send,
            product: CALENDAR,
            input: r#"{"type":"object","properties":{"event":{"type":"string"},"response":{"type":"string","enum":["accept","decline","tentativelyAccept"]},"comment":{"type":"string"}},"required":["event","response"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "files_search",
            description: "Find OneDrive files and folders by name or content.",
            risk: Risk::Read,
            product: FILES,
            input: r#"{"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["query"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "files_read",
            description: "A OneDrive file's text, or the file saved to the workspace when it is not text.",
            risk: Risk::Read,
            product: FILES,
            input: r#"{"type":"object","properties":{"item":{"type":"string","description":"The file's id from files_search."}},"required":["item"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "files_upload",
            description: "Create or replace a OneDrive file of at most 4 MB from text or base64.",
            risk: Risk::Write,
            product: FILES,
            input: r#"{"type":"object","properties":{"path":{"type":"string","description":"Such as /Documents/notes.md."},"text":{"type":"string"},"base64":{"type":"string"}},"required":["path"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "todo_lists",
            description: "Your To Do task lists.",
            risk: Risk::Read,
            product: TODO,
            input: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "todo_tasks",
            description: "The tasks in a To Do list, open ones only unless completed is true.",
            risk: Risk::Read,
            product: TODO,
            input: r#"{"type":"object","properties":{"list":{"type":"string"},"completed":{"type":"boolean"},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["list"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "todo_create_task",
            description: "Add a task to a To Do list.",
            risk: Risk::Write,
            product: TODO,
            input: r#"{"type":"object","properties":{"list":{"type":"string"},"title":{"type":"string"},"body":{"type":"string"},"due":{"type":"string","description":"A date, such as 2026-11-05."},"importance":{"type":"string","enum":["low","normal","high"]}},"required":["list","title"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "todo_complete_task",
            description: "Mark a To Do task as completed.",
            risk: Risk::Write,
            product: TODO,
            input: r#"{"type":"object","properties":{"list":{"type":"string"},"task":{"type":"string"}},"required":["list","task"],"additionalProperties":false}"#,
        },
    ],
    events: &[EventSpec {
        kind: "mail.received",
        description: "A message arrived in the Inbox.",
        product: MAIL,
    }],
    level: Level::Full,
    guide: &[
        Step {
            title: "Register an app",
            body: "In Microsoft Entra (or the Azure portal's App registrations) choose New registration. Name it Cubebot, choose \"Accounts in any organizational directory and personal Microsoft accounts\", and leave the redirect URI empty.",
            link: Some(
                "https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade",
            ),
            copy: None,
        },
        Step {
            title: "Allow public client flows",
            body: "Under Authentication, set \"Allow public client flows\" to Yes and save. Cubebot signs in with a device code, so the app needs no secret and no redirect address, and this works behind NAT.",
            link: None,
            copy: None,
        },
        Step {
            title: "Add API permissions",
            body: "Under API permissions choose Add a permission › Microsoft Graph › Delegated permissions, and add these.",
            link: None,
            copy: Some(
                "offline_access User.Read Mail.ReadWrite Mail.Send Calendars.ReadWrite Files.ReadWrite Tasks.ReadWrite",
            ),
        },
        Step {
            title: "Copy the client ID and sign in",
            body: "Copy the Application (client) ID from the Overview page and paste it below. Then open the address shown, enter the code and sign in with the account to connect.",
            link: None,
            copy: None,
        },
    ],
    warning: None,
};

const MAIL_INPUT: &str = r#"{"type":"object","properties":{"to":{"type":"array","items":{"type":"string"},"description":"Email addresses."},"cc":{"type":"array","items":{"type":"string"}},"bcc":{"type":"array","items":{"type":"string"}},"subject":{"type":"string"},"body":{"type":"string"},"html":{"type":"boolean","description":"True when body is HTML."}},"required":["to","subject","body"],"additionalProperties":false}"#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: Option<String>,
    filter: Option<String>,
    folder: Option<String>,
    limit: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mail {
    to: Vec<String>,
    #[serde(default)]
    cc: Vec<String>,
    #[serde(default)]
    bcc: Vec<String>,
    subject: String,
    body: String,
    #[serde(default)]
    html: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    message: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    message: String,
    comment: String,
    #[serde(default)]
    all: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Move {
    message: String,
    destination: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attachment {
    message: String,
    attachment: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Window {
    start: String,
    end: String,
    calendar: Option<String>,
    limit: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    event: String,
    #[serde(default)]
    all: bool,
    subject: Option<String>,
    start: Option<String>,
    end: Option<String>,
    all_day: Option<bool>,
    time_zone: Option<String>,
    body: Option<String>,
    location: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Removal {
    event: String,
    #[serde(default)]
    all: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewEvent {
    subject: String,
    start: String,
    end: String,
    time_zone: Option<String>,
    body: Option<String>,
    location: Option<String>,
    #[serde(default)]
    attendees: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Respond {
    event: String,
    response: String,
    comment: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSearch {
    query: String,
    limit: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    item: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Upload {
    path: String,
    text: Option<String>,
    base64: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tasks {
    list: String,
    #[serde(default)]
    completed: bool,
    limit: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewTask {
    list: String,
    title: String,
    body: Option<String>,
    due: Option<String>,
    importance: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Task {
    list: String,
    task: String,
}

impl Connector for Microsoft {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let url = format!(
                "{API}/me{}",
                query(&[("$select", "mail,userPrincipalName")])
            );
            let me = cx.json(cx.get(&url)?).await?;
            let name = me["mail"]
                .as_str()
                .or(me["userPrincipalName"].as_str())
                .unwrap_or("Microsoft account");
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool.split_once('_').map_or(tool, |(product, _)| product) {
                "mail" => mail(cx, tool, input).await,
                "calendar" => calendar(cx, tool, input).await,
                "files" => files(cx, tool, input).await,
                "todo" => todo(cx, tool, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(async move {
            let Some((since, seen)) = cursor.and_then(parse_cursor) else {
                return baseline(cx).await;
            };
            received(cx, since, &seen).await
        })
    }
}

async fn mail(cx: &Context, tool: &str, input: Value) -> Result<Value, Error> {
    match tool {
        "mail_search" => mail_search(cx, input).await,
        "mail_read" => mail_read(cx, input).await,
        "mail_send" => mail_send(cx, input).await,
        "mail_reply" => mail_reply(cx, input).await,
        "mail_draft" => mail_draft(cx, input).await,
        "mail_move" => mail_move(cx, input).await,
        "mail_save_attachment" => save_attachment(cx, input).await,
        _ => Err(Error::UnknownTool),
    }
}

async fn calendar(cx: &Context, tool: &str, input: Value) -> Result<Value, Error> {
    match tool {
        "calendar_list" => calendars(cx).await,
        "calendar_events" => events(cx, input).await,
        "calendar_update_event" => update_event(cx, input).await,
        "calendar_delete_event" => delete_event(cx, input).await,
        "calendar_create_event" => create_event(cx, input, false).await,
        "calendar_invite" => create_event(cx, input, true).await,
        "calendar_respond" => respond(cx, input).await,
        _ => Err(Error::UnknownTool),
    }
}

async fn files(cx: &Context, tool: &str, input: Value) -> Result<Value, Error> {
    match tool {
        "files_search" => files_search(cx, input).await,
        "files_read" => files_read(cx, input).await,
        "files_upload" => files_upload(cx, input).await,
        _ => Err(Error::UnknownTool),
    }
}

async fn todo(cx: &Context, tool: &str, input: Value) -> Result<Value, Error> {
    match tool {
        "todo_lists" => todo_lists(cx).await,
        "todo_tasks" => todo_tasks(cx, input).await,
        "todo_create_task" => todo_create(cx, input).await,
        "todo_complete_task" => todo_complete(cx, input).await,
        _ => Err(Error::UnknownTool),
    }
}

/// Percent-encodes everything outside `keep` and the unreserved characters.
fn encode(text: &str, keep: &[u8]) -> String {
    text.bytes()
        .map(
            |b| match b.is_ascii_alphanumeric() || b"-_.~".contains(&b) || keep.contains(&b) {
                true => char::from(b).to_string(),
                false => format!("%{b:02X}"),
            },
        )
        .collect()
}

/// A path segment, such as a message id.
fn part(text: &str) -> String {
    encode(text, b"")
}

/// A query string; OData's `$` names stay literal.
fn query(pairs: &[(&str, &str)]) -> String {
    let pairs: Vec<String> = pairs
        .iter()
        .map(|(key, value)| format!("{key}={}", encode(value, b"$,()'/:*@!")))
        .collect();
    format!("?{}", pairs.join("&"))
}

fn limit(limit: Option<u8>, default: u8) -> String {
    limit.unwrap_or(default).clamp(1, 100).to_string()
}

fn bounded(text: &str) -> (String, bool) {
    let mut chars = text.chars();
    let kept: String = chars.by_ref().take(LONGEST_TEXT).collect();
    (kept, chars.next().is_some())
}

fn list(answer: &Value, compact: fn(&Value) -> Value) -> Vec<Value> {
    answer["value"]
        .as_array()
        .into_iter()
        .flatten()
        .map(compact)
        .collect()
}

fn addresses(recipients: &Value) -> Vec<Value> {
    recipients
        .as_array()
        .into_iter()
        .flatten()
        .map(|recipient| recipient["emailAddress"]["address"].clone())
        .collect()
}

fn recipients(addresses: &[String]) -> Vec<Value> {
    addresses
        .iter()
        .map(|address| json!({"emailAddress": {"address": address}}))
        .collect()
}

fn summary(message: &Value) -> Value {
    json!({
        "id": message["id"],
        "from": message["from"]["emailAddress"]["address"],
        "subject": message["subject"],
        "received": message["receivedDateTime"],
        "preview": message["bodyPreview"],
    })
}

async fn mail_search(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: Search = crate::input(input)?;
    let path = match &search.folder {
        Some(folder) => format!("/me/mailFolders/{}/messages", part(folder)),
        None => "/me/messages".into(),
    };
    let top = limit(search.limit, 20);
    let mut pairs = vec![
        (
            "$select",
            "id,from,subject,receivedDateTime,bodyPreview".to_owned(),
        ),
        ("$top", top),
    ];
    // Graph refuses $orderby with $search, and with a filter not on receivedDateTime.
    match (search.query, search.filter) {
        (Some(words), _) => pairs.push(("$search", format!("\"{}\"", words.replace('"', "\\\"")))),
        (None, Some(filter)) => pairs.push(("$filter", filter)),
        (None, None) => pairs.push(("$orderby", "receivedDateTime desc".into())),
    }
    let pairs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let found = cx
        .json(cx.get(&format!("{API}{path}{}", query(&pairs)))?)
        .await?;
    Ok(json!({"messages": list(&found, summary)}))
}

async fn mail_read(cx: &Context, input: Value) -> Result<Value, Error> {
    let id = part(&crate::input::<Message>(input)?.message);
    let select =
        "subject,from,toRecipients,ccRecipients,receivedDateTime,body,hasAttachments,webLink";
    let url = format!("{API}/me/messages/{id}{}", query(&[("$select", select)]));
    let request = cx
        .get(&url)?
        .header("prefer", "outlook.body-content-type=\"text\"");
    let message = cx.json(request).await?;
    let mut attachments = Value::Array(Vec::new());
    if message["hasAttachments"] == true {
        let select = "id,name,contentType,size,isInline";
        let url = format!(
            "{API}/me/messages/{id}/attachments{}",
            query(&[("$select", select)])
        );
        let found = cx.json(cx.get(&url)?).await?;
        attachments = found["value"].clone();
    }
    let (body, truncated) = bounded(message["body"]["content"].as_str().unwrap_or_default());
    Ok(json!({
        "subject": message["subject"],
        "from": message["from"]["emailAddress"],
        "to": addresses(&message["toRecipients"]),
        "cc": addresses(&message["ccRecipients"]),
        "received": message["receivedDateTime"],
        "body": body,
        "truncated": truncated,
        "attachments": attachments,
        "link": message["webLink"],
    }))
}

fn message(mail: &Mail) -> Value {
    let kind = if mail.html { "HTML" } else { "Text" };
    json!({
        "subject": mail.subject,
        "body": {"contentType": kind, "content": mail.body},
        "toRecipients": recipients(&mail.to),
        "ccRecipients": recipients(&mail.cc),
        "bccRecipients": recipients(&mail.bcc),
    })
}

async fn mail_send(cx: &Context, input: Value) -> Result<Value, Error> {
    let mail: Mail = crate::input(input)?;
    if mail.to.is_empty() {
        return Err(Error::Invalid("to needs at least one address".into()));
    }
    let body = json!({"message": message(&mail), "saveToSentItems": true});
    cx.json(cx.post(&format!("{API}/me/sendMail"))?.json(&body))
        .await?;
    Ok(json!({"sent": true}))
}

async fn mail_reply(cx: &Context, input: Value) -> Result<Value, Error> {
    let reply: Reply = crate::input(input)?;
    let action = if reply.all { "replyAll" } else { "reply" };
    let url = format!("{API}/me/messages/{}/{action}", part(&reply.message));
    let body = json!({"comment": reply.comment});
    cx.json(cx.post(&url)?.json(&body)).await?;
    Ok(json!({"sent": true}))
}

async fn mail_draft(cx: &Context, input: Value) -> Result<Value, Error> {
    let mail: Mail = crate::input(input)?;
    let url = format!("{API}/me/messages");
    let draft = cx.json(cx.post(&url)?.json(&message(&mail))).await?;
    Ok(json!({"id": draft["id"], "link": draft["webLink"]}))
}

async fn mail_move(cx: &Context, input: Value) -> Result<Value, Error> {
    let moved: Move = crate::input(input)?;
    let url = format!("{API}/me/messages/{}/move", part(&moved.message));
    let body = json!({"destinationId": moved.destination});
    let answer = cx.json(cx.post(&url)?.json(&body)).await?;
    // Moving gives the message a new id.
    Ok(json!({"id": answer["id"], "folder": answer["parentFolderId"]}))
}

async fn save_attachment(cx: &Context, input: Value) -> Result<Value, Error> {
    let wanted: Attachment = crate::input(input)?;
    let url = format!(
        "{API}/me/messages/{}/attachments/{}",
        part(&wanted.message),
        part(&wanted.attachment)
    );
    let attachment = cx.json(cx.get(&url)?).await?;
    let Some(bytes) = attachment["contentBytes"].as_str() else {
        return Err(Error::Invalid(
            "not a file attachment, such as an attached message".into(),
        ));
    };
    // Graph already answers in standard base64, which is what a file result carries.
    let kind = attachment["contentType"].as_str();
    let name = attachment["name"].as_str().unwrap_or("attachment");
    Ok(json!({crate::FILE: {"name": name, "content_type": kind, "base64": bytes}}))
}

fn event(event: &Value) -> Value {
    json!({
        "id": event["id"],
        "subject": event["subject"],
        "start": event["start"]["dateTime"],
        "end": event["end"]["dateTime"],
        "time_zone": event["start"]["timeZone"],
        "series": event["seriesMasterId"],
        "all_day": event["isAllDay"],
        "location": event["location"]["displayName"],
        "organizer": event["organizer"]["emailAddress"]["address"],
        "attendees": addresses(&event["attendees"]),
        "join_url": event["onlineMeeting"]["joinUrl"],
        "link": event["webLink"],
    })
}

async fn calendars(cx: &Context) -> Result<Value, Error> {
    let found = cx.json(cx.get(&format!("{API}/me/calendars"))?).await?;
    let calendars = list(&found, |calendar| {
        let color = match calendar["hexColor"].as_str() {
            Some(hex) if !hex.is_empty() => &calendar["hexColor"],
            _ => &calendar["color"],
        };
        json!({"id": calendar["id"], "name": calendar["name"], "color": color, "can_edit": calendar["canEdit"]})
    });
    Ok(json!({"calendars": calendars}))
}

async fn events(cx: &Context, input: Value) -> Result<Value, Error> {
    let window: Window = crate::input(input)?;
    let wanted = usize::from(window.limit.unwrap_or(50).clamp(1, 500));
    let select = "subject,start,end,isAllDay,location,organizer,attendees,onlineMeeting,webLink,type,seriesMasterId";
    let top = wanted.min(100).to_string();
    let pairs = [
        ("startDateTime", window.start.as_str()),
        ("endDateTime", window.end.as_str()),
        ("$top", &top),
        ("$orderby", "start/dateTime"),
        ("$select", select),
    ];
    let base = match &window.calendar {
        Some(id) => format!("{API}/me/calendars/{}/calendarView", part(id)),
        None => format!("{API}/me/calendarView"),
    };
    let mut url = format!("{base}{}", query(&pairs));
    let mut found = Vec::new();
    loop {
        let request = cx.get(&url)?.header("prefer", "outlook.timezone=\"UTC\"");
        let page = cx.json(request).await?;
        let events = page["value"].as_array().into_iter().flatten();
        found.extend(events.map(|item| in_calendar(item, window.calendar.as_deref())));
        // The token goes wherever the next link points, so only Graph's own addresses are followed.
        match page["@odata.nextLink"].as_str() {
            Some(next) if found.len() < wanted && next.starts_with(API) => url = next.into(),
            _ => break,
        }
    }
    found.truncate(wanted);
    Ok(json!({"events": found}))
}

/// [`event`] as read in UTC from the calendar asked for.
fn in_calendar(item: &Value, calendar: Option<&str>) -> Value {
    let mut made = event(item);
    made["time_zone"] = json!("UTC");
    made["calendar"] = json!(calendar);
    made
}

fn event_path(id: &str) -> String {
    format!("{API}/me/events/{}", part(id))
}

/// The id to change and, with `all` on an occurrence, the occurrence read to find its series.
async fn series_of(cx: &Context, event: &str, all: bool) -> Result<(String, Option<Value>), Error> {
    if !all {
        return Ok((event.into(), None));
    }
    let select = query(&[("$select", "seriesMasterId,originalStart")]);
    let found = cx
        .json(cx.get(&format!("{}{select}", event_path(event)))?)
        .await?;
    Ok(match found["seriesMasterId"].as_str() {
        Some(master) => (master.into(), Some(found)),
        None => (event.into(), None),
    })
}

/// Graph takes an all-day event's dates as midnights.
fn stamp(text: &str, all_day: bool) -> Result<String, Error> {
    match (all_day, text.len()) {
        (true, 10) => Ok(format!("{text}T00:00:00")),
        (true, _) => Err(Error::Invalid(
            "all-day events take dates, such as 2026-11-05".into(),
        )),
        _ => Ok(text.into()),
    }
}

fn wall_clock(text: &str) -> Result<NaiveDateTime, Error> {
    NaiveDateTime::parse_from_str(text.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S%.f")
        .map_err(|_| Error::Invalid(format!("{text} is not a local time")))
}

fn zone(name: &str) -> Result<chrono_tz::Tz, Error> {
    crate::recurrence::zone(name)
        .ok_or_else(|| Error::Invalid(format!("{name} is not a time zone")))
}

/// The series' new start and end: its first occurrence moved as far as the edited occurrence on
/// the wall clock of the zone, keeping the new length.
async fn shifted(
    cx: &Context,
    (master, occurrence): (&str, &Value),
    (start, end): (&str, &str),
    zone_name: Option<&str>,
) -> Result<(Value, Value), Error> {
    let select = query(&[("$select", "start")]);
    let first = cx
        .json(cx.get(&format!("{}{select}", event_path(master)))?)
        .await?;
    let own = first["start"]["timeZone"].as_str();
    let name = zone_name
        .or(own)
        .ok_or_else(|| Error::Invalid("the series has no time zone; pass time_zone".into()))?;
    let (tz, own_tz) = (zone(name)?, zone(own.unwrap_or(name))?);
    let originally = occurrence["originalStart"].as_str().unwrap_or_default();
    let was = tz.from_utc_datetime(&wall_clock(originally)?).naive_local();
    let began = wall_clock(first["start"]["dateTime"].as_str().unwrap_or_default())?;
    let began = own_tz
        .from_local_datetime(&began)
        .single()
        .ok_or_else(|| Error::Invalid("the series start is not a time".into()))?
        .with_timezone(&tz)
        .naive_local();
    let (start, end) = (wall_clock(start)?, wall_clock(end)?);
    let from = began + (start - was);
    let render = |at: NaiveDateTime| json!({"dateTime": at.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": name});
    Ok((render(from), render(from + (end - start))))
}

/// The fields to patch apart from the times, and the stamped times with whether they are dates.
fn edits(change: &Change) -> Result<(serde_json::Map<String, Value>, Option<Edited>), Error> {
    let mut body = serde_json::Map::new();
    let text = change.subject.as_ref().map(|text| json!(text));
    let notes = change
        .body
        .as_ref()
        .map(|text| json!({"contentType": "Text", "content": text}));
    let place = change
        .location
        .as_ref()
        .map(|place| json!({"displayName": place}));
    for (key, value) in [("subject", text), ("body", notes), ("location", place)] {
        body.extend(value.map(|value| (key.into(), value)));
    }
    let times = match (&change.start, &change.end) {
        (Some(start), Some(end)) => {
            let all_day = change.all_day.unwrap_or(false);
            Some((stamp(start, all_day)?, stamp(end, all_day)?, all_day))
        }
        (None, None) => None,
        _ => return Err(Error::Invalid("start and end are given together".into())),
    };
    if body.is_empty() && times.is_none() {
        return Err(Error::Invalid("nothing to change".into()));
    }
    Ok((body, times))
}

/// A start, an end and whether they are all-day dates.
type Edited = (String, String, bool);

/// Patches the event, or with `all` the series its occurrence belongs to.
async fn update_event(cx: &Context, input: Value) -> Result<Value, Error> {
    let change: Change = crate::input(input)?;
    let (mut body, times) = edits(&change)?;
    let (id, occurrence) = series_of(cx, &change.event, change.all).await?;
    if let Some((start, end, all_day)) = times {
        let zone = change.time_zone.as_deref();
        let (start, end) = match &occurrence {
            Some(found) => shifted(cx, (&id, found), (&start, &end), zone).await?,
            None => {
                let zone = zone.unwrap_or("UTC");
                (
                    json!({"dateTime": start, "timeZone": zone}),
                    json!({"dateTime": end, "timeZone": zone}),
                )
            }
        };
        body.insert("isAllDay".into(), json!(all_day));
        body.insert("start".into(), start);
        body.insert("end".into(), end);
    }
    let request = cx.request(Method::PATCH, &event_path(&id))?;
    Ok(event(&cx.json(request.json(&body)).await?))
}

/// Deletes the event, or with `all` the series its occurrence belongs to.
async fn delete_event(cx: &Context, input: Value) -> Result<Value, Error> {
    let removal: Removal = crate::input(input)?;
    let (id, _) = series_of(cx, &removal.event, removal.all).await?;
    cx.send(cx.request(Method::DELETE, &event_path(&id))?)
        .await?;
    Ok(json!({"deleted": id}))
}

async fn create_event(cx: &Context, input: Value, invite: bool) -> Result<Value, Error> {
    let new: NewEvent = crate::input(input)?;
    if new.attendees.is_empty() == invite {
        let wrong = match invite {
            true => "calendar_invite needs attendees",
            false => "inviting attendees is calendar_invite's job",
        };
        return Err(Error::Invalid(wrong.into()));
    }
    let zone = new.time_zone.as_deref().unwrap_or("UTC");
    let mut body = json!({
        "subject": new.subject,
        "start": {"dateTime": new.start, "timeZone": zone},
        "end": {"dateTime": new.end, "timeZone": zone},
        "attendees": new.attendees.iter().map(|address| json!({"emailAddress": {"address": address}, "type": "required"})).collect::<Vec<_>>(),
    });
    if let Some(text) = new.body {
        body["body"] = json!({"contentType": "Text", "content": text});
    }
    if let Some(place) = new.location {
        body["location"] = json!({"displayName": place});
    }
    let made = cx
        .json(cx.post(&format!("{API}/me/events"))?.json(&body))
        .await?;
    Ok(event(&made))
}

async fn respond(cx: &Context, input: Value) -> Result<Value, Error> {
    let respond: Respond = crate::input(input)?;
    if !["accept", "decline", "tentativelyAccept"].contains(&respond.response.as_str()) {
        return Err(Error::Invalid(
            "response is accept, decline or tentativelyAccept".into(),
        ));
    }
    let url = format!(
        "{API}/me/events/{}/{}",
        part(&respond.event),
        respond.response
    );
    let body = json!({"comment": respond.comment.unwrap_or_default(), "sendResponse": true});
    cx.json(cx.post(&url)?.json(&body)).await?;
    Ok(json!({"responded": respond.response}))
}

fn drive_item(item: &Value) -> Value {
    json!({
        "id": item["id"],
        "name": item["name"],
        "folder": item["folder"].is_object(),
        "size": item["size"],
        "type": item["file"]["mimeType"],
        "modified": item["lastModifiedDateTime"],
        "path": item["parentReference"]["path"],
        "link": item["webUrl"],
    })
}

async fn files_search(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: FileSearch = crate::input(input)?;
    let words = encode(&search.query.replace('\'', "''"), b"'");
    let select = "id,name,size,file,folder,lastModifiedDateTime,parentReference,webUrl";
    let pairs = [
        ("$top", limit(search.limit, 25)),
        ("$select", select.into()),
    ];
    let pairs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let url = format!("{API}/me/drive/root/search(q='{words}'){}", query(&pairs));
    let found = cx.json(cx.get(&url)?).await?;
    Ok(json!({"items": list(&found, drive_item)}))
}

async fn files_read(cx: &Context, input: Value) -> Result<Value, Error> {
    let id = part(&crate::input::<Item>(input)?.item);
    let item = cx
        .json(cx.get(&format!("{API}/me/drive/items/{id}"))?)
        .await?;
    let Some(download) = item["@microsoft.graph.downloadUrl"].as_str() else {
        return Err(Error::Invalid(
            "not a file, or one that cannot be downloaded".into(),
        ));
    };
    // The download address is pre-authenticated on another host, so the Graph token stays home.
    let anonymous = Context::new(
        Value::Null,
        Credential::Fields {
            fields: Default::default(),
        },
    );
    let (bytes, _) = anonymous
        .bytes(anonymous.get(cx.url(download)?.as_str())?)
        .await?;
    let name = item["name"].as_str().unwrap_or("file");
    let kind = item["file"]["mimeType"].as_str();
    let texty = kind.is_some_and(|kind| {
        kind.starts_with("text/") || kind.contains("json") || kind.contains("xml")
    });
    match std::str::from_utf8(&bytes) {
        Ok(text) if texty => {
            let (text, truncated) = bounded(text);
            Ok(json!({"name": name, "text": text, "truncated": truncated}))
        }
        _ => Ok(crate::file(name, kind, &bytes)),
    }
}

async fn files_upload(cx: &Context, input: Value) -> Result<Value, Error> {
    let upload: Upload = crate::input(input)?;
    let bytes = match (upload.text, upload.base64) {
        (Some(text), None) => text.into_bytes(),
        (None, Some(data)) => base64::engine::general_purpose::STANDARD
            .decode(data.trim())
            .map_err(|error| Error::Invalid(format!("base64: {error}")))?,
        _ => return Err(Error::Invalid("give either text or base64".into())),
    };
    if bytes.len() > LARGEST_UPLOAD {
        return Err(Error::Invalid("larger than 4 MB".into()));
    }
    let path: Vec<String> = upload
        .path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(part)
        .collect();
    if path.is_empty() {
        return Err(Error::Invalid(
            "path names a file, such as /notes.md".into(),
        ));
    }
    let url = format!("{API}/me/drive/root:/{}:/content", path.join("/"));
    let request = cx
        .request(Method::PUT, &url)?
        .header("content-type", "application/octet-stream");
    let made = cx.json(request.body(bytes)).await?;
    Ok(drive_item(&made))
}

async fn todo_lists(cx: &Context) -> Result<Value, Error> {
    let found = cx.json(cx.get(&format!("{API}/me/todo/lists"))?).await?;
    let lists = list(
        &found,
        |list| json!({"id": list["id"], "name": list["displayName"], "kind": list["wellknownListName"]}),
    );
    Ok(json!({"lists": lists}))
}

fn task(task: &Value) -> Value {
    json!({
        "id": task["id"],
        "title": task["title"],
        "status": task["status"],
        "importance": task["importance"],
        "due": task["dueDateTime"]["dateTime"],
    })
}

async fn todo_tasks(cx: &Context, input: Value) -> Result<Value, Error> {
    let tasks: Tasks = crate::input(input)?;
    let mut pairs = vec![("$top", limit(tasks.limit, 50))];
    if !tasks.completed {
        pairs.push(("$filter", "status ne 'completed'".into()));
    }
    let pairs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let url = format!(
        "{API}/me/todo/lists/{}/tasks{}",
        part(&tasks.list),
        query(&pairs)
    );
    let found = cx.json(cx.get(&url)?).await?;
    Ok(json!({"tasks": list(&found, task)}))
}

async fn todo_create(cx: &Context, input: Value) -> Result<Value, Error> {
    let new: NewTask = crate::input(input)?;
    let mut body = json!({"title": new.title});
    if let Some(text) = new.body {
        body["body"] = json!({"contentType": "text", "content": text});
    }
    if let Some(due) = new.due {
        body["dueDateTime"] = json!({"dateTime": format!("{due}T00:00:00"), "timeZone": "UTC"});
    }
    if let Some(importance) = new.importance {
        body["importance"] = json!(importance);
    }
    let url = format!("{API}/me/todo/lists/{}/tasks", part(&new.list));
    let made = cx.json(cx.post(&url)?.json(&body)).await?;
    Ok(task(&made))
}

async fn todo_complete(cx: &Context, input: Value) -> Result<Value, Error> {
    let done: Task = crate::input(input)?;
    let url = format!(
        "{API}/me/todo/lists/{}/tasks/{}",
        part(&done.list),
        part(&done.task)
    );
    let body = json!({"status": "completed"});
    let changed = cx
        .json(cx.request(Method::PATCH, &url)?.json(&body))
        .await?;
    Ok(task(&changed))
}

/// The cursor is the newest `receivedDateTime` seen and the ids received at that second: Graph
/// times have one-second precision, so the next poll asks from that second and skips those ids.
fn parse_cursor(cursor: &str) -> Option<(&str, Vec<&str>)> {
    let (since, ids) = cursor.split_once(' ')?;
    // It goes into a filter, so only a timestamp's characters are allowed.
    let timestamp = since
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-:.".contains(&b));
    timestamp.then(|| (since, ids.split(',').filter(|id| !id.is_empty()).collect()))
}

fn cursor(messages: &[Value], since: &str) -> String {
    let newest = messages
        .last()
        .and_then(|message| message["receivedDateTime"].as_str())
        .unwrap_or(since);
    let ids: Vec<&str> = messages
        .iter()
        .filter(|message| message["receivedDateTime"] == newest)
        .filter_map(|message| message["id"].as_str())
        .collect();
    format!("{newest} {}", ids.join(","))
}

async fn inbox(cx: &Context, pairs: &[(&str, &str)]) -> Result<Vec<Value>, Error> {
    let url = format!("{API}{INBOX}{}", query(pairs));
    let found = cx.json(cx.get(&url)?).await?;
    Ok(found["value"].as_array().cloned().unwrap_or_default())
}

async fn baseline(cx: &Context) -> Result<Poll, Error> {
    let pairs = [
        ("$select", "id,receivedDateTime"),
        ("$orderby", "receivedDateTime desc"),
        ("$top", "1"),
    ];
    let latest = inbox(cx, &pairs).await?;
    Ok(Poll {
        events: Vec::new(),
        cursor: Some(cursor(&latest, EPOCH)),
    })
}

async fn received(cx: &Context, since: &str, seen: &[&str]) -> Result<Poll, Error> {
    let filter = format!("receivedDateTime ge {since}");
    let pairs = [
        ("$select", "id,from,subject,receivedDateTime,bodyPreview"),
        ("$filter", filter.as_str()),
        ("$orderby", "receivedDateTime asc"),
        ("$top", "50"),
    ];
    let messages = inbox(cx, &pairs).await?;
    let events = messages
        .iter()
        .filter(|message| !seen.contains(&message["id"].as_str().unwrap_or_default()))
        .map(|message| Event {
            kind: "mail.received",
            id: message["id"].as_str().unwrap_or_default().into(),
            summary: format!(
                "Mail from {}: {}",
                message["from"]["emailAddress"]["address"]
                    .as_str()
                    .unwrap_or("unknown"),
                message["subject"].as_str().unwrap_or_default()
            ),
            data: summary(message),
        })
        .collect();
    let next = match messages.is_empty() {
        true => format!("{since} {}", seen.join(",")),
        false => cursor(&messages, since),
    };
    Ok(Poll {
        events,
        cursor: Some(next),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Exchange, Fixture};

    fn cx(fixture: &Fixture) -> Context {
        let credential = Credential::Token {
            token: "graph-token".into(),
        };
        Context::new(json!({"tenant": "common"}), credential).with_origin(&fixture.origin())
    }

    /// Shapes from the Graph API reference examples.
    fn recorded(name: &str) -> Value {
        let path = format!(
            "{}/fixtures/microsoft/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn graph(method: &str, path: &str, body: Value) -> Exchange {
        Exchange::new(
            method,
            &format!("/graph.microsoft.com/v1.0{path}"),
            200,
            body,
        )
    }

    #[tokio::test]
    async fn mail_search_searches_without_ordering_and_lists_newest_first_otherwise() {
        let fixture = Fixture::start(vec![
            graph("GET", "/me/messages", recorded("messages")),
            graph(
                "GET",
                "/me/mailFolders/inbox/messages",
                recorded("messages"),
            ),
        ])
        .await;
        let cx = cx(&fixture);
        let found = Microsoft
            .call(&cx, "mail_search", json!({"query": "pizza", "limit": 5}))
            .await
            .unwrap();
        assert_eq!(found["messages"][0]["from"], "megan@contoso.com");
        assert_eq!(found["messages"][0]["subject"], "Lunch on Friday?");
        Microsoft
            .call(&cx, "mail_search", json!({"folder": "inbox"}))
            .await
            .unwrap();
        let seen = fixture.finish().await;
        assert!(
            seen[0].target.contains("$search=%22pizza%22"),
            "{}",
            seen[0].target
        );
        assert!(seen[0].target.contains("$top=5"));
        assert!(!seen[0].target.contains("$orderby"));
        assert!(seen[1].target.contains("$orderby=receivedDateTime%20desc"));
    }

    #[tokio::test]
    async fn mail_read_asks_for_a_text_body_and_lists_attachments() {
        let fixture = Fixture::start(vec![
            graph("GET", "/me/messages/AAMkAD1", recorded("message")),
            graph(
                "GET",
                "/me/messages/AAMkAD1/attachments",
                recorded("attachments"),
            ),
        ])
        .await;
        let read = Microsoft
            .call(&cx(&fixture), "mail_read", json!({"message": "AAMkAD1"}))
            .await
            .unwrap();
        assert_eq!(read["body"], "Shall we get pizza on Friday?\r\n");
        assert_eq!(read["to"], json!(["adele@contoso.com"]));
        assert_eq!(read["attachments"][0]["name"], "menu.pdf");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].header("prefer"),
            Some("outlook.body-content-type=\"text\"")
        );
        assert_eq!(seen[0].header("authorization"), Some("Bearer graph-token"));
    }

    #[tokio::test]
    async fn mail_send_posts_a_message_with_recipients() {
        let fixture = Fixture::start(vec![Exchange::new(
            "POST",
            "/graph.microsoft.com/v1.0/me/sendMail",
            202,
            Value::Null,
        )])
        .await;
        let input = json!({"to": ["ana@example.com"], "cc": ["bo@example.com"], "subject": "Hi", "body": "Hello."});
        let sent = Microsoft
            .call(&cx(&fixture), "mail_send", input)
            .await
            .unwrap();
        assert_eq!(sent, json!({"sent": true}));
        let body = fixture.finish().await[0].json();
        assert_eq!(
            body,
            json!({"saveToSentItems": true, "message": {
                "subject": "Hi",
                "body": {"contentType": "Text", "content": "Hello."},
                "toRecipients": [{"emailAddress": {"address": "ana@example.com"}}],
                "ccRecipients": [{"emailAddress": {"address": "bo@example.com"}}],
                "bccRecipients": [],
            }})
        );
    }

    #[tokio::test]
    async fn calendar_events_reads_a_window_and_creating_refuses_attendees() {
        let fixture = Fixture::start(vec![graph(
            "GET",
            "/me/calendarView",
            recorded("calendar_view"),
        )])
        .await;
        let cx = cx(&fixture);
        let input = json!({"start": "2026-11-05T00:00:00Z", "end": "2026-11-06T00:00:00Z"});
        let found = Microsoft.call(&cx, "calendar_events", input).await.unwrap();
        assert_eq!(found["events"][0]["subject"], "Plan summer company picnic");
        assert_eq!(found["events"][0]["organizer"], "dana@contoso.com");
        assert_eq!(found["events"][0]["attendees"], json!(["alex@contoso.com"]));
        let invite = json!({"subject": "s", "start": "a", "end": "b", "attendees": ["x@y.z"]});
        let refused = Microsoft.call(&cx, "calendar_create_event", invite).await;
        assert!(matches!(refused, Err(Error::Invalid(_))));
        let seen = fixture.finish().await;
        assert!(
            seen[0]
                .target
                .contains("startDateTime=2026-11-05T00:00:00Z")
        );
    }

    #[tokio::test]
    async fn calendar_list_reads_calendars() {
        let fixture =
            Fixture::start(vec![graph("GET", "/me/calendars", recorded("calendars"))]).await;
        let found = Microsoft
            .call(&cx(&fixture), "calendar_list", json!({}))
            .await
            .unwrap();
        assert_eq!(
            found["calendars"],
            json!([
                {"id": "AAMkAGI2TGuLAAA=", "name": "Calendar", "color": "auto", "can_edit": true},
                {"id": "AAMkAGI2THVSAAA=", "name": "Birthdays", "color": "#f4a6c8", "can_edit": false},
            ])
        );
    }

    #[tokio::test]
    async fn calendar_events_follows_next_links_in_utc() {
        let mut first = recorded("calendar_view");
        first["value"][0]["type"] = json!("occurrence");
        first["value"][0]["seriesMasterId"] = json!("AAMkMaster");
        first["@odata.nextLink"] = json!(
            "https://graph.microsoft.com/v1.0/me/calendars/cal%201/calendarView?$skiptoken=page2"
        );
        let mut second = recorded("calendar_view");
        second["value"][0]["id"] = json!("AAMkSecond");
        let calendar = "/me/calendars/cal%201/calendarView";
        let fixture = Fixture::start(vec![
            graph("GET", calendar, first),
            graph("GET", calendar, second),
        ])
        .await;
        let input = json!({"start": "2026-11-05T00:00:00Z", "end": "2026-11-06T00:00:00Z", "calendar": "cal 1", "limit": 500});
        let found = Microsoft
            .call(&cx(&fixture), "calendar_events", input)
            .await
            .unwrap();
        let events = found["events"].as_array().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["series"], "AAMkMaster");
        assert_eq!(events[1]["id"], "AAMkSecond");
        assert_eq!(events[1]["series"], Value::Null);
        assert_eq!(events[0]["calendar"], "cal 1");
        assert_eq!(events[0]["time_zone"], "UTC");
        let seen = fixture.finish().await;
        assert!(seen[1].target.contains("$skiptoken=page2"));
        for each in &seen {
            assert_eq!(each.header("prefer"), Some("outlook.timezone=\"UTC\""));
        }
    }

    #[tokio::test]
    async fn calendar_update_event_patches() {
        let made = recorded("calendar_view")["value"][0].clone();
        let fixture = Fixture::start(vec![graph("PATCH", "/me/events/AAMkOcc", made)]).await;
        let cx = cx(&fixture);
        let input = json!({"event": "AAMkOcc", "subject": "Picnic", "location": "Park", "start": "2026-11-05T09:00:00", "end": "2026-11-05T10:00:00", "time_zone": "Europe/Lisbon"});
        let changed = Microsoft
            .call(&cx, "calendar_update_event", input)
            .await
            .unwrap();
        assert_eq!(changed["subject"], "Plan summer company picnic");
        let nothing = Microsoft
            .call(&cx, "calendar_update_event", json!({"event": "AAMkOcc"}))
            .await;
        assert!(matches!(nothing, Err(Error::Invalid(m)) if m == "nothing to change"));
        let half = json!({"event": "AAMkOcc", "start": "2026-11-05T09:00:00"});
        let refused = Microsoft.call(&cx, "calendar_update_event", half).await;
        assert!(matches!(refused, Err(Error::Invalid(_))));
        let sent = fixture.finish().await[0].json();
        assert_eq!(
            sent,
            json!({
                "subject": "Picnic",
                "location": {"displayName": "Park"},
                "isAllDay": false,
                "start": {"dateTime": "2026-11-05T09:00:00", "timeZone": "Europe/Lisbon"},
                "end": {"dateTime": "2026-11-05T10:00:00", "timeZone": "Europe/Lisbon"},
            })
        );
    }

    #[tokio::test]
    async fn calendar_update_event_switches_to_all_day() {
        let made = recorded("calendar_view")["value"][0].clone();
        let fixture = Fixture::start(vec![graph("PATCH", "/me/events/AAMkOne", made)]).await;
        let input = json!({"event": "AAMkOne", "start": "2026-11-16", "end": "2026-11-17", "all_day": true});
        Microsoft
            .call(&cx(&fixture), "calendar_update_event", input)
            .await
            .unwrap();
        let sent = fixture.finish().await[0].json();
        assert_eq!(sent["isAllDay"], true);
        assert_eq!(
            sent["start"],
            json!({"dateTime": "2026-11-16T00:00:00", "timeZone": "UTC"})
        );
        assert_eq!(sent["end"]["dateTime"], "2026-11-17T00:00:00");
    }

    #[tokio::test]
    async fn calendar_update_event_with_all_keeps_wall_clock_across_dst() {
        let occurrence = json!({"id": "AAMkOcc", "type": "occurrence", "seriesMasterId": "AAMkMaster", "originalStart": "2026-10-31T13:00:00Z"});
        let master = json!({"id": "AAMkMaster", "start": {"dateTime": "2026-10-25T09:00:00.0000000", "timeZone": "Eastern Standard Time"}, "end": {"dateTime": "2026-10-25T10:00:00.0000000", "timeZone": "Eastern Standard Time"}});
        let fixture = Fixture::start(vec![
            graph("GET", "/me/events/AAMkOcc", occurrence),
            graph("GET", "/me/events/AAMkMaster", master.clone()),
            graph("PATCH", "/me/events/AAMkMaster", master),
        ])
        .await;
        // Oct 31 moves to Nov 2 at 09:00 EST: two days on the wall clock, 49 hours in absolute time.
        let input = json!({"event": "AAMkOcc", "all": true, "start": "2026-11-02T09:00:00", "end": "2026-11-02T10:00:00", "time_zone": "America/New_York"});
        Microsoft
            .call(&cx(&fixture), "calendar_update_event", input)
            .await
            .unwrap();
        let sent = fixture.finish().await[2].json();
        let zone = "America/New_York";
        assert_eq!(
            sent["start"],
            json!({"dateTime": "2026-10-27T09:00:00", "timeZone": zone})
        );
        assert_eq!(
            sent["end"],
            json!({"dateTime": "2026-10-27T10:00:00", "timeZone": zone})
        );
    }

    #[tokio::test]
    async fn calendar_update_event_with_all_patches_the_master_for_text() {
        let occurrence = json!({"id": "AAMkOcc", "seriesMasterId": "AAMkMaster"});
        let fixture = Fixture::start(vec![
            graph("GET", "/me/events/AAMkOcc", occurrence),
            graph("PATCH", "/me/events/AAMkMaster", json!({})),
        ])
        .await;
        let input = json!({"event": "AAMkOcc", "all": true, "subject": "Standup"});
        Microsoft
            .call(&cx(&fixture), "calendar_update_event", input)
            .await
            .unwrap();
        let seen = fixture.finish().await;
        assert_eq!(seen[1].json(), json!({"subject": "Standup"}));
    }

    #[tokio::test]
    async fn calendar_delete_event_deletes() {
        let occurrence = json!({"id": "AAMkOcc", "seriesMasterId": "AAMkMaster"});
        let fixture = Fixture::start(vec![
            graph("DELETE", "/me/events/AAMkOcc", Value::Null),
            graph("GET", "/me/events/AAMkOcc", occurrence),
            graph("DELETE", "/me/events/AAMkMaster", Value::Null),
        ])
        .await;
        let cx = cx(&fixture);
        let one = json!({"event": "AAMkOcc"});
        let gone = Microsoft.call(&cx, "calendar_delete_event", one).await;
        assert_eq!(gone.unwrap(), json!({"deleted": "AAMkOcc"}));
        let series = json!({"event": "AAMkOcc", "all": true});
        let gone = Microsoft.call(&cx, "calendar_delete_event", series).await;
        assert_eq!(gone.unwrap(), json!({"deleted": "AAMkMaster"}));
        fixture.finish().await;
    }

    #[tokio::test]
    async fn files_read_saves_a_binary_file_without_sending_the_token_to_the_download_host() {
        let fixture = Fixture::start(vec![
            graph("GET", "/me/drive/items/01BYE5RZ", recorded("drive_item")),
            Exchange::raw(
                "GET",
                "/public.dm.files.1drv.com/y4mPhoto",
                200,
                "\u{1}\u{2}\u{ff}",
            ),
        ])
        .await;
        let read = Microsoft
            .call(&cx(&fixture), "files_read", json!({"item": "01BYE5RZ"}))
            .await
            .unwrap();
        let file = &read[crate::FILE];
        assert_eq!(file["name"], "photo.png");
        assert_eq!(file["content_type"], "image/png");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(file["base64"].as_str().unwrap())
            .unwrap();
        assert_eq!(bytes, "\u{1}\u{2}\u{ff}".as_bytes());
        let seen = fixture.finish().await;
        assert_eq!(seen[1].header("authorization"), None);
    }

    #[tokio::test]
    async fn the_first_poll_takes_a_baseline_and_later_ones_report_only_new_mail() {
        let fixture = Fixture::start(vec![
            graph("GET", INBOX, recorded("inbox_latest")),
            graph("GET", INBOX, recorded("inbox_new")),
        ])
        .await;
        let cx = cx(&fixture);
        let first = Microsoft.poll(&cx, None).await.unwrap();
        assert!(first.events.is_empty());
        let cursor = first.cursor.unwrap();
        assert_eq!(cursor, "2026-11-05T10:00:00Z AAMkOld");
        let second = Microsoft.poll(&cx, Some(&cursor)).await.unwrap();
        let ids: Vec<&str> = second
            .events
            .iter()
            .map(|event| event.id.as_str())
            .collect();
        assert_eq!(
            ids,
            ["AAMkNew1", "AAMkNew2"],
            "the baseline message is skipped"
        );
        assert_eq!(second.events[0].kind, "mail.received");
        assert_eq!(second.events[0].data["subject"], "Quarterly report");
        assert_eq!(
            second.cursor.as_deref(),
            Some("2026-11-05T10:05:00Z AAMkNew2")
        );
        let seen = fixture.finish().await;
        assert!(seen[0].target.contains("$top=1"));
        assert!(
            seen[1]
                .target
                .contains("$filter=receivedDateTime%20ge%202026-11-05T10:00:00Z"),
            "{}",
            seen[1].target
        );
    }

    #[test]
    fn a_disabled_product_is_neither_offered_nor_scoped() {
        let config = json!({"products": ["mail", "calendar"]});
        let offered: Vec<&str> = MANIFEST
            .offered(&config, Level::Full, &[])
            .map(|tool| tool.name)
            .collect();
        assert!(offered.contains(&"mail_send"));
        assert!(
            !offered
                .iter()
                .any(|name| name.starts_with("files") || name.starts_with("todo"))
        );
        let Auth::OAuth2(oauth) = &MANIFEST.auth[0] else {
            panic!("Microsoft signs in with OAuth");
        };
        let scopes = MANIFEST.scopes(oauth, &config);
        assert!(scopes.contains(&"Mail.Send") && scopes.contains(&"offline_access"));
        assert!(!scopes.contains(&"Files.ReadWrite") && !scopes.contains(&"Tasks.ReadWrite"));
    }
}

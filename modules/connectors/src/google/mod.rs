//! Google through the person's own Desktop app OAuth client: one connection for Gmail, Calendar,
//! Drive, Contacts, Sheets and Docs. Only the ticked products' scopes are requested and only
//! their tools offered. Everything Google answers is the account's content, so it is untrusted.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, EventSpec, Field, FieldKind, Level,
    Manifest, OAuth2, Poll, Product, Risk, Scope, Step, ToolSpec,
};
use base64::{
    Engine,
    alphabet::URL_SAFE,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use reqwest::Url;
use serde_json::{Value, json};

mod calendar;
mod contacts;
mod documents;
mod drive;
mod gmail;
#[cfg(test)]
mod tests;

pub struct Google;

/// Gmail sends base64url with or without padding.
const BASE64URL: GeneralPurpose = GeneralPurpose::new(
    &URL_SAFE,
    GeneralPurposeConfig::new()
        .with_encode_padding(false)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent),
);
/// The most text one document, file or message body returns to the model.
const LONGEST_TEXT: usize = 50_000;

static MANIFEST: Manifest = Manifest {
    id: "google",
    name: "Google",
    description: "Gmail, Calendar, Drive, Contacts, Sheets and Docs through your own Google OAuth client.",
    category: Category::MailCalendar,
    auth: &[Auth::OAuth2(OAuth2 {
        authorize: "https://accounts.google.com/o/oauth2/v2/auth",
        token: "https://oauth2.googleapis.com/token",
        // Google's device flow does not allow the Gmail or Calendar scopes.
        device: None,
        revoke: Some("https://oauth2.googleapis.com/revoke"),
        scopes: &[
            Scope {
                scope: "openid",
                product: None,
            },
            Scope {
                scope: "email",
                product: None,
            },
            // The full scope, because the Email connector signs in to IMAP with XOAUTH2 through it.
            Scope {
                scope: "https://mail.google.com/",
                product: Some("gmail"),
            },
            Scope {
                scope: "https://www.googleapis.com/auth/calendar",
                product: Some("calendar"),
            },
            Scope {
                scope: "https://www.googleapis.com/auth/drive",
                product: Some("drive"),
            },
            Scope {
                scope: "https://www.googleapis.com/auth/contacts.readonly",
                product: Some("contacts"),
            },
            Scope {
                scope: "https://www.googleapis.com/auth/spreadsheets",
                product: Some("sheets"),
            },
            Scope {
                scope: "https://www.googleapis.com/auth/documents",
                product: Some("docs"),
            },
        ],
        // A refresh token, and a fresh consent so Google issues one on every sign-in.
        params: &[("access_type", "offline"), ("prompt", "consent")],
        fields: &[
            Field {
                key: "client_id",
                label: "Client ID",
                kind: FieldKind::Text,
                required: true,
                placeholder: "….apps.googleusercontent.com",
                help: "From the Desktop app OAuth client you created.",
            },
            Field {
                key: "client_secret",
                label: "Client secret",
                kind: FieldKind::Secret,
                required: true,
                placeholder: "GOCSPX-…",
                help: "Shown when the client is created.",
            },
        ],
    })],
    config: &[],
    products: &[
        Product {
            id: "gmail",
            name: "Gmail",
        },
        Product {
            id: "calendar",
            name: "Calendar",
        },
        Product {
            id: "drive",
            name: "Drive",
        },
        Product {
            id: "contacts",
            name: "Contacts",
        },
        Product {
            id: "sheets",
            name: "Sheets",
        },
        Product {
            id: "docs",
            name: "Docs",
        },
    ],
    tools: &[
        ToolSpec {
            name: "gmail_search",
            description: "Find messages with Gmail's search syntax, such as `from:ana is:unread newer_than:7d`. Newest first.",
            risk: Risk::Read,
            product: Some("gmail"),
            input: r#"{"type":"object","properties":{"query":{"type":"string","description":"Gmail search syntax; empty lists recent mail."},"limit":{"type":"integer","minimum":1,"maximum":25}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "gmail_read_thread",
            description: "A thread's messages: headers, plain text body and the attachments each one carries.",
            risk: Risk::Read,
            product: Some("gmail"),
            input: r#"{"type":"object","properties":{"thread":{"type":"string","description":"The thread id from gmail_search."}},"required":["thread"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "gmail_list_labels",
            description: "The mailbox's labels with their ids, for searching and gmail_modify.",
            risk: Risk::Read,
            product: Some("gmail"),
            input: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "gmail_send",
            description: "Send a plain text email, or a reply in a thread when reply_to names a message.",
            risk: Risk::Send,
            product: Some("gmail"),
            input: COMPOSE,
        },
        ToolSpec {
            name: "gmail_draft",
            description: "Save a plain text email as a draft without sending it; reply_to makes it a reply.",
            risk: Risk::Write,
            product: Some("gmail"),
            input: COMPOSE,
        },
        ToolSpec {
            name: "gmail_modify",
            description: "Add or remove labels on a message: archive removes INBOX, mark read removes UNREAD, trash adds TRASH.",
            risk: Risk::Write,
            product: Some("gmail"),
            input: r#"{"type":"object","properties":{"message":{"type":"string"},"add":{"type":"array","items":{"type":"string"},"description":"Label ids."},"remove":{"type":"array","items":{"type":"string"}}},"required":["message"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "gmail_save_attachment",
            description: "Save a message's attachment as a file in the workspace.",
            risk: Risk::Read,
            product: Some("gmail"),
            input: r#"{"type":"object","properties":{"message":{"type":"string"},"attachment":{"type":"string","description":"The attachment id from gmail_read_thread."},"filename":{"type":"string"}},"required":["message","attachment"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "calendar_list",
            description: "The calendars the account can see, with their ids and access.",
            risk: Risk::Read,
            product: Some("calendar"),
            input: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "calendar_events",
            description: "Events in order of start, from now unless `from` says otherwise, with recurring events expanded.",
            risk: Risk::Read,
            product: Some("calendar"),
            input: r#"{"type":"object","properties":{"calendar":{"type":"string","description":"A calendar id; primary when absent."},"from":{"type":"string","description":"RFC 3339, such as 2026-11-05T09:00:00-03:00."},"to":{"type":"string"},"query":{"type":"string","description":"Free text matched against the events."},"limit":{"type":"integer","minimum":1,"maximum":250},"page":{"type":"string","description":"The next_page of the previous answer."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "calendar_freebusy",
            description: "The busy periods of calendars between two times, for finding a free slot.",
            risk: Risk::Read,
            product: Some("calendar"),
            input: r#"{"type":"object","properties":{"from":{"type":"string","description":"RFC 3339."},"to":{"type":"string"},"calendars":{"type":"array","items":{"type":"string"},"description":"Calendar ids or email addresses; primary when absent."}},"required":["from","to"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "calendar_create_event",
            description: "Create an event on a calendar without notifying anyone; use calendar_invite to add guests.",
            risk: Risk::Write,
            product: Some("calendar"),
            input: r#"{"type":"object","properties":{"calendar":{"type":"string"},"summary":{"type":"string"},"start":{"type":"string","description":"RFC 3339, or YYYY-MM-DD for an all-day event."},"end":{"type":"string","description":"Exclusive: the day after for an all-day event."},"time_zone":{"type":"string","description":"An IANA zone such as America/Sao_Paulo."},"description":{"type":"string"},"location":{"type":"string"}},"required":["summary","start","end"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "calendar_update_event",
            description: "Change an event without notifying its guests. An occurrence's id changes only that occurrence, and with all the whole series, new times moving every occurrence as much as that one moves; all is ignored for an event that does not repeat.",
            risk: Risk::Write,
            product: Some("calendar"),
            input: r#"{"type":"object","properties":{"calendar":{"type":"string"},"event":{"type":"string"},"all":{"type":"boolean","description":"Change the whole series the occurrence belongs to."},"summary":{"type":"string"},"start":{"type":"string","description":"RFC 3339, or YYYY-MM-DD for an all-day event; given together with end."},"end":{"type":"string","description":"Exclusive: the day after for an all-day event."},"time_zone":{"type":"string","description":"An IANA zone such as America/Sao_Paulo."},"description":{"type":"string"},"location":{"type":"string"}},"required":["event"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "calendar_invite",
            description: "Add guests to an event and email them the invitation.",
            risk: Risk::Send,
            product: Some("calendar"),
            input: r#"{"type":"object","properties":{"calendar":{"type":"string"},"event":{"type":"string"},"attendees":{"type":"array","items":{"type":"string"},"minItems":1,"description":"Email addresses."}},"required":["event","attendees"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "calendar_respond",
            description: "Accept, decline or tentatively accept an invitation; the organizer is told.",
            risk: Risk::Send,
            product: Some("calendar"),
            input: r#"{"type":"object","properties":{"calendar":{"type":"string"},"event":{"type":"string"},"response":{"type":"string","enum":["accepted","declined","tentative"]}},"required":["event","response"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "calendar_delete_event",
            description: "Delete an event without notifying its guests.",
            risk: Risk::Write,
            product: Some("calendar"),
            input: r#"{"type":"object","properties":{"calendar":{"type":"string"},"event":{"type":"string"}},"required":["event"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "drive_search",
            description: "Find Drive files by name, or with Drive's query syntax such as `mimeType = 'application/pdf'`.",
            risk: Risk::Read,
            product: Some("drive"),
            input: r#"{"type":"object","properties":{"name":{"type":"string","description":"Text the file's name contains."},"query":{"type":"string","description":"Drive query syntax; used instead of name."},"limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "drive_read",
            description: "A file's text: Google Docs and Slides as plain text, Sheets as CSV of the first sheet; other files are saved to the workspace.",
            risk: Risk::Read,
            product: Some("drive"),
            input: r#"{"type":"object","properties":{"file":{"type":"string","description":"The file's id or link."}},"required":["file"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "drive_upload",
            description: "Create a Drive file from text, optionally inside a folder.",
            risk: Risk::Write,
            product: Some("drive"),
            input: r#"{"type":"object","properties":{"name":{"type":"string"},"text":{"type":"string"},"mime_type":{"type":"string","description":"text/plain when absent."},"folder":{"type":"string","description":"The parent folder's id or link."}},"required":["name","text"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "drive_share",
            description: "Share a file with someone by email; Google emails them the link.",
            risk: Risk::Send,
            product: Some("drive"),
            input: r#"{"type":"object","properties":{"file":{"type":"string"},"email":{"type":"string"},"role":{"type":"string","enum":["reader","commenter","writer"]},"message":{"type":"string","description":"A note in the email."}},"required":["file","email","role"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "contacts_search",
            description: "Find contacts by name, email or phone, with their emails and phone numbers.",
            risk: Risk::Read,
            product: Some("contacts"),
            input: r#"{"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":30}},"required":["query"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "sheets_read",
            description: "The values in a spreadsheet range, such as `Sheet1!A1:D20`, as rows.",
            risk: Risk::Read,
            product: Some("sheets"),
            input: r#"{"type":"object","properties":{"spreadsheet":{"type":"string","description":"The spreadsheet's id or link."},"range":{"type":"string","description":"A1 notation; a sheet name alone reads the whole sheet."}},"required":["spreadsheet","range"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "sheets_update",
            description: "Overwrite a range with rows of values, parsed as if typed: formulas and dates work.",
            risk: Risk::Write,
            product: Some("sheets"),
            input: VALUES,
        },
        ToolSpec {
            name: "sheets_append",
            description: "Add rows after the last row of the table in a range, parsed as if typed.",
            risk: Risk::Write,
            product: Some("sheets"),
            input: VALUES,
        },
        ToolSpec {
            name: "docs_read",
            description: "A Google Doc's title and its text, tables included.",
            risk: Risk::Read,
            product: Some("docs"),
            input: r#"{"type":"object","properties":{"document":{"type":"string","description":"The document's id or link."}},"required":["document"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "docs_append",
            description: "Add text to the end of a Google Doc.",
            risk: Risk::Write,
            product: Some("docs"),
            input: r#"{"type":"object","properties":{"document":{"type":"string"},"text":{"type":"string"}},"required":["document","text"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "docs_create",
            description: "Create a Google Doc with a title and optional text, in the root of Drive.",
            risk: Risk::Write,
            product: Some("docs"),
            input: r#"{"type":"object","properties":{"title":{"type":"string"},"text":{"type":"string"}},"required":["title"],"additionalProperties":false}"#,
        },
    ],
    events: &[
        EventSpec {
            kind: "mail.received",
            description: "A message arrived in the inbox.",
            product: Some("gmail"),
        },
        EventSpec {
            kind: "event.starting",
            description: "An event on the primary calendar starts within 15 minutes.",
            product: Some("calendar"),
        },
    ],
    // Sending email is on by default (docs/research/connections.md, decision D2).
    level: Level::Full,
    guide: &[
        Step {
            title: "Create a Google Cloud project",
            body: "Sign in to Google Cloud with the account you are connecting, create a project (any name, such as Cubebot) and keep it selected. An existing project works too.",
            link: Some("https://console.cloud.google.com/projectcreate"),
            copy: None,
        },
        Step {
            title: "Turn on the APIs",
            body: "This link turns on the Gmail, Google Calendar, Google Drive, People (Contacts), Google Sheets and Google Docs APIs in one go; an API you never use costs nothing. To turn on only some, find them by name under APIs & Services › Library.",
            link: Some(
                "https://console.cloud.google.com/flows/enableapi?apiid=gmail.googleapis.com,calendar-json.googleapis.com,drive.googleapis.com,people.googleapis.com,sheets.googleapis.com,docs.googleapis.com",
            ),
            copy: None,
        },
        Step {
            title: "Configure the OAuth consent screen",
            body: "Choose Get started, name the app and give your email for support and contact. Under Audience choose External (a Workspace account can choose Internal). Then either add your own address under Test users, or choose Publish app to put it In production so sign-ins do not expire after 7 days.",
            link: Some("https://console.cloud.google.com/auth/overview"),
            copy: None,
        },
        Step {
            title: "List the scopes",
            body: "Under Data access choose Add or remove scopes and paste these under Manually add scopes. Sign-in asks only for the ones of the products you tick.",
            link: Some("https://console.cloud.google.com/auth/scopes"),
            copy: Some(
                "openid email https://mail.google.com/ https://www.googleapis.com/auth/calendar https://www.googleapis.com/auth/drive https://www.googleapis.com/auth/contacts.readonly https://www.googleapis.com/auth/spreadsheets https://www.googleapis.com/auth/documents",
            ),
        },
        Step {
            title: "Create a Desktop app client",
            body: "Create an OAuth client with Application type Desktop app. Google sends you back to {redirect_uri}; a Desktop client accepts loopback addresses without registering them, so there is no redirect address to add.",
            link: Some("https://console.cloud.google.com/auth/clients/create"),
            copy: None,
        },
        Step {
            title: "Copy the client ID and secret",
            body: "Paste the Client ID and Client secret below. Google shows the secret only while creating the client, so copy it, or download the JSON, before closing the dialog.",
            link: None,
            copy: None,
        },
        Step {
            title: "Sign in",
            body: "Tick the products to connect and sign in. Google warns that the app is not verified because it is your own: choose Advanced, then Go to the app, and allow the access asked for.",
            link: None,
            copy: None,
        },
    ],
    warning: Some(
        "While the OAuth consent screen is in Testing, Google expires refresh tokens after 7 days and the connection must sign in again. Set the app to In production under Audience: an unverified app then shows a warning screen at sign-in and is limited to 100 users, which is fine for personal use. A Google Workspace account can choose Internal instead and avoid both.",
    ),
};

const COMPOSE: &str = r#"{"type":"object","properties":{"to":{"type":"array","items":{"type":"string"},"description":"Addresses, such as ana@example.com or Ana <ana@example.com>."},"cc":{"type":"array","items":{"type":"string"}},"bcc":{"type":"array","items":{"type":"string"}},"subject":{"type":"string","description":"Re: and the original subject when replying and absent."},"text":{"type":"string"},"reply_to":{"type":"string","description":"The id of the message this answers; keeps the thread."}},"required":["to","text"],"additionalProperties":false}"#;

const VALUES: &str = r#"{"type":"object","properties":{"spreadsheet":{"type":"string"},"range":{"type":"string"},"values":{"type":"array","items":{"type":"array","items":{}},"description":"Rows of cell values."}},"required":["spreadsheet","range","values"],"additionalProperties":false}"#;

impl Connector for Google {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let me = cx
                .json(cx.get("https://openidconnect.googleapis.com/v1/userinfo")?)
                .await?;
            let name = me["email"].as_str().unwrap_or("Google account");
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool.split_once('_') {
                Some(("gmail", name)) => gmail::call(cx, name, input).await,
                Some(("calendar", name)) => calendar::call(cx, name, input).await,
                Some(("drive", name)) => drive::call(cx, name, input).await,
                Some(("contacts", "search")) => contacts::search(cx, input).await,
                Some(("sheets" | "docs", _)) => documents::call(cx, tool, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    /// The cursor is a JSON object with a part per polled product, so each keeps its own place
    /// and a product switched on later starts from now.
    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(async move {
            let last: Value = cursor
                .and_then(|cursor| serde_json::from_str(cursor).ok())
                .unwrap_or_default();
            let enabled = MANIFEST.enabled(cx.config());
            let (mut events, mut next) = (Vec::new(), json!({}));
            if enabled.contains(&"gmail") {
                next["gmail"] = gmail::poll(cx, last["gmail"].as_str(), &mut events).await?;
            }
            if enabled.contains(&"calendar") {
                next["calendar"] = calendar::poll(cx, &last["calendar"], &mut events).await?;
            }
            Ok(Poll {
                events,
                cursor: Some(next.to_string()),
            })
        })
    }
}

/// `base` with each segment appended and escaped, so an id cannot change the path.
fn at(base: &str, segments: &[&str]) -> String {
    let mut url = Url::parse(base).expect("a valid base address");
    url.path_segments_mut()
        .expect("an address with a path")
        .extend(segments);
    url.into()
}

/// The id in a Google link such as `https://docs.google.com/document/d/<id>/edit`, or the text.
fn id(text: &str) -> &str {
    text.split_once("/d/")
        .and_then(|(_, rest)| rest.split(['/', '?', '#']).next())
        .unwrap_or(text)
        .trim()
}

/// At most `most` characters, marked when cut.
fn clip(text: &str, most: usize) -> String {
    match text.char_indices().nth(most) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

fn decode(data: &str) -> Result<Vec<u8>, Error> {
    BASE64URL
        .decode(data.trim())
        .map_err(|_| Error::Unavailable("Google sent data that is not base64url".into()))
}

/// Seconds since the Unix epoch.
fn now() -> i64 {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    since.map_or(0, |since| since.as_secs() as i64)
}

fn rfc3339(seconds: i64) -> String {
    mail_parser::DateTime::from_timestamp(seconds).to_rfc3339()
}

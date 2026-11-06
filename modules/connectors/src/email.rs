//! Email over IMAP (RFC 9051) and SMTP submission (RFC 6409): one connector for Gmail, Outlook,
//! iCloud, Fastmail, Yahoo, Proton Bridge and self-hosted servers. Gmail can also sign in with
//! XOAUTH2 using a Google connection's token, and its IMAP extensions give threads and labels.
use crate::{
    Account, Answer, Auth, Category, Choice, Connector, Context, Credential, Error, Event,
    EventSpec, Field, FieldKind, Level, Manifest, Poll, Risk, Step, ToolSpec,
};
use async_imap::{
    Client, Session,
    error::Error as ImapError,
    imap_proto::{AttributeValue, Response},
    types::{Fetch, Flag, Name, NameAttribute},
};
use futures_util::TryStreamExt;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
    transport::smtp::{
        authentication::{Credentials, Mechanism},
        client::{Tls, TlsParameters},
    },
};
use mail_parser::{HeaderValue, MessageParser, MimeHeaders};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    hash::BuildHasher,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
};

/// The most messages a search, a thread or a poll answers with.
const MOST: usize = 50;
const EVENTS: usize = 20;
const TEXT: usize = 100 * 1024;
const THREAD_TEXT: usize = 20 * 1024;
const SNIPPET: usize = 200;
/// Enough of a message for its headers and a snippet.
const SUMMARY: &str = "(UID FLAGS BODY.PEEK[]<0.16384>)";
/// A whole message, bounded; `PEEK` leaves it unread.
const WHOLE: &str = "(UID FLAGS BODY.PEEK[]<0.26214400>)";
const LONGEST: Duration = Duration::from_secs(60);

/// (provider, IMAP host, SMTP host, SMTP port); IMAP is on 993 everywhere.
const PROVIDERS: &[(&str, &str, &str, u16)] = &[
    ("gmail", "imap.gmail.com", "smtp.gmail.com", 465),
    (
        "outlook",
        "outlook.office365.com",
        "smtp.office365.com",
        587,
    ),
    ("icloud", "imap.mail.me.com", "smtp.mail.me.com", 587),
    ("fastmail", "imap.fastmail.com", "smtp.fastmail.com", 465),
    ("yahoo", "imap.mail.yahoo.com", "smtp.mail.yahoo.com", 465),
];

pub struct Email;

static MANIFEST: Manifest = Manifest {
    id: "email",
    name: "Email",
    description: "Read, search, file and send mail over IMAP and SMTP, from any provider.",
    category: Category::MailCalendar,
    auth: &[
        Auth::Password {
            fields: &[
                Field {
                    key: "username",
                    label: "Username",
                    kind: FieldKind::Text,
                    required: false,
                    placeholder: "you@example.com",
                    help: "Usually the address itself; leave empty to use it.",
                },
                Field {
                    key: "password",
                    label: "Password",
                    kind: FieldKind::Secret,
                    required: true,
                    placeholder: "",
                    help: "An app password: Gmail, iCloud, Yahoo and Fastmail refuse the account password.",
                },
            ],
        },
        Auth::Linked {
            connectors: &["google"],
            product: "gmail",
        },
    ],
    config: &[
        Field {
            key: "provider",
            label: "Provider",
            kind: FieldKind::Choice(&[
                Choice {
                    value: "gmail",
                    label: "Gmail",
                },
                Choice {
                    value: "outlook",
                    label: "Outlook and Microsoft 365",
                },
                Choice {
                    value: "icloud",
                    label: "iCloud",
                },
                Choice {
                    value: "fastmail",
                    label: "Fastmail",
                },
                Choice {
                    value: "yahoo",
                    label: "Yahoo",
                },
                Choice {
                    value: "other",
                    label: "Other, such as Proton Bridge or your own server",
                },
            ]),
            required: true,
            placeholder: "",
            help: "Fills in the servers below when they are left empty.",
        },
        Field {
            key: "address",
            label: "Address",
            kind: FieldKind::Text,
            required: true,
            placeholder: "you@example.com",
            help: "Mail is sent from this address.",
        },
        Field {
            key: "display_name",
            label: "Display name",
            kind: FieldKind::Text,
            required: false,
            placeholder: "Ada Lovelace",
            help: "Shown beside the address on sent mail.",
        },
        Field {
            key: "imap_host",
            label: "IMAP server",
            kind: FieldKind::Text,
            required: false,
            placeholder: "imap.example.com",
            help: "",
        },
        Field {
            key: "imap_port",
            label: "IMAP port",
            kind: FieldKind::Number,
            required: false,
            placeholder: "993",
            help: "",
        },
        Field {
            key: "smtp_host",
            label: "SMTP server",
            kind: FieldKind::Text,
            required: false,
            placeholder: "smtp.example.com",
            help: "",
        },
        Field {
            key: "smtp_port",
            label: "SMTP port",
            kind: FieldKind::Number,
            required: false,
            placeholder: "465",
            help: "",
        },
        Field {
            key: "security",
            label: "Security",
            kind: FieldKind::Choice(&[
                Choice {
                    value: "tls",
                    label: "TLS (STARTTLS on SMTP port 587)",
                },
                Choice {
                    value: "starttls",
                    label: "STARTTLS",
                },
                Choice {
                    value: "none",
                    label: "None, for a bridge on 127.0.0.1 only",
                },
            ]),
            required: false,
            placeholder: "tls",
            help: "",
        },
    ],
    products: &[],
    tools: &[
        ToolSpec {
            name: "search",
            description: "Find messages in a folder, newest first, by sender, recipient, subject, text, date or unread.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"folder":{"type":"string","description":"Defaults to INBOX."},"from":{"type":"string"},"to":{"type":"string"},"subject":{"type":"string"},"text":{"type":"string","description":"Words anywhere in the message."},"since":{"type":"string","description":"YYYY-MM-DD, inclusive."},"before":{"type":"string","description":"YYYY-MM-DD, exclusive."},"unseen":{"type":"boolean"},"limit":{"type":"integer","minimum":1,"maximum":50}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_message",
            description: "A message's headers, its text and its attachments, without marking it read.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"folder":{"type":"string","description":"Defaults to INBOX."},"uid":{"type":"integer","minimum":1}},"required":["uid"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_thread",
            description: "Every message of the conversation a message belongs to, oldest first, within its folder.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"folder":{"type":"string","description":"Defaults to INBOX."},"uid":{"type":"integer","minimum":1}},"required":["uid"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_folders",
            description: "The account's folders, with the special ones such as sent, drafts and trash marked.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "save_attachment",
            description: "Save one of a message's attachments, by its index from read_message, as a file.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"folder":{"type":"string","description":"Defaults to INBOX."},"uid":{"type":"integer","minimum":1},"index":{"type":"integer","minimum":0}},"required":["uid","index"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "draft",
            description: "Save a plain-text message in the Drafts folder without sending it.",
            risk: Risk::Write,
            product: None,
            input: COMPOSE,
        },
        ToolSpec {
            name: "move",
            description: "Move a message to another folder, such as an archive or the trash.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"folder":{"type":"string","description":"Defaults to INBOX."},"uid":{"type":"integer","minimum":1},"destination":{"type":"string"}},"required":["uid","destination"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "label",
            description: "Add or remove flags such as \\Seen and \\Flagged, or Gmail labels, on a message.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"folder":{"type":"string","description":"Defaults to INBOX."},"uid":{"type":"integer","minimum":1},"add":{"type":"array","items":{"type":"string"},"maxItems":20},"remove":{"type":"array","items":{"type":"string"},"maxItems":20}},"required":["uid"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "send",
            description: "Send a plain-text message from the connection's address.",
            risk: Risk::Send,
            product: None,
            input: COMPOSE,
        },
        ToolSpec {
            name: "reply",
            description: "Reply in plain text to a message, keeping it in the same conversation.",
            risk: Risk::Send,
            product: None,
            input: r#"{"type":"object","properties":{"folder":{"type":"string","description":"Defaults to INBOX."},"uid":{"type":"integer","minimum":1},"text":{"type":"string"},"reply_all":{"type":"boolean","description":"Also send to the other recipients."}},"required":["uid","text"],"additionalProperties":false}"#,
        },
    ],
    events: &[EventSpec {
        kind: "mail.received",
        description: "New mail arrived in the inbox.",
        product: None,
    }],
    level: Level::Full,
    guide: &[Step {
        title: "Create an app password",
        body: "Most providers refuse the account password over IMAP. Turn on two-step verification, create an app password for Cubebot, and paste it below. With a Google connection that has Gmail switched on, sign in through it instead.",
        link: None,
        copy: None,
    }],
    warning: Some(
        "Mail is written by strangers. An agent that reads it can be steered by it, so lower the level or switch off send when that matters.",
    ),
};

const COMPOSE: &str = r#"{"type":"object","properties":{"to":{"type":"array","items":{"type":"string"},"minItems":1,"description":"Addresses, optionally as Name <address>."},"cc":{"type":"array","items":{"type":"string"}},"bcc":{"type":"array","items":{"type":"string"}},"subject":{"type":"string"},"text":{"type":"string"}},"required":["to","subject","text"],"additionalProperties":false}"#;

fn inbox() -> String {
    "INBOX".into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    #[serde(default = "inbox")]
    folder: String,
    from: Option<String>,
    to: Option<String>,
    subject: Option<String>,
    text: Option<String>,
    since: Option<String>,
    before: Option<String>,
    #[serde(default)]
    unseen: bool,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct At {
    #[serde(default = "inbox")]
    folder: String,
    uid: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attachment {
    #[serde(default = "inbox")]
    folder: String,
    uid: u32,
    index: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Compose {
    to: Vec<String>,
    #[serde(default)]
    cc: Vec<String>,
    #[serde(default)]
    bcc: Vec<String>,
    subject: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    #[serde(default = "inbox")]
    folder: String,
    uid: u32,
    text: String,
    #[serde(default)]
    reply_all: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Move {
    #[serde(default = "inbox")]
    folder: String,
    uid: u32,
    destination: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Label {
    #[serde(default = "inbox")]
    folder: String,
    uid: u32,
    #[serde(default)]
    add: Vec<String>,
    #[serde(default)]
    remove: Vec<String>,
}

/// The headers that put a reply in its conversation.
struct Threading {
    in_reply_to: String,
    references: String,
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send + std::fmt::Debug {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + std::fmt::Debug> Io for T {}
type Imap = Session<Box<dyn Io>>;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Security {
    Tls,
    StartTls,
    Plain,
}

#[derive(Debug, PartialEq)]
struct Server {
    host: String,
    port: u16,
    security: Security,
}

impl Connector for Email {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let mut imap = patiently(open(cx)).await?;
            imap.logout().await.map_err(failed(cx, false))?;
            let name = cx.setting("address").map_or_else(|| user(cx), Ok)?;
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            let risk = MANIFEST.tool(tool).ok_or(Error::UnknownTool)?.risk;
            let work = async {
                match tool {
                    "send" => send(cx, input).await,
                    _ => run(cx, tool, input).await,
                }
            };
            match tokio::time::timeout(LONGEST, work).await {
                Ok(answer) => answer,
                Err(_) if risk == Risk::Read => Err(Error::Unavailable("timed out".into())),
                Err(_) => Err(Error::Uncertain),
            }
        })
    }

    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(async move {
            let mut imap = patiently(open(cx)).await?;
            let polled = patiently(new_mail(cx, &mut imap, cursor)).await;
            let _ = imap.logout().await;
            polled
        })
    }
}

/// A read that gives up rather than hang on a silent server.
async fn patiently<T>(work: impl Future<Output = Result<T, Error>>) -> Result<T, Error> {
    tokio::time::timeout(LONGEST, work)
        .await
        .unwrap_or_else(|_| Err(Error::Unavailable("timed out".into())))
}

/// A tool's input, checked before any connection is made.
enum Request {
    Search(Search, String),
    Message(At),
    Thread(At),
    Folders,
    Attachment(Attachment),
    Draft(Message, String),
    Move(Move),
    Label(Label),
    Reply(Reply),
}

fn request(cx: &Context, tool: &str, input: Value) -> Result<Request, Error> {
    match tool {
        "search" => crate::input(input).and_then(searching),
        "read_message" => crate::input(input).map(Request::Message),
        "read_thread" => crate::input(input).map(Request::Thread),
        "list_folders" => Ok(Request::Folders),
        "save_attachment" => crate::input(input).map(Request::Attachment),
        "draft" => crate::input(input)
            .and_then(|compose| message(cx, &compose, None, true))
            .map(|(message, id)| Request::Draft(message, id)),
        "move" => crate::input(input).map(Request::Move),
        "label" => crate::input(input).and_then(labelling),
        "reply" => crate::input(input).map(Request::Reply),
        _ => Err(Error::UnknownTool),
    }
}

fn searching(search: Search) -> Result<Request, Error> {
    let query = criteria(&search)?;
    Ok(Request::Search(search, query))
}

fn labelling(label: Label) -> Result<Request, Error> {
    if label.add.len() + label.remove.len() > 40 {
        return Err(Error::Invalid("at most 40 flags at once".into()));
    }
    Ok(Request::Label(label))
}

async fn send(cx: &Context, input: Value) -> Result<Value, Error> {
    let (message, id) = message(cx, &crate::input(input)?, None, false)?;
    deliver(cx, message).await?;
    Ok(json!({"sent": true, "message_id": id}))
}

async fn run(cx: &Context, tool: &str, input: Value) -> Result<Value, Error> {
    let request = request(cx, tool, input)?;
    let mut imap = open(cx).await?;
    let imap = &mut imap;
    let answer = match request {
        Request::Search(search, query) => find(cx, imap, search, &query).await,
        Request::Message(at) => read_message(cx, imap, at).await,
        Request::Thread(at) => read_thread(cx, imap, at).await,
        Request::Folders => list_folders(cx, imap).await,
        Request::Attachment(wanted) => save_attachment(cx, imap, wanted).await,
        Request::Draft(message, id) => draft(cx, imap, message, id).await,
        Request::Move(order) => move_message(cx, imap, order).await,
        Request::Label(label) => relabel(cx, imap, label).await,
        Request::Reply(reply) => answer_to(cx, imap, reply).await,
    };
    // Logging out is a courtesy; the answer is already settled.
    let _ = imap.logout().await;
    answer
}

fn address(cx: &Context) -> Result<&str, Error> {
    cx.setting("address")
        .ok_or_else(|| Error::Invalid("the connection has no address".into()))
}

/// Who signs in: the address for XOAUTH2, else the username when it differs.
fn user(cx: &Context) -> Result<&str, Error> {
    match cx.credential() {
        Credential::Token { .. } => address(cx),
        Credential::Fields { .. } => cx.setting("username").map_or_else(|| address(cx), Ok),
    }
}

fn server(cx: &Context, smtp: bool) -> Result<Server, Error> {
    let (host_key, port_key) = match smtp {
        true => ("smtp_host", "smtp_port"),
        false => ("imap_host", "imap_port"),
    };
    let known = PROVIDERS
        .iter()
        .find(|known| cx.setting("provider") == Some(known.0));
    let host = cx
        .setting(host_key)
        .or(known.map(|known| if smtp { known.2 } else { known.1 }))
        .ok_or_else(|| Error::Invalid(format!("the connection has no {host_key}")))?;
    let fallback = match (smtp, known) {
        (false, _) => 993,
        (true, Some(known)) => known.3,
        (true, None) => 465,
    };
    let port = port(cx, port_key)?.unwrap_or(fallback);
    let security = security(cx, host, smtp && port == 587)?;
    Ok(Server {
        host: host.to_owned(),
        port,
        security,
    })
}

fn security(cx: &Context, host: &str, submission: bool) -> Result<Security, Error> {
    match cx.setting("security").unwrap_or("tls") {
        // Submission on 587 is STARTTLS by definition (RFC 8314).
        "tls" if submission => Ok(Security::StartTls),
        "tls" => Ok(Security::Tls),
        "starttls" => Ok(Security::StartTls),
        "none" if host == "127.0.0.1" => Ok(Security::Plain),
        "none" => Err(Error::Invalid(
            "security none is allowed only for a server on 127.0.0.1".into(),
        )),
        other => Err(Error::Invalid(format!("unknown security {other:?}"))),
    }
}

/// A port setting, stored as a number or as text.
fn port(cx: &Context, key: &str) -> Result<Option<u16>, Error> {
    let value = &cx.config()[key];
    let number = match value.as_str() {
        Some(text) if text.trim().is_empty() => return Ok(None),
        Some(text) => text.trim().parse().ok(),
        None if value.is_null() => return Ok(None),
        None => value.as_u64(),
    };
    number
        .and_then(|number| u16::try_from(number).ok())
        .map(Some)
        .ok_or_else(|| Error::Invalid(format!("{key} is not a port")))
}

async fn tls(stream: Box<dyn Io>, host: &str) -> Result<Box<dyn Io>, Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| Error::Unavailable(format!("TLS: {error}")))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.to_owned())
        .map_err(|_| Error::Invalid(format!("{host:?} is not a server name")))?;
    let stream = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(name, stream)
        .await
        .map_err(|error| Error::Unavailable(format!("TLS with the IMAP server failed: {error}")))?;
    Ok(Box::new(stream))
}

async fn open(cx: &Context) -> Result<Imap, Error> {
    let server = server(cx, false)?;
    let tcp = TcpStream::connect((server.host.as_str(), server.port))
        .await
        .map_err(|error| Error::Unavailable(format!("could not reach the IMAP server: {error}")))?;
    let mut stream: Box<dyn Io> = Box::new(tcp);
    if server.security == Security::Tls {
        stream = tls(stream, &server.host).await?;
    }
    let mut client = Client::new(stream);
    let greeting = client.read_response().await;
    if !matches!(greeting, Ok(Some(_))) {
        return Err(Error::Unavailable(
            "the IMAP server sent no greeting".into(),
        ));
    }
    if server.security == Security::StartTls {
        client
            .run_command_and_check_ok("STARTTLS", None)
            .await
            .map_err(failed(cx, false))?;
        // No greeting follows STARTTLS.
        client = Client::new(tls(client.into_inner(), &server.host).await?);
    }
    login(cx, client).await
}

async fn login(cx: &Context, client: Client<Box<dyn Io>>) -> Result<Imap, Error> {
    let user = user(cx)?;
    let session = match cx.credential() {
        Credential::Token { token } => {
            let bearer = XOAuth2(format!("user={user}\x01auth=Bearer {token}\x01\x01"));
            client.authenticate("XOAUTH2", bearer).await
        }
        Credential::Fields { .. } => client.login(user, cx.secret("password")?).await,
    };
    session.map_err(|(error, _)| match error {
        ImapError::No(_) => Error::Unauthorized,
        other => failed(cx, false)(other),
    })
}

/// SASL XOAUTH2 (https://developers.google.com/gmail/imap/xoauth2-protocol).
struct XOAuth2(String);

impl async_imap::Authenticator for XOAuth2 {
    type Response = String;
    fn process(&mut self, challenge: &[u8]) -> String {
        // A refusal arrives as a JSON challenge; an empty answer gets the final NO.
        match challenge.is_empty() {
            true => std::mem::take(&mut self.0),
            false => String::new(),
        }
    }
}

/// An IMAP failure as the model reads it. `effect` when the command may have changed the mailbox.
fn failed(cx: &Context, effect: bool) -> impl Fn(ImapError) -> Error + '_ {
    move |error| match error {
        ImapError::No(message) | ImapError::Bad(message) => Error::Rejected(clean(cx, &message)),
        ImapError::Validate(_) => Error::Invalid("a value holds a line break".into()),
        ImapError::Append => Error::Rejected("the server refused the message".into()),
        _ if effect => Error::Uncertain,
        other => Error::Unavailable(clean(cx, &other.to_string())),
    }
}

/// Server words on one bounded line, with any credential struck out.
fn clean(cx: &Context, text: &str) -> String {
    cx.credential()
        .redact(text)
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(300)
        .collect()
}

/// An IMAP quoted string. Non-ASCII text relies on servers accepting UTF-8 there, as Gmail and
/// Dovecot do.
fn quoted(text: &str) -> Result<String, Error> {
    if text.contains(['\r', '\n', '\0']) {
        return Err(Error::Invalid("a value holds a line break".into()));
    }
    Ok(format!(
        "\"{}\"",
        text.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

/// `YYYY-MM-DD` as IMAP's `D-Mon-YYYY`.
fn date(text: &str) -> Result<String, Error> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let bad = || Error::Invalid(format!("{text:?} is not a YYYY-MM-DD date"));
    let mut parts = text.splitn(3, '-').map(str::parse::<u32>);
    let (Some(Ok(year)), Some(Ok(month)), Some(Ok(day))) =
        (parts.next(), parts.next(), parts.next())
    else {
        return Err(bad());
    };
    let month = MONTHS
        .get(month.wrapping_sub(1) as usize)
        .filter(|_| (1..=31).contains(&day) && year >= 1000)
        .ok_or_else(bad)?;
    Ok(format!("{day}-{month}-{year}"))
}

fn criteria(search: &Search) -> Result<String, Error> {
    let mut keys = Vec::new();
    let texts = [
        ("FROM", &search.from),
        ("TO", &search.to),
        ("SUBJECT", &search.subject),
        ("TEXT", &search.text),
    ];
    for (key, value) in texts {
        if let Some(value) = value {
            keys.push(format!("{key} {}", quoted(value)?));
        }
    }
    for (key, value) in [("SINCE", &search.since), ("BEFORE", &search.before)] {
        if let Some(value) = value {
            keys.push(format!("{key} {}", date(value)?));
        }
    }
    if search.unseen {
        keys.push("UNSEEN".into());
    }
    let keys = keys.join(" ");
    Ok(match keys.as_str() {
        "" => "ALL".into(),
        _ if !keys.is_ascii() => format!("CHARSET UTF-8 {keys}"),
        _ => keys,
    })
}

async fn examine(cx: &Context, imap: &mut Imap, folder: &str) -> Result<(), Error> {
    imap.examine(folder).await.map_err(failed(cx, false))?;
    Ok(())
}

async fn has(cx: &Context, imap: &mut Imap, capability: &str) -> Result<bool, Error> {
    let capabilities = imap.capabilities().await.map_err(failed(cx, false))?;
    Ok(capabilities.has_str(capability))
}

async fn uids(cx: &Context, imap: &mut Imap, query: &str) -> Result<Vec<u32>, Error> {
    let found = imap.uid_search(query).await.map_err(failed(cx, false))?;
    let mut found: Vec<u32> = found.into_iter().collect();
    found.sort_unstable();
    Ok(found)
}

/// The messages `uids` names that still exist, in ascending UID order.
async fn fetch(
    cx: &Context,
    imap: &mut Imap,
    uids: &[u32],
    items: &str,
) -> Result<Vec<Fetch>, Error> {
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let set: Vec<String> = uids.iter().map(u32::to_string).collect();
    let stream = imap
        .uid_fetch(set.join(","), items)
        .await
        .map_err(failed(cx, false))?;
    let mut fetched: Vec<Fetch> = stream.try_collect().await.map_err(failed(cx, false))?;
    // Unsolicited flag updates carry no UID.
    fetched.retain(|fetch| fetch.uid.is_some_and(|uid| uids.contains(&uid)));
    fetched.sort_by_key(|fetch| fetch.uid);
    Ok(fetched)
}

async fn one(cx: &Context, imap: &mut Imap, uid: u32, items: &str) -> Result<Fetch, Error> {
    fetch(cx, imap, &[uid], items)
        .await?
        .pop()
        .ok_or_else(|| Error::NotFound(format!("message {uid}")))
}

fn parse(raw: &[u8]) -> Result<mail_parser::Message<'_>, Error> {
    MessageParser::default()
        .parse(raw)
        .ok_or_else(|| Error::Rejected("the message could not be parsed".into()))
}

fn flags(fetch: &Fetch) -> Vec<String> {
    fetch
        .flags()
        .map(|flag| match flag {
            Flag::Seen => "\\Seen".into(),
            Flag::Answered => "\\Answered".into(),
            Flag::Flagged => "\\Flagged".into(),
            Flag::Deleted => "\\Deleted".into(),
            Flag::Draft => "\\Draft".into(),
            Flag::Recent => "\\Recent".into(),
            Flag::MayCreate => "\\*".into(),
            Flag::Custom(name) => name.into_owned(),
        })
        .collect()
}

fn addresses(list: Option<&mail_parser::Address>) -> Vec<String> {
    list.into_iter()
        .flat_map(|list| list.iter())
        .take(MOST)
        .filter_map(|addr| {
            let address = addr.address()?;
            Some(match addr.name() {
                Some(name) => format!("{name} <{address}>"),
                None => address.to_owned(),
            })
        })
        .collect()
}

/// At most `limit` bytes of `text`, cut on a character boundary, and whether it was cut.
fn bounded(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_owned(), false);
    }
    let end = (0..=limit).rev().find(|end| text.is_char_boundary(*end));
    (text[..end.unwrap_or(0)].to_owned(), true)
}

fn headers(message: &mail_parser::Message) -> Value {
    json!({
        "from": addresses(message.from()),
        "to": addresses(message.to()),
        "cc": addresses(message.cc()),
        "subject": message.subject(),
        "date": message.date().map(|date| date.to_rfc3339()),
        "message_id": message.message_id(),
    })
}

/// What a list of messages shows of each: who, what, when, and the first words.
fn summary(fetch: &Fetch) -> Value {
    let mut shown = json!({"uid": fetch.uid, "flags": flags(fetch)});
    if let Some(message) = fetch
        .body()
        .and_then(|raw| MessageParser::default().parse(raw))
    {
        let preview = message.body_preview(SNIPPET).unwrap_or_default();
        let snippet: Vec<&str> = preview.split_whitespace().collect();
        let mut headers = headers(&message);
        for key in ["from", "to", "subject", "date"] {
            shown[key] = headers[key].take();
        }
        shown["snippet"] = json!(snippet.join(" "));
    }
    shown
}

async fn find(cx: &Context, imap: &mut Imap, search: Search, query: &str) -> Result<Value, Error> {
    examine(cx, imap, &search.folder).await?;
    let mut found = uids(cx, imap, query).await?;
    let total = found.len();
    found.reverse();
    found.truncate(search.limit.unwrap_or(20).clamp(1, MOST));
    let mut messages: Vec<Value> = fetch(cx, imap, &found, SUMMARY)
        .await?
        .iter()
        .map(summary)
        .collect();
    messages.reverse();
    Ok(json!({"folder": search.folder, "total": total, "messages": messages}))
}

async fn read_message(cx: &Context, imap: &mut Imap, at: At) -> Result<Value, Error> {
    examine(cx, imap, &at.folder).await?;
    let fetched = one(cx, imap, at.uid, WHOLE).await?;
    let message = parse(fetched.body().unwrap_or_default())?;
    let (text, truncated) = bounded(&message.body_text(0).unwrap_or_default(), TEXT);
    let attachments: Vec<Value> = message
        .attachments()
        .take(MOST)
        .enumerate()
        .map(|(index, part)| {
            json!({
                "index": index,
                "name": part.attachment_name(),
                "size": part.len(),
                "type": content_type(part),
            })
        })
        .collect();
    let mut read = headers(&message);
    read["uid"] = json!(at.uid);
    read["flags"] = json!(flags(&fetched));
    read["text"] = json!(text);
    read["truncated"] = json!(truncated);
    read["attachments"] = json!(attachments);
    Ok(read)
}

fn content_type(part: &mail_parser::MessagePart) -> Option<String> {
    let kind = part.content_type()?;
    Some(match kind.subtype() {
        Some(subtype) => format!("{}/{subtype}", kind.ctype()),
        None => kind.ctype().to_owned(),
    })
}

async fn save_attachment(
    cx: &Context,
    imap: &mut Imap,
    wanted: Attachment,
) -> Result<Value, Error> {
    examine(cx, imap, &wanted.folder).await?;
    let fetched = one(cx, imap, wanted.uid, WHOLE).await?;
    let message = parse(fetched.body().unwrap_or_default())?;
    let part = message
        .attachment(wanted.index)
        .ok_or_else(|| Error::NotFound(format!("attachment {}", wanted.index)))?;
    let name = part
        .attachment_name()
        .map_or_else(|| format!("attachment-{}", wanted.index), str::to_owned);
    Ok(crate::file(
        &name,
        content_type(part).as_deref(),
        part.contents(),
    ))
}

async fn read_thread(cx: &Context, imap: &mut Imap, at: At) -> Result<Value, Error> {
    let gmail = has(cx, imap, "X-GM-EXT-1").await?;
    examine(cx, imap, &at.folder).await?;
    let query = match gmail {
        true => format!("X-GM-THRID {}", thread_id(cx, imap, at.uid).await?),
        false => related(cx, imap, at.uid).await?,
    };
    let mut found = uids(cx, imap, &query).await?;
    if !found.contains(&at.uid) {
        found.push(at.uid);
        found.sort_unstable();
    }
    let latest = found.split_off(found.len().saturating_sub(MOST));
    let fetched = fetch(cx, imap, &latest, "(UID FLAGS BODY.PEEK[]<0.262144>)").await?;
    let messages: Vec<Value> = fetched
        .iter()
        .map(|fetched| {
            let mut shown = summary(fetched);
            let message = fetched
                .body()
                .and_then(|raw| MessageParser::default().parse(raw));
            let text =
                message.and_then(|message| message.body_text(0).map(|text| text.into_owned()));
            shown["text"] = json!(bounded(&text.unwrap_or_default(), THREAD_TEXT).0);
            shown
        })
        .collect();
    Ok(json!({"folder": at.folder, "omitted": found.len(), "messages": messages}))
}

/// Gmail's conversation id (https://developers.google.com/gmail/imap/imap-extensions), which
/// async-imap fetches but does not expose.
async fn thread_id(cx: &Context, imap: &mut Imap, uid: u32) -> Result<u64, Error> {
    let tag = imap
        .run_command(format!("UID FETCH {uid} (X-GM-THRID)"))
        .await
        .map_err(failed(cx, false))?;
    let mut found = None;
    loop {
        let response = imap
            .read_response()
            .await
            .map_err(|error| Error::Unavailable(clean(cx, &error.to_string())))?
            .ok_or_else(|| Error::Unavailable("the IMAP server closed the connection".into()))?;
        match response.parsed() {
            Response::Fetch(_, attributes) => {
                found = found.or(attributes.iter().find_map(|attribute| match attribute {
                    AttributeValue::GmailThrId(id) => Some(*id),
                    _ => None,
                }));
            }
            Response::Done { tag: done, .. } if *done == tag => break,
            _ => (),
        }
    }
    found.ok_or_else(|| Error::NotFound(format!("message {uid}")))
}

/// A search for the messages a message refers to and those that refer to its first ancestor.
async fn related(cx: &Context, imap: &mut Imap, uid: u32) -> Result<String, Error> {
    let fetched = one(cx, imap, uid, "(UID BODY.PEEK[HEADER])").await?;
    let message = parse(fetched.header().unwrap_or_default())?;
    let mut chain = ids(message.references());
    chain.extend(ids(message.in_reply_to()));
    chain.extend(message.message_id().map(str::to_owned));
    chain.dedup();
    let chain = chain.split_off(chain.len().saturating_sub(20));
    let Some(root) = chain.first() else {
        return Ok(format!("UID {uid}"));
    };
    let mut keys = vec![format!("HEADER References {}", quoted(root)?)];
    for id in &chain {
        keys.push(format!("HEADER Message-ID {}", quoted(id)?));
    }
    Ok(keys
        .into_iter()
        .reduce(|either, key| format!("OR {key} {either}"))
        .unwrap_or_default())
}

fn ids(value: &HeaderValue) -> Vec<String> {
    match value {
        HeaderValue::Text(id) => vec![id.to_string()],
        HeaderValue::TextList(list) => list.iter().map(|id| id.to_string()).collect(),
        _ => Vec::new(),
    }
}

async fn list_folders(cx: &Context, imap: &mut Imap) -> Result<Value, Error> {
    let names: Vec<Name> = imap
        .list(None, Some("*"))
        .await
        .map_err(failed(cx, false))?
        .try_collect()
        .await
        .map_err(failed(cx, false))?;
    let folders: Vec<Value> = names
        .iter()
        .take(500)
        .map(|name| json!({"name": name.name(), "special": special(name)}))
        .collect();
    Ok(json!({"folders": folders}))
}

/// The folder's special use (RFC 6154), such as `drafts`.
fn special(name: &Name) -> Option<&'static str> {
    name.attributes()
        .iter()
        .find_map(|attribute| match attribute {
            NameAttribute::All => Some("all"),
            NameAttribute::Archive => Some("archive"),
            NameAttribute::Drafts => Some("drafts"),
            NameAttribute::Flagged => Some("flagged"),
            NameAttribute::Junk => Some("junk"),
            NameAttribute::Sent => Some("sent"),
            NameAttribute::Trash => Some("trash"),
            _ => None,
        })
}

async fn draft(
    cx: &Context,
    imap: &mut Imap,
    message: Message,
    id: String,
) -> Result<Value, Error> {
    let names: Vec<Name> = imap
        .list(None, Some("*"))
        .await
        .map_err(failed(cx, false))?
        .try_collect()
        .await
        .map_err(failed(cx, false))?;
    let folder = names
        .iter()
        .find(|name| special(name) == Some("drafts"))
        .map_or("Drafts", Name::name)
        .to_owned();
    imap.append(&folder, Some("(\\Draft \\Seen)"), None, message.formatted())
        .await
        .map_err(failed(cx, true))?;
    Ok(json!({"folder": folder, "message_id": id}))
}

fn mailbox(text: &str) -> Result<Mailbox, Error> {
    text.parse()
        .map_err(|error| Error::Invalid(format!("{text:?} is not an address: {error}")))
}

fn message_id(domain: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    // RandomState is seeded from the OS, so ids stay unique without another dependency.
    let random = std::collections::hash_map::RandomState::new().hash_one(nanos);
    format!("<{nanos:x}.{random:x}@{domain}>")
}

/// A plain-text message from the connection's address, and its Message-ID. A draft keeps Bcc.
fn message(
    cx: &Context,
    compose: &Compose,
    threading: Option<Threading>,
    draft: bool,
) -> Result<(Message, String), Error> {
    let count = compose.to.len() + compose.cc.len() + compose.bcc.len();
    if compose.to.is_empty() || count > 100 {
        return Err(Error::Invalid(
            "between 1 and 100 recipients, at least one in to".into(),
        ));
    }
    let from = Mailbox::new(
        cx.setting("display_name").map(str::to_owned),
        address(cx)?
            .parse()
            .map_err(|_| Error::Invalid("the connection's address is not valid".into()))?,
    );
    let id = message_id(from.email.domain());
    let mut builder = Message::builder()
        .from(from)
        .subject(&compose.subject)
        .message_id(Some(id.clone()));
    for to in &compose.to {
        builder = builder.to(mailbox(to)?);
    }
    for cc in &compose.cc {
        builder = builder.cc(mailbox(cc)?);
    }
    for bcc in &compose.bcc {
        builder = builder.bcc(mailbox(bcc)?);
    }
    if let Some(threading) = threading {
        builder = builder
            .in_reply_to(threading.in_reply_to)
            .references(threading.references);
    }
    if draft {
        builder = builder.keep_bcc();
    }
    let message = builder
        .header(ContentType::TEXT_PLAIN)
        .body(compose.text.clone())
        .map_err(|error| Error::Invalid(error.to_string()))?;
    Ok((message, id))
}

async fn deliver(cx: &Context, message: Message) -> Result<(), Error> {
    let server = server(cx, true)?;
    let parameters = || {
        TlsParameters::new_rustls(server.host.clone())
            .map_err(|error| Error::Unavailable(format!("TLS: {error}")))
    };
    let tls = match server.security {
        Security::Tls => Tls::Wrapper(parameters()?),
        Security::StartTls => Tls::Required(parameters()?),
        Security::Plain => Tls::None,
    };
    let (credentials, mechanisms) = match cx.credential() {
        Credential::Token { token } => (
            Credentials::new(address(cx)?.into(), token.clone()),
            vec![Mechanism::Xoauth2],
        ),
        Credential::Fields { .. } => (
            Credentials::new(user(cx)?.into(), cx.secret("password")?.into()),
            vec![Mechanism::Plain, Mechanism::Login],
        ),
    };
    let transport = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&server.host)
        .port(server.port)
        .tls(tls)
        .credentials(credentials)
        .authentication(mechanisms)
        .timeout(Some(Duration::from_secs(30)))
        .build();
    transport
        .send(message)
        .await
        .map(drop)
        .map_err(|error| refused(cx, &error))
}

/// An SMTP failure. A reply code means the server refused before accepting the message.
fn refused(cx: &Context, error: &lettre::transport::smtp::Error) -> Error {
    let message = clean(cx, &error.to_string());
    let code = error.status().map(|code| code.to_string());
    match code.as_deref() {
        Some("530" | "534" | "535") => Error::Unauthorized,
        Some(_) if error.is_permanent() => Error::Rejected(message),
        Some(_) => Error::Unavailable(message),
        // lettre marks only its connect failures this way; nothing was sent.
        None if message.starts_with("Connection error") || error.is_tls() || error.is_client() => {
            Error::Unavailable(message)
        }
        None => Error::Uncertain,
    }
}

async fn answer_to(cx: &Context, imap: &mut Imap, reply: Reply) -> Result<Value, Error> {
    examine(cx, imap, &reply.folder).await?;
    let fetched = one(cx, imap, reply.uid, "(UID BODY.PEEK[HEADER])").await?;
    let (compose, threading) = answer(cx, &fetched, reply)?;
    let (message, id) = message(cx, &compose, Some(threading), false)?;
    deliver(cx, message).await?;
    Ok(json!({"sent": true, "message_id": id, "to": compose.to, "cc": compose.cc}))
}

/// The reply to a message's headers: to its Reply-To or sender, Re: its subject, in its thread.
fn answer(cx: &Context, fetched: &Fetch, reply: Reply) -> Result<(Compose, Threading), Error> {
    let original = parse(fetched.header().unwrap_or_default())?;
    let own = address(cx)?.to_ascii_lowercase();
    let mut to = addresses(original.reply_to().or(original.from()));
    let mut cc = Vec::new();
    if reply.reply_all {
        cc = addresses(original.to());
        cc.extend(addresses(original.cc()));
    }
    let mine = |text: &String| {
        let text = text.to_ascii_lowercase();
        text == own || text.ends_with(&format!("<{own}>"))
    };
    cc.retain(|address| !mine(address) && !to.contains(address));
    if to.iter().all(mine) && !cc.is_empty() {
        to = std::mem::take(&mut cc);
    }
    let subject = original.subject().unwrap_or_default();
    let subject = match subject.get(..3) {
        Some(re) if re.eq_ignore_ascii_case("re:") => subject.to_owned(),
        _ => format!("Re: {subject}"),
    };
    let id = original
        .message_id()
        .map(|id| format!("<{id}>"))
        .ok_or_else(|| Error::Rejected("the message has no Message-ID to reply to".into()))?;
    let mut references: Vec<String> = ids(original.references())
        .into_iter()
        .map(|id| format!("<{id}>"))
        .collect();
    references.push(id.clone());
    let compose = Compose {
        to,
        cc,
        bcc: Vec::new(),
        subject,
        text: reply.text,
    };
    let threading = Threading {
        in_reply_to: id,
        references: references.join(" "),
    };
    Ok((compose, threading))
}

async fn move_message(cx: &Context, imap: &mut Imap, order: Move) -> Result<Value, Error> {
    let can_move = has(cx, imap, "MOVE").await?;
    let can_expunge = has(cx, imap, "UIDPLUS").await?;
    imap.select(&order.folder)
        .await
        .map_err(failed(cx, false))?;
    one(cx, imap, order.uid, "(UID)").await?;
    let uid = order.uid.to_string();
    if can_move {
        imap.uid_mv(&uid, &order.destination)
            .await
            .map_err(failed(cx, true))?;
        return Ok(json!({"moved": true, "destination": order.destination}));
    }
    imap.uid_copy(&uid, &order.destination)
        .await
        .map_err(failed(cx, true))?;
    store(cx, imap, &uid, "+FLAGS (\\Deleted)").await?;
    // A plain EXPUNGE would also erase every other message marked deleted, so without UIDPLUS
    // the original stays, marked deleted.
    if can_expunge {
        let expunged = imap.uid_expunge(&uid).await.map_err(failed(cx, true))?;
        let _: Vec<u32> = expunged.try_collect().await.map_err(failed(cx, true))?;
    }
    Ok(json!({"moved": true, "destination": order.destination, "original_removed": can_expunge}))
}

async fn store(cx: &Context, imap: &mut Imap, uid: &str, change: &str) -> Result<(), Error> {
    let stored = imap
        .uid_store(uid, change)
        .await
        .map_err(failed(cx, true))?;
    let _: Vec<Fetch> = stored.try_collect().await.map_err(failed(cx, true))?;
    Ok(())
}

/// A flag as IMAP writes it: `\Seen` or a keyword atom.
fn atom(flag: &str) -> Result<&str, Error> {
    let name = flag.strip_prefix('\\').unwrap_or(flag);
    let valid = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"$_.-".contains(&b));
    valid
        .then_some(flag)
        .ok_or_else(|| Error::Invalid(format!("{flag:?} is not a flag")))
}

async fn relabel(cx: &Context, imap: &mut Imap, label: Label) -> Result<Value, Error> {
    let gmail = has(cx, imap, "X-GM-EXT-1").await?;
    imap.select(&label.folder)
        .await
        .map_err(failed(cx, false))?;
    one(cx, imap, label.uid, "(UID)").await?;
    let uid = label.uid.to_string();
    for (sign, names) in [('+', &label.add), ('-', &label.remove)] {
        // On Gmail a name without a backslash is a label; elsewhere it is a keyword.
        let (system, labels): (Vec<&String>, Vec<&String>) = names
            .iter()
            .partition(|name| name.starts_with('\\') || !gmail);
        if !system.is_empty() {
            let flags: Result<Vec<&str>, Error> = system.iter().map(|flag| atom(flag)).collect();
            store(
                cx,
                imap,
                &uid,
                &format!("{sign}FLAGS ({})", flags?.join(" ")),
            )
            .await?;
        }
        if !labels.is_empty() {
            let quoted: Result<Vec<String>, Error> =
                labels.iter().map(|name| quoted(name)).collect();
            let change = format!("{sign}X-GM-LABELS ({})", quoted?.join(" "));
            store(cx, imap, &uid, &change).await?;
        }
    }
    Ok(json!({"uid": label.uid, "added": label.add, "removed": label.remove}))
}

/// New inbox mail since `cursor`, `"<uidvalidity>:<uidnext>"`.
async fn new_mail(cx: &Context, imap: &mut Imap, cursor: Option<&str>) -> Result<Poll, Error> {
    let inbox = imap.examine("INBOX").await.map_err(failed(cx, false))?;
    let (Some(validity), Some(next)) = (inbox.uid_validity, inbox.uid_next) else {
        return Err(Error::Unavailable(
            "the inbox reports no UIDVALIDITY or UIDNEXT".into(),
        ));
    };
    let current = format!("{validity}:{next}");
    // A changed UIDVALIDITY renumbered the mailbox; starting over beats replaying it.
    let since = cursor
        .and_then(|cursor| cursor.split_once(':'))
        .filter(|(seen, _)| *seen == validity.to_string())
        .and_then(|(_, since)| since.parse::<u32>().ok())
        .filter(|since| *since < next);
    let Some(since) = since else {
        return Ok(Poll {
            events: Vec::new(),
            cursor: Some(current),
        });
    };
    // `n:*` always matches the last message, even below n.
    let mut found = uids(cx, imap, &format!("UID {since}:*")).await?;
    found.retain(|uid| *uid >= since);
    found.truncate(EVENTS);
    let cursor = match found.last() {
        Some(last) if found.len() == EVENTS => format!("{validity}:{}", last + 1),
        _ => current,
    };
    let events = fetch(cx, imap, &found, SUMMARY)
        .await?
        .iter()
        .map(|fetched| event(validity, fetched))
        .collect();
    Ok(Poll {
        events,
        cursor: Some(cursor),
    })
}

fn event(validity: u32, fetched: &Fetch) -> Event {
    let data = summary(fetched);
    let from = data["from"][0].as_str().unwrap_or("someone");
    let subject = data["subject"].as_str().unwrap_or("(no subject)");
    let summary: String = format!("New mail from {from}: {subject}")
        .chars()
        .take(SNIPPET)
        .collect();
    Event {
        kind: "mail.received",
        id: format!("{validity}:{}", fetched.uid.unwrap_or_default()),
        summary,
        data,
    }
}

#[cfg(test)]
mod tests {
    //! Transcripts follow what Dovecot 2.3, Gmail's IMAP and Postfix's submission service send,
    //! as seen in `openssl s_client` sessions, trimmed to the commands each tool issues.
    use super::*;
    use crate::testing::Script;
    use base64::Engine;
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::TcpListener,
        task::JoinHandle,
    };

    const ADA: &str = "ada@example.com";

    fn password() -> Credential {
        Credential::Fields {
            fields: [("password".into(), "app-password-123".into())].into(),
        }
    }

    fn cx(imap: u16, smtp: u16, credential: Credential) -> Context {
        let config = json!({
            "provider": "other", "address": ADA, "display_name": "Ada Lovelace",
            "imap_host": "127.0.0.1", "imap_port": imap,
            "smtp_host": "127.0.0.1", "smtp_port": smtp.to_string(), "security": "none",
        });
        Context::new(config, credential)
    }

    fn script(text: &str) -> Vec<String> {
        text.lines()
            .map(str::trim_start)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// Dovecot's greeting and a password login, as tag A0001.
    fn dovecot() -> Vec<String> {
        script(
            r#"S: * OK [CAPABILITY IMAP4rev1 SASL-IR LOGIN-REFERRALS ID ENABLE IDLE LITERAL+ AUTH=PLAIN] Dovecot ready.
            C: A0001 LOGIN "ada@example.com" "app-password-123"
            S: A0001 OK [CAPABILITY IMAP4rev1 SASL-IR LOGIN-REFERRALS ID ENABLE IDLE SORT THREAD=REFERENCES MOVE UIDPLUS SPECIAL-USE] Logged in"#,
        )
    }

    fn examine(tag: &str, next: u32) -> Vec<String> {
        script(&format!(
            r#"C: {tag} EXAMINE "INBOX"
            S: * FLAGS (\Answered \Flagged \Deleted \Seen \Draft)
            S: * OK [PERMANENTFLAGS ()] Read-only mailbox.
            S: * 3 EXISTS
            S: * 0 RECENT
            S: * OK [UIDVALIDITY 1700000000] UIDs valid
            S: * OK [UIDNEXT {next}] Predicted next UID
            S: {tag} OK [READ-ONLY] Examine completed (0.001 + 0.000 secs)."#
        ))
    }

    fn logout(tag: &str) -> Vec<String> {
        script(&format!(
            "C: {tag} LOGOUT\nS: * BYE Logging out\nS: {tag} OK Logout completed (0.001 + 0.000 secs)."
        ))
    }

    /// A FETCH response carrying `raw` as an IMAP literal with CRLF line ends.
    fn literal(head: &str, raw: &str) -> Vec<String> {
        let text = raw.replace("\r\n", "\n").replace('\n', "\r\n");
        let text = text.strip_suffix("\r\n").unwrap_or(&text);
        let mut lines = vec![format!("S: {head} {{{}}}", text.len() + 2)];
        lines.extend(text.split("\r\n").map(|line| format!("S: {line}")));
        lines.push("S: )".into());
        lines
    }

    fn mail(subject: &str, id: &str, text: &str) -> String {
        format!(
            "From: Grace Hopper <grace@example.org>\nTo: Ada Lovelace <ada@example.com>\nSubject: {subject}\nDate: Wed, 4 Nov 2026 10:12:31 +0100\nMessage-ID: <{id}>\nContent-Type: text/plain; charset=\"UTF-8\"\n\n{text}\n"
        )
    }

    fn report() -> String {
        let path = format!("{}/fixtures/email/report.eml", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(path).unwrap()
    }

    fn conversation(parts: Vec<Vec<String>>) -> Vec<String> {
        parts.into_iter().flatten().collect()
    }

    /// A Postfix-like submission server that takes one message and records every client line.
    async fn postfix() -> (u16, JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut seen: Vec<String> = Vec::new();
            let mut data = false;
            let mut answer = "220 smtp.example.com ESMTP Postfix";
            while seen.last().is_none_or(|line| line != "QUIT") {
                if !answer.is_empty() {
                    let sent = format!("{answer}\r\n");
                    stream.get_mut().write_all(sent.as_bytes()).await.unwrap();
                }
                let mut line = String::new();
                if stream.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                let line = line.trim_end_matches(['\r', '\n']).to_owned();
                answer = postfix_answer(&line, data);
                data = (data && line != ".") || line == "DATA";
                seen.push(line);
            }
            let _ = stream.get_mut().write_all(b"221 2.0.0 Bye\r\n").await;
            seen
        });
        (port, task)
    }

    fn postfix_answer(line: &str, data: bool) -> &'static str {
        match line {
            "." if data => "250 2.0.0 Ok: queued as 4XyZ1k2abcz9sWq",
            _ if data => "",
            "DATA" => "354 End data with <CR><LF>.<CR><LF>",
            "QUIT" => "221 2.0.0 Bye",
            ehlo if ehlo.starts_with("EHLO") => {
                "250-smtp.example.com\r\n250-PIPELINING\r\n250-SIZE 10240000\r\n250-AUTH PLAIN LOGIN XOAUTH2\r\n250-8BITMIME\r\n250 SMTPUTF8"
            }
            auth if auth.starts_with("AUTH") => "235 2.7.0 Authentication successful",
            _ => "250 2.1.0 Ok",
        }
    }

    #[tokio::test]
    async fn search_asks_the_server_and_lists_the_newest_first_with_snippets() {
        let older = mail("Lunch on Friday?", "a1@example.org", "Are you free?");
        let newer = mail(
            "Q3 report",
            "a2@example.org",
            "The numbers   are\nattached.",
        );
        let imap = Script::start(conversation(vec![
            dovecot(),
            examine("A0002", 45),
            script(
                r#"C: A0003 UID SEARCH FROM "grace@example.org" SINCE 1-Nov-2026 UNSEEN
                S: * SEARCH 40 42 44
                S: A0003 OK Search completed (0.001 + 0.000 secs).
                C: A0004 UID FETCH 44,42 (UID FLAGS BODY.PEEK[]<0.16384>)"#,
            ),
            literal("* 2 FETCH (UID 42 FLAGS () BODY[]<0>", &older),
            literal("* 3 FETCH (UID 44 FLAGS (\\Flagged) BODY[]<0>", &newer),
            script("S: A0004 OK Fetch completed (0.002 + 0.000 + 0.001 secs)."),
            logout("A0005"),
        ]))
        .await;
        let input =
            json!({"from": "grace@example.org", "since": "2026-11-01", "unseen": true, "limit": 2});
        let found = Email
            .call(&cx(imap.port(), 9, password()), "search", input)
            .await
            .unwrap();
        imap.finish().await;
        assert_eq!(found["total"], 3);
        let newest = &found["messages"][0];
        assert_eq!(newest["uid"], 44);
        assert_eq!(newest["subject"], "Q3 report");
        assert_eq!(newest["flags"], json!(["\\Flagged"]));
        assert_eq!(newest["snippet"], "The numbers are attached.");
        assert_eq!(
            found["messages"][1]["from"],
            json!(["Grace Hopper <grace@example.org>"])
        );
        let bad = Email
            .call(
                &cx(9, 9, password()),
                "search",
                json!({"since": "yesterday"}),
            )
            .await;
        assert!(matches!(bad, Err(Error::Invalid(_))), "{bad:?}");
    }

    fn read_report() -> Vec<String> {
        conversation(vec![
            dovecot(),
            examine("A0002", 45),
            script("C: A0003 UID FETCH 42 (UID FLAGS BODY.PEEK[]<0.26214400>)"),
            literal("* 2 FETCH (UID 42 FLAGS (\\Seen) BODY[]<0>", &report()),
            script("S: A0003 OK Fetch completed (0.001 + 0.000 secs)."),
            logout("A0004"),
        ])
    }

    #[tokio::test]
    async fn read_message_gives_the_plain_text_and_lists_attachments() {
        let imap = Script::start(read_report()).await;
        let cx = cx(imap.port(), 9, password());
        let read = Email
            .call(&cx, "read_message", json!({"uid": 42}))
            .await
            .unwrap();
        imap.finish().await;
        assert_eq!(read["subject"], "Q3 report");
        assert_eq!(read["from"], json!(["Grace Hopper <grace@example.org>"]));
        assert_eq!(read["message_id"], "CAF3k9xQ1@mail.example.org");
        assert_eq!(
            read["text"],
            "Hi Ada,\r\n\r\nThe Q3 numbers are attached. Revenue is up 12%.\r\n\r\nGrace\r\n"
        );
        assert_eq!(
            read["attachments"],
            json!([{"index": 0, "name": "q3.csv", "size": 23, "type": "text/csv"}])
        );
    }

    #[tokio::test]
    async fn save_attachment_answers_the_decoded_file() {
        let imap = Script::start(read_report()).await;
        let cx = cx(imap.port(), 9, password());
        let saved = Email
            .call(&cx, "save_attachment", json!({"uid": 42, "index": 0}))
            .await
            .unwrap();
        imap.finish().await;
        let file = &saved[crate::FILE];
        assert_eq!(file["name"], "q3.csv");
        assert_eq!(file["content_type"], "text/csv");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(file["base64"].as_str().unwrap())
            .unwrap();
        assert_eq!(bytes, b"quarter,revenue\nQ3,112\n");
    }

    #[tokio::test]
    async fn send_submits_the_message_with_its_headers_and_keeps_bcc_off_it() {
        let (port, smtp) = postfix().await;
        let input = json!({
            "to": ["Grace Hopper <grace@example.org>"], "bcc": ["alan@example.net"],
            "subject": "Q3 report", "text": "Thanks, Grace.",
        });
        let sent = Email
            .call(&cx(9, port, password()), "send", input)
            .await
            .unwrap();
        let seen = smtp.await.unwrap();
        let plain =
            base64::engine::general_purpose::STANDARD.encode("\0ada@example.com\0app-password-123");
        assert!(seen.contains(&format!("AUTH PLAIN {plain}")), "{seen:?}");
        for line in [
            "MAIL FROM:<ada@example.com>",
            "RCPT TO:<grace@example.org>",
            "RCPT TO:<alan@example.net>",
        ] {
            assert!(seen.contains(&line.to_owned()), "{line}: {seen:?}");
        }
        let id = sent["message_id"].as_str().unwrap();
        assert!(id.ends_with("@example.com>"), "{id}");
        let data = seen.join("\n");
        for header in [
            "From: \"Ada Lovelace\" <ada@example.com>",
            "To: \"Grace Hopper\" <grace@example.org>",
            "Subject: Q3 report",
            &format!("Message-ID: {id}"),
            "Thanks, Grace.",
        ] {
            assert!(data.contains(header), "{header}: {data}");
        }
        assert!(!data.to_ascii_lowercase().contains("bcc:"), "{data}");
    }

    #[tokio::test]
    async fn reply_all_threads_under_the_original_and_leaves_out_the_sender() {
        let original = "Message-ID: <CAF3k9xQ2@mail.example.org>\nIn-Reply-To: <CAF3k9xQ0@mail.example.com>\nReferences: <CAF3k9xQ0@mail.example.com>\nDate: Wed, 4 Nov 2026 10:12:31 +0100\nSubject: Re: Lunch on Friday?\nFrom: Grace Hopper <grace@example.org>\nTo: Ada Lovelace <ada@example.com>\nCc: Alan Turing <alan@example.net>\n\n";
        let imap = Script::start(conversation(vec![
            dovecot(),
            examine("A0002", 45),
            script("C: A0003 UID FETCH 42 (UID BODY.PEEK[HEADER])"),
            literal("* 2 FETCH (UID 42 BODY[HEADER]", original),
            script("S: A0003 OK Fetch completed (0.001 + 0.000 secs)."),
            logout("A0004"),
        ]))
        .await;
        let (port, smtp) = postfix().await;
        let input = json!({"uid": 42, "text": "Friday works.", "reply_all": true});
        let sent = Email
            .call(&cx(imap.port(), port, password()), "reply", input)
            .await
            .unwrap();
        imap.finish().await;
        assert_eq!(sent["to"], json!(["Grace Hopper <grace@example.org>"]));
        assert_eq!(sent["cc"], json!(["Alan Turing <alan@example.net>"]));
        let seen = smtp.await.unwrap();
        assert!(!seen.contains(&"RCPT TO:<ada@example.com>".to_owned()));
        assert!(seen.contains(&"RCPT TO:<alan@example.net>".to_owned()));
        let data = seen.join("\n");
        for header in [
            "Subject: Re: Lunch on Friday?\n",
            "In-Reply-To: <CAF3k9xQ2@mail.example.org>",
            "References: <CAF3k9xQ0@mail.example.com> <CAF3k9xQ2@mail.example.org>",
        ] {
            assert!(data.contains(header), "{header}: {data}");
        }
    }

    fn quiet_inbox() -> Vec<String> {
        conversation(vec![dovecot(), examine("A0002", 45), logout("A0003")])
    }

    #[tokio::test]
    async fn poll_starts_from_now_and_starts_over_when_the_mailbox_is_renumbered() {
        for cursor in [None, Some("1600000000:40")] {
            let imap = Script::start(quiet_inbox()).await;
            let polled = Email
                .poll(&cx(imap.port(), 9, password()), cursor)
                .await
                .unwrap();
            imap.finish().await;
            assert!(polled.events.is_empty(), "{cursor:?}");
            assert_eq!(polled.cursor.as_deref(), Some("1700000000:45"));
        }
    }

    #[tokio::test]
    async fn poll_reports_mail_that_arrived_since_the_cursor() {
        let arrived = mail("Q3 report", "a2@example.org", "The numbers are attached.");
        let imap = Script::start(conversation(vec![
            dovecot(),
            examine("A0002", 45),
            script(
                "C: A0003 UID SEARCH UID 44:*
                S: * SEARCH 44
                S: A0003 OK Search completed (0.001 + 0.000 secs).
                C: A0004 UID FETCH 44 (UID FLAGS BODY.PEEK[]<0.16384>)",
            ),
            literal("* 3 FETCH (UID 44 FLAGS () BODY[]<0>", &arrived),
            script("S: A0004 OK Fetch completed (0.001 + 0.000 secs)."),
            logout("A0005"),
        ]))
        .await;
        let polled = Email
            .poll(&cx(imap.port(), 9, password()), Some("1700000000:44"))
            .await
            .unwrap();
        imap.finish().await;
        assert_eq!(polled.cursor.as_deref(), Some("1700000000:45"));
        let [event] = polled.events.as_slice() else {
            panic!("one event: {:?}", polled.events);
        };
        assert_eq!(event.kind, "mail.received");
        assert_eq!(event.id, "1700000000:44");
        assert_eq!(
            event.summary,
            "New mail from Grace Hopper <grace@example.org>: Q3 report"
        );
        assert_eq!(event.data["snippet"], "The numbers are attached.");
    }

    #[tokio::test]
    async fn plaintext_is_refused_for_any_server_but_loopback() {
        let config = json!({"address": ADA, "imap_host": "imap.example.com", "security": "none"});
        let refused = Email.check(&Context::new(config, password())).await;
        assert!(
            matches!(refused, Err(Error::Invalid(ref m)) if m.contains("127.0.0.1")),
            "{refused:?}"
        );
        let outlook = Context::new(json!({"provider": "outlook", "address": ADA}), password());
        let smtp = Server {
            host: "smtp.office365.com".into(),
            port: 587,
            security: Security::StartTls,
        };
        assert_eq!(server(&outlook, true), Ok(smtp));
        assert_eq!(server(&outlook, false).unwrap().security, Security::Tls);
    }

    #[tokio::test]
    async fn xoauth2_signs_in_with_the_linked_token_and_a_refusal_needs_signing_in_again() {
        let token = Credential::Token {
            token: "ya29.a0AfB_byC-token".into(),
        };
        let bearer = base64::engine::general_purpose::STANDARD
            .encode("user=ada@example.com\x01auth=Bearer ya29.a0AfB_byC-token\x01\x01");
        let gmail = |answer: &str| {
            script(&format!(
                "S: * OK Gimap ready for requests from 203.0.113.7 q2mb123456789abc
                C: A0001 AUTHENTICATE XOAUTH2
                S: +
                C: {bearer}
                {answer}"
            ))
        };
        let accepted = Script::start(gmail(
            "S: * CAPABILITY IMAP4rev1 UNSELECT IDLE NAMESPACE QUOTA ID XLIST CHILDREN X-GM-EXT-1 UIDPLUS COMPRESS=DEFLATE ENABLE MOVE CONDSTORE ESEARCH UTF8=ACCEPT LIST-EXTENDED LIST-STATUS LITERAL- SPECIAL-USE APPENDLIMIT=35651584
            S: A0001 OK ada@example.com authenticated (Success)
            C: A0002 LOGOUT
            S: * BYE LOGOUT Requested
            S: A0002 OK 73 good day (Success)",
        ))
        .await;
        let account = Email
            .check(&cx(accepted.port(), 9, token.clone()))
            .await
            .unwrap();
        accepted.finish().await;
        assert_eq!(account.name, ADA);
        // Gmail explains a refusal in a base64 JSON challenge and waits for an empty answer.
        let refused = Script::start(gmail(
            "S: + eyJzdGF0dXMiOiI0MDAiLCJzY2hlbWVzIjoiQmVhcmVyIiwic2NvcGUiOiJodHRwczovL21haWwuZ29vZ2xlLmNvbS8ifQ==
            C~ 2
            S: A0001 NO [AUTHENTICATIONFAILED] Invalid credentials (Failure)",
        ))
        .await;
        let failed = Email.check(&cx(refused.port(), 9, token)).await;
        refused.finish().await;
        assert_eq!(failed, Err(Error::Unauthorized));
    }
}

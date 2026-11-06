//! CalDAV and CardDAV (RFC 4791, RFC 6352) with an app-specific password: iCloud, Fastmail,
//! Nextcloud, Radicale and other servers. Collections are found from the server's address
//! through the principal (RFC 5397), and the credential is only ever sent within that site.
use crate::{
    Account, Answer, Auth, Category, Choice, Connector, Context, Error, Event, EventSpec, Field,
    FieldKind, Level, Manifest, Poll, Product, Risk, Step, ToolSpec,
};
use reqwest::{Method, RequestBuilder, Response, StatusCode, Url, header};
use roxmltree::Node;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

const DAV: &str = "DAV:";
const REDIRECTS: usize = 3;
/// The most a PROPFIND or REPORT answer may hold.
const LARGEST: usize = 5 * 1024 * 1024;
const EVENTS: usize = 100;
const CONTACTS: usize = 50;
/// The longest text field passed on, such as a description.
const LONGEST: usize = 500;
/// How far ahead `event.starting` looks, in seconds.
const SOON: i64 = 15 * 60;
/// Calendars read per poll, and event ids remembered between polls.
const POLLED: usize = 20;
const REMEMBERED: usize = 200;

pub struct Dav;

/// What differs between calendars and address books during discovery.
struct Kind {
    setting: &'static str,
    /// Each preset server's address, by the `server` setting's value.
    presets: &'static [(&'static str, &'static str)],
    home: &'static str,
    ask_home: &'static str,
    collection: &'static str,
}

static CALENDARS: Kind = Kind {
    setting: "caldav_url",
    presets: &[
        ("icloud", "https://caldav.icloud.com/"),
        ("fastmail", "https://caldav.fastmail.com/dav/"),
    ],
    home: "calendar-home-set",
    ask_home: r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><c:calendar-home-set/></d:prop></d:propfind>"#,
    collection: "calendar",
};

static BOOKS: Kind = Kind {
    setting: "carddav_url",
    presets: &[
        ("icloud", "https://contacts.icloud.com/"),
        ("fastmail", "https://carddav.fastmail.com/dav/"),
    ],
    home: "addressbook-home-set",
    ask_home: r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav"><d:prop><c:addressbook-home-set/></d:prop></d:propfind>"#,
    collection: "addressbook",
};

const ASK_PRINCIPAL: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#;
const ASK_COLLECTIONS: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:a="http://apple.com/ns/ical/"><d:prop><d:resourcetype/><d:displayname/><a:calendar-color/></d:prop></d:propfind>"#;

static MANIFEST: Manifest = Manifest {
    id: "dav",
    name: "CalDAV and CardDAV",
    description: "Calendars and contacts on iCloud, Fastmail, Nextcloud, Radicale and other servers.",
    category: Category::MailCalendar,
    auth: &[Auth::Password {
        fields: &[
            Field {
                key: "username",
                label: "Username",
                kind: FieldKind::Text,
                required: true,
                placeholder: "you@icloud.com",
                help: "Usually your email address or login.",
            },
            Field {
                key: "password",
                label: "App-specific password",
                kind: FieldKind::Secret,
                required: true,
                placeholder: "",
                help: "Make one for Cubebot in your account's security settings rather than using your main password.",
            },
        ],
    }],
    config: &[
        Field {
            key: "server",
            label: "Server",
            kind: FieldKind::Choice(&[
                Choice {
                    value: "icloud",
                    label: "iCloud",
                },
                Choice {
                    value: "fastmail",
                    label: "Fastmail",
                },
                Choice {
                    value: "nextcloud",
                    label: "Nextcloud",
                },
                Choice {
                    value: "other",
                    label: "Another server",
                },
            ]),
            required: true,
            placeholder: "",
            help: "iCloud and Fastmail need no addresses; for others enter them below.",
        },
        Field {
            key: "caldav_url",
            label: "CalDAV address",
            kind: FieldKind::Url,
            required: false,
            placeholder: "https://cloud.example.com/remote.php/dav/",
            help: "Nextcloud: https://<your server>/remote.php/dav/. Radicale: its address, such as http://radicale.lan:5232/.",
        },
        Field {
            key: "carddav_url",
            label: "CardDAV address",
            kind: FieldKind::Url,
            required: false,
            placeholder: "https://cloud.example.com/remote.php/dav/",
            help: "Often the same as the CalDAV address.",
        },
    ],
    products: &[
        Product {
            id: "calendar",
            name: "Calendar",
        },
        Product {
            id: "contacts",
            name: "Contacts",
        },
    ],
    tools: &[
        ToolSpec {
            name: "list_calendars",
            description: "The account's calendars, with the href each calendar tool takes.",
            risk: Risk::Read,
            product: Some("calendar"),
            input: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_events",
            description: "Events of a calendar between two times; repeating events come as their occurrences when the server expands them.",
            risk: Risk::Read,
            product: Some("calendar"),
            input: r#"{"type":"object","properties":{"calendar":{"type":"string","description":"A calendar's href from list_calendars."},"start":{"type":"string","description":"RFC 3339, such as 2026-11-05T00:00:00Z."},"end":{"type":"string","description":"RFC 3339."}},"required":["calendar","start","end"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "create_event",
            description: "Add an event to a calendar. Nobody is invited.",
            risk: Risk::Write,
            product: Some("calendar"),
            input: r#"{"type":"object","properties":{"calendar":{"type":"string","description":"A calendar's href from list_calendars."},"summary":{"type":"string"},"start":{"type":"string","description":"RFC 3339, such as 2026-11-05T09:30:00+01:00."},"end":{"type":"string","description":"RFC 3339."},"description":{"type":"string"},"location":{"type":"string"}},"required":["calendar","summary","start","end"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "delete_event",
            description: "Delete an event; with its etag, only if it has not changed since it was read.",
            risk: Risk::Write,
            product: Some("calendar"),
            input: r#"{"type":"object","properties":{"event":{"type":"string","description":"The event's href from list_events."},"etag":{"type":"string"}},"required":["event"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_address_books",
            description: "The account's address books, with the href search_contacts takes.",
            risk: Risk::Read,
            product: Some("contacts"),
            input: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "search_contacts",
            description: "Contacts whose name or email address contains the text.",
            risk: Risk::Read,
            product: Some("contacts"),
            input: r#"{"type":"object","properties":{"address_book":{"type":"string","description":"An address book's href from list_address_books."},"text":{"type":"string"}},"required":["address_book","text"],"additionalProperties":false}"#,
        },
    ],
    events: &[EventSpec {
        kind: "event.starting",
        description: "An event starts within 15 minutes.",
        product: Some("calendar"),
    }],
    level: Level::Write,
    guide: &[
        Step {
            title: "Make an app-specific password",
            body: "iCloud: account.apple.com › Sign-In and Security › App-Specific Passwords. Fastmail: Settings › Privacy & Security › Manage app passwords, with calendars and contacts access. Nextcloud: Settings › Security › Create new app password.",
            link: None,
            copy: None,
        },
        Step {
            title: "Choose the server",
            body: "Pick iCloud or Fastmail, or enter your server's CalDAV and CardDAV addresses, then your username and the app password.",
            link: None,
            copy: None,
        },
    ],
    warning: None,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Nothing {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Between {
    calendar: String,
    start: String,
    end: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    calendar: String,
    summary: String,
    start: String,
    end: String,
    description: Option<String>,
    location: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Delete {
    event: String,
    etag: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Find {
    address_book: String,
    text: String,
}

impl Connector for Dav {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let calendars = MANIFEST.enabled(cx.config()).contains(&"calendar");
            let base = base(cx, if calendars { &CALENDARS } else { &BOOKS })?;
            principal(cx, &base).await?;
            let name = cx.setting("username").unwrap_or_default();
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "list_calendars" => listed(cx, input, &CALENDARS).await,
                "list_events" => list_events(cx, input).await,
                "create_event" => create(cx, input).await,
                "delete_event" => delete(cx, input).await,
                "list_address_books" => listed(cx, input, &BOOKS).await,
                "search_contacts" => search(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(poll(cx, cursor))
    }
}

/// The server's address for `kind`: the one entered, or the chosen preset's.
fn base(cx: &Context, kind: &Kind) -> Result<Url, Error> {
    let server = cx.setting("server").unwrap_or_default();
    let preset = kind.presets.iter().find(|(name, _)| *name == server);
    let address = cx
        .setting(kind.setting)
        .or(preset.map(|(_, address)| *address))
        .ok_or_else(|| Error::Invalid(format!("the connection has no {} setting", kind.setting)))?;
    Url::parse(address).map_err(|_| Error::Invalid(format!("{} is not an address", kind.setting)))
}

/// Whether the credential may go to `url`: the server's own origin, or over HTTPS another host
/// of its site, such as iCloud's `p42-caldav.icloud.com` partitions.
fn trusted(base: &Url, url: &Url) -> bool {
    let same_site = site(base).is_some_and(|base| site(url) == Some(base));
    base.origin() == url.origin() || (url.scheme() == "https" && same_site)
}

/// The registrable part of a host name, approximated as its last two labels, or three under a
/// country's second level such as `co.uk`. shortcut: no public-suffix list, so a shared hosting
/// domain counts as one site; use one if such servers need cross-host redirects.
fn site(url: &Url) -> Option<String> {
    let labels: Vec<&str> = url.domain()?.trim_end_matches('.').rsplit('.').collect();
    let country = labels.first().is_some_and(|tld| tld.len() == 2);
    let take = if country && labels.get(1).is_some_and(|sld| sld.len() <= 3) {
        3
    } else {
        2
    };
    (labels.len() >= take).then(|| labels[..take].join(".").to_ascii_lowercase())
}

/// A request to `url` with the connection's Basic credential, refused off the server's site.
fn authed(cx: &Context, method: Method, base: &Url, url: &Url) -> Result<RequestBuilder, Error> {
    if !trusted(base, url) {
        return Err(Error::Invalid(
            "that address is not on the connection's server".into(),
        ));
    }
    let user = cx
        .setting("username")
        .ok_or_else(|| Error::Invalid("the connection has no username".into()))?;
    let password = cx.secret("password")?;
    Ok(cx
        .request(method, url.as_str())?
        .basic_auth(user, Some(password)))
}

/// An href from a tool's input.
fn address(text: &str) -> Result<Url, Error> {
    Url::parse(text)
        .map_err(|_| Error::Invalid(format!("{text:?} is not an href from a list tool")))
}

/// A PROPFIND or REPORT, following up to [`REDIRECTS`] redirects within the server's site, which
/// the HTTP client does not. Answers where it ended, which relative hrefs are resolved against.
async fn read(
    cx: &Context,
    base: &Url,
    method: &str,
    mut url: Url,
    depth: &str,
    body: &str,
) -> Result<(Url, Vec<Entry>), Error> {
    let method = Method::from_bytes(method.as_bytes())
        .map_err(|_| Error::Invalid("not an HTTP method".into()))?;
    for _ in 0..=REDIRECTS {
        let request = authed(cx, method.clone(), base, &url)?
            .header("depth", depth)
            .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
            .body(body.to_owned());
        let response = request
            .send()
            .await
            .map_err(|_| Error::Unavailable("could not reach the server".into()))?;
        if !matches!(response.status().as_u16(), 301 | 302 | 307 | 308) {
            let xml = text(response).await?;
            return Ok((url, multistatus(&xml)?));
        }
        url = response
            .headers()
            .get(header::LOCATION)
            .and_then(|location| location.to_str().ok())
            .and_then(|location| url.join(location).ok())
            .filter(|next| trusted(base, next))
            .ok_or_else(|| {
                Error::Rejected(
                    "the server redirected to another site; enter that address instead".into(),
                )
            })?;
    }
    Err(Error::Rejected(
        "the server redirected too many times".into(),
    ))
}

/// A successful answer's body, bounded.
async fn text(mut response: Response) -> Result<String, Error> {
    let status = response.status();
    if !status.is_success() {
        return Err(refused(status));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| Error::Unavailable("the answer was cut off".into()))?
    {
        if body.len() + chunk.len() > LARGEST {
            return Err(Error::Rejected("the answer is larger than 5 MiB".into()));
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| Error::Unavailable("the answer was not UTF-8".into()))
}

/// What a read's status means; writes go through [`Context::send`] instead.
fn refused(status: StatusCode) -> Error {
    let answered = format!("the server answered {status}");
    match status.as_u16() {
        401 => Error::Unauthorized,
        403 => Error::Forbidden(answered),
        404 | 410 => Error::NotFound(answered),
        429 | 500..=599 => Error::Unavailable(answered),
        _ => Error::Rejected(answered),
    }
}

/// One `DAV:response` of a multistatus: its href and the properties it carries.
struct Entry {
    href: String,
    props: Vec<Prop>,
}

struct Prop {
    /// The local name; the names used here do not clash across namespaces.
    name: String,
    text: String,
    href: Option<String>,
    children: Vec<String>,
}

impl Entry {
    fn text(&self, name: &str) -> Option<&str> {
        self.props
            .iter()
            .filter(|prop| prop.name == name)
            .map(|prop| prop.text.as_str())
            .find(|text| !text.is_empty())
    }
    fn link(&self, name: &str) -> Option<&str> {
        self.props
            .iter()
            .filter(|prop| prop.name == name)
            .find_map(|prop| prop.href.as_deref())
    }
    fn is(&self, kind: &str) -> bool {
        self.props
            .iter()
            .any(|prop| prop.name == "resourcetype" && prop.children.iter().any(|c| c == kind))
    }
}

fn dav(node: &Node, name: &str) -> bool {
    node.tag_name().namespace() == Some(DAV) && node.tag_name().name() == name
}

fn href(node: Node) -> Option<String> {
    let href = node.children().find(|child| dav(child, "href"))?;
    Some(href.text()?.trim().to_owned())
}

fn multistatus(xml: &str) -> Result<Vec<Entry>, Error> {
    let document = roxmltree::Document::parse(xml)
        .map_err(|_| Error::Unavailable("the server's answer was not valid XML".into()))?;
    let entries = document
        .descendants()
        .filter(|node| dav(node, "response"))
        .map(|response| Entry {
            href: href(response).unwrap_or_default(),
            props: response
                .descendants()
                .filter(|node| dav(node, "prop"))
                .flat_map(|prop| prop.children().filter(Node::is_element))
                .map(|prop| Prop {
                    name: prop.tag_name().name().to_owned(),
                    text: prop.text().unwrap_or_default().trim().to_owned(),
                    href: href(prop),
                    children: prop
                        .children()
                        .filter(Node::is_element)
                        .map(|child| child.tag_name().name().to_owned())
                        .collect(),
                })
                .collect(),
        })
        .collect();
    Ok(entries)
}

/// The account's principal; servers without `current-user-principal` serve it at the base.
async fn principal(cx: &Context, base: &Url) -> Result<Url, Error> {
    let (at, found) = read(cx, base, "PROPFIND", base.clone(), "0", ASK_PRINCIPAL).await?;
    let principal = found
        .iter()
        .find_map(|entry| entry.link("current-user-principal"))
        .and_then(|href| at.join(href).ok());
    Ok(principal.unwrap_or(at))
}

/// The account's calendars or address books, with their absolute hrefs.
async fn discover(cx: &Context, kind: &Kind, base: &Url) -> Result<Vec<(Url, Entry)>, Error> {
    let principal = principal(cx, base).await?;
    let (at, found) = read(cx, base, "PROPFIND", principal, "0", kind.ask_home).await?;
    let home = found
        .iter()
        .find_map(|entry| entry.link(kind.home))
        .and_then(|href| at.join(href).ok())
        .ok_or_else(|| Error::NotFound(format!("the server names no {}", kind.home)))?;
    let (at, found) = read(cx, base, "PROPFIND", home, "1", ASK_COLLECTIONS).await?;
    Ok(found
        .into_iter()
        .filter(|entry| entry.is(kind.collection))
        .filter_map(|entry| Some((at.join(&entry.href).ok()?, entry)))
        .collect())
}

async fn listed(cx: &Context, input: Value, kind: &Kind) -> Result<Value, Error> {
    crate::input::<Nothing>(input)?;
    let base = base(cx, kind)?;
    let found: Vec<Value> = discover(cx, kind, &base)
        .await?
        .iter()
        .map(|(url, entry)| {
            json!({
                "href": url.as_str(),
                "name": entry.text("displayname").map(clip),
                "color": entry.text("calendar-color").map(clip),
            })
        })
        .collect();
    Ok(json!({"collections": found}))
}

async fn list_events(cx: &Context, input: Value) -> Result<Value, Error> {
    let between: Between = crate::input(input)?;
    let (start, end) = (seconds(&between.start)?, seconds(&between.end)?);
    let base = base(cx, &CALENDARS)?;
    let events = events_between(cx, &base, address(&between.calendar)?, start, end).await?;
    Ok(json!({"events": events}))
}

/// Events overlapping `start..end`, asking the server to expand repeating ones (RFC 4791 §9.6.5).
async fn events_between(
    cx: &Context,
    base: &Url,
    calendar: Url,
    start: i64,
    end: i64,
) -> Result<Vec<Value>, Error> {
    let (start, end) = (ical(start), ical(end));
    let query = format!(
        r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/><c:calendar-data><c:expand start="{start}" end="{end}"/></c:calendar-data></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"><c:time-range start="{start}" end="{end}"/></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"#
    );
    let (at, found) = read(cx, base, "REPORT", calendar, "1", &query).await?;
    let events = found.iter().flat_map(|entry| {
        let href = at.join(&entry.href).map(String::from).unwrap_or_default();
        let etag = entry.text("getetag").map(str::to_owned);
        vevents(entry.text("calendar-data").unwrap_or_default())
            .into_iter()
            .map(move |mut event| {
                event["href"] = json!(href);
                event["etag"] = json!(etag);
                event
            })
    });
    Ok(events.take(EVENTS).collect())
}

async fn create(cx: &Context, input: Value) -> Result<Value, Error> {
    let create: Create = crate::input(input)?;
    let (start, end) = (seconds(&create.start)?, seconds(&create.end)?);
    if end <= start {
        return Err(Error::Invalid("the end is not after the start".into()));
    }
    let base = base(cx, &CALENDARS)?;
    let mut calendar = address(&create.calendar)?;
    if !calendar.path().ends_with('/') {
        calendar.set_path(&format!("{}/", calendar.path()));
    }
    let uid = uid(&create);
    let url = calendar
        .join(&format!("{uid}.ics"))
        .map_err(|_| Error::Invalid("not a calendar href".into()))?;
    let request = authed(cx, Method::PUT, &base, &url)?
        .header(header::CONTENT_TYPE, "text/calendar; charset=utf-8")
        // Never replaces an existing event.
        .header(header::IF_NONE_MATCH, "*")
        .body(icalendar(&uid, &create, start, end));
    let response = cx.send(request).await?;
    Ok(json!({"href": url.as_str(), "uid": uid, "etag": etag(&response)}))
}

async fn delete(cx: &Context, input: Value) -> Result<Value, Error> {
    let delete: Delete = crate::input(input)?;
    let base = base(cx, &CALENDARS)?;
    let mut request = authed(cx, Method::DELETE, &base, &address(&delete.event)?)?;
    if let Some(etag) = delete.etag {
        request = request.header(header::IF_MATCH, etag);
    }
    cx.send(request).await?;
    Ok(json!({"deleted": true}))
}

async fn search(cx: &Context, input: Value) -> Result<Value, Error> {
    let find: Find = crate::input(input)?;
    let base = base(cx, &BOOKS)?;
    let text = escape_xml(&find.text);
    let matching = |name: &str| {
        format!(
            r#"<c:prop-filter name="{name}"><c:text-match collation="i;unicode-casemap" match-type="contains">{text}</c:text-match></c:prop-filter>"#
        )
    };
    let query = format!(
        r#"<?xml version="1.0" encoding="utf-8"?><c:addressbook-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav"><d:prop><d:getetag/><c:address-data/></d:prop><c:filter test="anyof">{}{}</c:filter></c:addressbook-query>"#,
        matching("FN"),
        matching("EMAIL"),
    );
    let book = address(&find.address_book)?;
    let (at, found) = read(cx, &base, "REPORT", book, "1", &query).await?;
    let contacts: Vec<Value> = found
        .iter()
        .filter_map(|entry| {
            let mut contact = contact(entry.text("address-data")?);
            contact["href"] = json!(at.join(&entry.href).ok()?.as_str());
            Some(contact)
        })
        .take(CONTACTS)
        .collect();
    Ok(json!({"contacts": contacts}))
}

/// Events starting within [`SOON`]. The cursor is the JSON list of `<uid>@<start>` already
/// reported whose start is still ahead; without one, those in the window now are only recorded.
async fn poll(cx: &Context, cursor: Option<&str>) -> Result<Poll, Error> {
    let mut reported: Vec<String> = cursor
        .and_then(|cursor| serde_json::from_str(cursor).ok())
        .unwrap_or_default();
    if !MANIFEST.enabled(cx.config()).contains(&"calendar") {
        return Ok(Poll {
            events: Vec::new(),
            cursor: cursor.map(str::to_owned),
        });
    }
    let now = now_nanos() as i64 / 1_000_000_000;
    let started = stamp(now);
    reported.retain(|id| {
        id.rsplit_once('@')
            .is_some_and(|(_, start)| start >= &started[..19])
    });
    let base = base(cx, &CALENDARS)?;
    let mut events = Vec::new();
    for (calendar, _) in discover(cx, &CALENDARS, &base)
        .await?
        .into_iter()
        .take(POLLED)
    {
        for event in events_between(cx, &base, calendar, now, now + SOON).await? {
            let start = event["start"].as_str().unwrap_or_default();
            let id = format!("{}@{}", event["uid"].as_str().unwrap_or_default(), start);
            if !starting(start, now) || reported.contains(&id) {
                continue;
            }
            reported.push(id.clone());
            let summary = event["summary"].as_str().unwrap_or("An event");
            let summary = format!("{summary} starts at {start}");
            events.push(Event {
                kind: "event.starting",
                id,
                summary,
                data: event,
            });
        }
    }
    let excess = reported.len().saturating_sub(REMEMBERED);
    reported.drain(..excess);
    Ok(Poll {
        events: if cursor.is_some() { events } else { Vec::new() },
        cursor: Some(json!(reported).to_string()),
    })
}

/// Whether a shown start such as `2026-11-05T09:30:00Z` is within [`SOON`] from `now`; all-day
/// events have no start time and never are. shortcut: a TZID or floating start is compared as
/// if it were UTC; servers that expand answer in UTC, which is the common case.
fn starting(start: &str, now: i64) -> bool {
    let Some(start) = start.get(..19) else {
        return false;
    };
    let (from, until) = (stamp(now), stamp(now + SOON));
    start >= &from[..19] && start < &until[..19]
}

/// Derived from the event, so creating it again after an uncertain answer is refused by
/// If-None-Match instead of making a duplicate.
fn uid(create: &Create) -> String {
    let fields = [
        &create.summary,
        &create.start,
        &create.end,
        create.description.as_deref().unwrap_or_default(),
        create.location.as_deref().unwrap_or_default(),
    ];
    let digest = Sha256::digest(fields.join("\n"));
    let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("cubebot-{hex}")
}

fn etag(response: &Response) -> Option<&str> {
    response.headers().get(header::ETAG)?.to_str().ok()
}

fn clip(text: &str) -> String {
    text.chars().take(LONGEST).collect()
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// One content line (RFC 5545 §3.1, RFC 6350 §3.3).
struct Line {
    /// Upper-cased, without a group prefix such as Apple's `item1.`.
    name: String,
    params: String,
    value: String,
}

/// The unfolded content lines of an iCalendar or vCard text.
fn lines(text: &str) -> Vec<Line> {
    let unfolded = text
        .replace("\r\n", "\n")
        .replace("\n ", "")
        .replace("\n\t", "");
    unfolded
        .lines()
        .filter_map(|line| {
            let (head, value) = line.split_once(':')?;
            let (name, params) = head.split_once(';').unwrap_or((head, ""));
            let name = name.rsplit('.').next().unwrap_or(name);
            Some(Line {
                name: name.to_ascii_uppercase(),
                params: params.to_owned(),
                value: value.to_owned(),
            })
        })
        .collect()
}

/// A text value's escapes undone (RFC 5545 §3.3.11), bounded.
fn unescape(value: &str) -> String {
    let mut text = String::new();
    let mut characters = value.chars();
    while let Some(c) = characters.next() {
        if c != '\\' {
            text.push(c);
            continue;
        }
        match characters.next() {
            Some('n' | 'N') => text.push('\n'),
            Some(other) => text.push(other),
            None => (),
        }
    }
    clip(&text)
}

/// The VEVENTs of an iCalendar text, leaving out nested components such as alarms.
fn vevents(ics: &str) -> Vec<Value> {
    let mut events = Vec::new();
    let mut current: Option<Value> = None;
    let mut nested = 0usize;
    for line in lines(ics) {
        let inside = current.is_some();
        match (line.name.as_str(), line.value.as_str()) {
            ("BEGIN", "VEVENT") if !inside => current = Some(json!({})),
            ("END", "VEVENT") if nested == 0 => events.extend(current.take()),
            ("BEGIN", _) if inside => nested += 1,
            ("END", _) if inside => nested = nested.saturating_sub(1),
            _ if nested == 0 => current.iter_mut().for_each(|event| field(event, &line)),
            _ => (),
        }
    }
    events
}

fn field(event: &mut Value, line: &Line) {
    let (key, value) = match line.name.as_str() {
        "UID" => ("uid", json!(clip(&line.value))),
        "SUMMARY" => ("summary", json!(unescape(&line.value))),
        "LOCATION" => ("location", json!(unescape(&line.value))),
        "DESCRIPTION" => ("description", json!(unescape(&line.value))),
        "DTSTART" => ("start", json!(when(&line.params, &line.value))),
        "DTEND" => ("end", json!(when(&line.params, &line.value))),
        "RRULE" => ("repeats", json!(clip(&line.value))),
        _ => return,
    };
    event[key] = value;
}

/// A DATE or DATE-TIME as `2026-11-05`, `2026-11-05T09:30:00Z` or, with its zone,
/// `2026-11-05T09:30:00 Europe/Berlin`; anything else as given.
fn when(params: &str, value: &str) -> String {
    let zone = params
        .split(';')
        .find_map(|param| param.strip_prefix("TZID="))
        .map(|zone| format!(" {}", zone.trim_matches('"')))
        .unwrap_or_default();
    let part = |range: std::ops::Range<usize>| value.get(range).unwrap_or_default();
    let shown = match value.len() {
        8 => format!("{}-{}-{}", part(0..4), part(4..6), part(6..8)),
        15 | 16 => format!(
            "{}-{}-{}T{}:{}:{}{}{zone}",
            part(0..4),
            part(4..6),
            part(6..8),
            part(9..11),
            part(11..13),
            part(13..15),
            value.get(15..).unwrap_or_default(),
        ),
        _ => return clip(value),
    };
    if value.is_ascii() { shown } else { clip(value) }
}

fn contact(vcard: &str) -> Value {
    let (mut name, mut org) = (None, None);
    let (mut emails, mut phones) = (Vec::new(), Vec::new());
    for line in lines(vcard) {
        match line.name.as_str() {
            "FN" => name = Some(unescape(&line.value)),
            "EMAIL" => emails.push(unescape(&line.value)),
            "TEL" => phones.push(unescape(line.value.trim_start_matches("tel:"))),
            "ORG" => {
                let units: Vec<String> = line
                    .value
                    .split(';')
                    .filter(|unit| !unit.is_empty())
                    .map(unescape)
                    .collect();
                org = Some(units.join(", "));
            }
            _ => (),
        }
    }
    json!({"name": name, "emails": emails, "phones": phones, "org": org})
}

fn icalendar(uid: &str, create: &Create, start: i64, end: i64) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_owned(),
        "VERSION:2.0".into(),
        "PRODID:-//Cubebot//Connectors//EN".into(),
        "BEGIN:VEVENT".into(),
        format!("UID:{uid}"),
        format!("DTSTAMP:{}", ical(now_nanos() as i64 / 1_000_000_000)),
        format!("DTSTART:{}", ical(start)),
        format!("DTEND:{}", ical(end)),
        format!("SUMMARY:{}", escape_text(&create.summary)),
    ];
    lines.extend(
        create
            .description
            .as_deref()
            .map(|d| format!("DESCRIPTION:{}", escape_text(d))),
    );
    lines.extend(
        create
            .location
            .as_deref()
            .map(|l| format!("LOCATION:{}", escape_text(l))),
    );
    lines.extend(["END:VEVENT".into(), "END:VCALENDAR".into()]);
    lines.iter().map(|line| fold(line)).collect()
}

/// Escaped so the text stays one value: a newline cannot start another property.
fn escape_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace("\r\n", "\\n")
        .replace(['\r', '\n'], "\\n")
}

/// A content line folded at 75 octets without splitting a character, with its CRLF.
fn fold(line: &str) -> String {
    let mut folded = String::new();
    let mut width = 0;
    for c in line.chars() {
        if width + c.len_utf8() > 75 {
            folded.push_str("\r\n ");
            width = 1;
        }
        folded.push(c);
        width += c.len_utf8();
    }
    folded + "\r\n"
}

fn now_nanos() -> u128 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH);
    since.map(|since| since.as_nanos()).unwrap_or_default()
}

/// Seconds since the epoch of an RFC 3339 time, such as `2026-11-05T09:30:00+01:00`.
fn seconds(text: &str) -> Result<i64, Error> {
    let bad = || {
        Error::Invalid(format!(
            "{text:?} is not an RFC 3339 time such as 2026-11-05T09:30:00Z"
        ))
    };
    let number = |range: std::ops::Range<usize>| {
        let digits = text
            .get(range)
            .filter(|d| d.bytes().all(|b| b.is_ascii_digit()));
        digits.and_then(|d| d.parse::<i64>().ok()).ok_or_else(bad)
    };
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    let valid = (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour < 24
        && minute < 60
        && second <= 60;
    let offset = text.get(19..).and_then(offset).ok_or_else(bad)?;
    if !valid {
        return Err(bad());
    }
    Ok(days(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// What follows the seconds, such as `.5Z` or `+01:00`, as an offset in seconds.
fn offset(zone: &str) -> Option<i64> {
    let zone = zone.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let sign = match zone.get(..1)? {
        "Z" | "z" if zone.len() == 1 => return Some(0),
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    let (hours, minutes) = zone.get(1..)?.split_once(':')?;
    let (hours, minutes) = (hours.parse::<i64>().ok()?, minutes.parse::<i64>().ok()?);
    Some(sign * (hours * 3600 + minutes * 60))
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's `days_from_civil`).
fn days(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    era * 146_097 + of_cycle - 719_468
}

/// `2026-11-05T09:30:00Z` for seconds since the epoch (Hinnant's `civil_from_days`).
fn stamp(seconds: i64) -> String {
    let (day, time) = (
        seconds.div_euclid(86_400) + 719_468,
        seconds.rem_euclid(86_400),
    );
    let era = day.div_euclid(146_097);
    let of_cycle = day - era * 146_097;
    let of_era = (of_cycle - of_cycle / 1460 + of_cycle / 36_524 - of_cycle / 146_096) / 365;
    let of_year = of_cycle - (365 * of_era + of_era / 4 - of_era / 100);
    let shifted = (5 * of_year + 2) / 153;
    let date = of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    let year = of_era + era * 400 + i64::from(month <= 2);
    let (hour, minute, second) = (time / 3600, time % 3600 / 60, time % 60);
    format!("{year:04}-{month:02}-{date:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// The iCalendar UTC form, `20261105T093000Z`.
fn ical(seconds: i64) -> String {
    stamp(seconds).replace(['-', ':'], "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };
    use base64::Engine;

    const PASSWORD: &str = "abcd-efgh-ijkl-mnop";

    fn cx(fixture: &Fixture, config: Value) -> Context {
        let credential = Credential::Fields {
            fields: [("password".into(), PASSWORD.into())].into(),
        };
        Context::new(config, credential).with_origin(&fixture.origin())
    }

    /// A self-hosted server on the fixture's loopback address.
    fn local(fixture: &Fixture) -> Context {
        let dav = format!("{}dav/", fixture.origin());
        let config =
            json!({"server": "other", "caldav_url": dav, "carddav_url": dav, "username": "victor"});
        cx(fixture, config)
    }

    /// Answers shaped as Radicale (principal, home), Nextcloud (calendars, events) and iCloud
    /// (contacts, the CDATA event) send them.
    fn recorded(name: &str) -> String {
        let path = format!("{}/fixtures/dav/{name}.xml", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(path).unwrap()
    }

    fn multistatus(method: &str, path: &str, name: &str) -> Exchange {
        Exchange::raw(method, path, 207, &recorded(name))
    }

    fn discovery() -> Vec<Exchange> {
        vec![
            multistatus("PROPFIND", "/dav/", "principal"),
            multistatus("PROPFIND", "/dav/victor/", "home"),
            multistatus("PROPFIND", "/dav/victor/", "calendars"),
        ]
    }

    #[tokio::test]
    async fn discovery_goes_from_the_principal_to_the_home_to_its_calendars() {
        let fixture = Fixture::start(discovery()).await;
        let listed = Dav
            .call(&local(&fixture), "list_calendars", json!({}))
            .await
            .unwrap();
        let personal = format!("{}dav/victor/personal/", fixture.origin());
        assert_eq!(
            listed,
            json!({"collections": [{"href": personal, "name": "Personal", "color": "#0082c9"}]})
        );
        let seen = fixture.finish().await;
        let depths: Vec<_> = seen.iter().map(|s| s.header("depth").unwrap()).collect();
        assert_eq!(depths, ["0", "0", "1"]);
        let basic = base64::engine::general_purpose::STANDARD.encode(format!("victor:{PASSWORD}"));
        let basic = format!("Basic {basic}");
        assert_eq!(seen[0].header("authorization"), Some(basic.as_str()));
        assert!(seen[1].text().contains("calendar-home-set"));
    }

    #[tokio::test]
    async fn icloud_redirects_to_its_partition_host_and_answers_absolute_hrefs() {
        let ok = |body: &str| {
            let xml = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><multistatus xmlns="DAV:"><response>{body}<status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#
            );
            xml
        };
        let principal = ok(
            "<href>/</href><propstat><prop><current-user-principal><href>/1234567890/principal/</href></current-user-principal></prop>",
        );
        let home = ok(
            r#"<href>/1234567890/principal/</href><propstat><prop><calendar-home-set xmlns="urn:ietf:params:xml:ns:caldav"><href xmlns="DAV:">https://p42-caldav.icloud.com:443/1234567890/calendars/</href></calendar-home-set></prop>"#,
        );
        let calendars = ok(
            r#"<href>/1234567890/calendars/home/</href><propstat><prop><resourcetype><collection/><calendar xmlns="urn:ietf:params:xml:ns:caldav"/></resourcetype><displayname>Home</displayname><calendar-color xmlns="http://apple.com/ns/ical/">#FF2968FF</calendar-color></prop>"#,
        );
        let fixture = Fixture::start(vec![
            Exchange::raw("PROPFIND", "/caldav.icloud.com/", 301, "")
                .header("location", "https://p42-caldav.icloud.com/"),
            Exchange::raw("PROPFIND", "/p42-caldav.icloud.com/", 207, &principal),
            Exchange::raw(
                "PROPFIND",
                "/p42-caldav.icloud.com/1234567890/principal/",
                207,
                &home,
            ),
            Exchange::raw(
                "PROPFIND",
                "/p42-caldav.icloud.com/1234567890/calendars/",
                207,
                &calendars,
            ),
        ])
        .await;
        let config = json!({"server": "icloud", "username": "ada@icloud.com"});
        let listed = Dav
            .call(&cx(&fixture, config), "list_calendars", json!({}))
            .await
            .unwrap();
        assert_eq!(
            listed["collections"][0],
            json!({"href": "https://p42-caldav.icloud.com/1234567890/calendars/home/", "name": "Home", "color": "#FF2968FF"})
        );
        fixture.finish().await;
    }

    #[tokio::test]
    async fn a_redirect_or_an_href_off_the_servers_site_never_gets_the_password() {
        let fixture = Fixture::start(vec![
            Exchange::raw("PROPFIND", "/dav/", 302, "")
                .header("location", "https://evil.example.net/dav/"),
        ])
        .await;
        let cx = local(&fixture);
        let listed = Dav.call(&cx, "list_calendars", json!({})).await;
        assert!(matches!(listed, Err(Error::Rejected(m)) if m.contains("another site")));
        let input = json!({"calendar": "https://evil.example.net/cal/", "start": "2026-11-06T00:00:00Z", "end": "2026-11-07T00:00:00Z"});
        let elsewhere = Dav.call(&cx, "list_events", input).await;
        assert!(matches!(elsewhere, Err(Error::Invalid(_))));
        assert_eq!(fixture.finish().await.len(), 1);

        let icloud = Url::parse("https://caldav.icloud.com/").unwrap();
        let at = |url: &str| trusted(&icloud, &Url::parse(url).unwrap());
        assert!(at("https://p42-caldav.icloud.com/1/"));
        assert!(!at("http://p42-caldav.icloud.com/1/"), "https only");
        assert!(!at("https://icloud.com.evil.net/"));
        let british = Url::parse("https://dav.example.co.uk/").unwrap();
        assert!(!trusted(
            &british,
            &Url::parse("https://evil.co.uk/").unwrap()
        ));
    }

    #[tokio::test]
    async fn list_events_unfolds_lines_keeps_zones_and_skips_alarms() {
        let fixture = Fixture::start(vec![multistatus(
            "REPORT",
            "/dav/victor/personal/",
            "events",
        )])
        .await;
        let calendar = format!("{}dav/victor/personal/", fixture.origin());
        let input = json!({"calendar": calendar, "start": "2026-11-06T01:00:00+01:00", "end": "2026-11-08T00:00:00Z"});
        let listed = Dav
            .call(&local(&fixture), "list_events", input)
            .await
            .unwrap();
        let planning = &listed["events"][0];
        assert_eq!(planning["summary"], "Planning, Q1");
        assert_eq!(planning["start"], "2026-11-06T09:30:00 Europe/Berlin");
        assert_eq!(
            planning["description"],
            "Agenda: budget, hiring and the roadmap for the first quarter of next year.\nBring numbers."
        );
        assert_eq!(planning["etag"], "\"8a7c1f0e2d3b4a596877e1d2c3b4a596\"");
        assert_eq!(
            planning["href"],
            format!("{calendar}6f1c2a3e-1b6d-4c9e-9a51-0d2f6b3c8e41.ics")
        );
        let holiday = &listed["events"][1];
        assert_eq!(holiday["start"], "2026-11-07");
        assert_eq!(holiday["end"], "2026-11-08");
        assert_eq!(holiday["repeats"], "FREQ=YEARLY");
        let seen = fixture.finish().await;
        assert_eq!(seen[0].header("depth"), Some("1"));
        let range = r#"<c:time-range start="20261106T000000Z" end="20261108T000000Z"/>"#;
        assert!(seen[0].text().contains(range));
    }

    #[tokio::test]
    async fn create_event_puts_a_new_escaped_event_that_invites_nobody() {
        let mut input = json!({
            "summary": "Lunch, then a walk\nATTENDEE:mailto:someone@example.com",
            "start": "2026-11-06T12:30:00+01:00",
            "end": "2026-11-06T13:30:00+01:00",
            "description": "ö".repeat(60),
        });
        input["calendar"] = json!("");
        let uid = uid(&crate::input(input.clone()).unwrap());
        let path = format!("/dav/victor/personal/{uid}.ics");
        let fixture = Fixture::start(vec![
            Exchange::raw("PUT", &path, 201, "").header("etag", "\"1\""),
        ])
        .await;
        input["calendar"] = json!(format!("{}dav/victor/personal", fixture.origin()));
        let made = Dav
            .call(&local(&fixture), "create_event", input)
            .await
            .unwrap();
        assert_eq!(
            (made["uid"].as_str(), made["etag"].as_str()),
            (Some(uid.as_str()), Some("\"1\""))
        );
        let seen = fixture.finish().await;
        assert_eq!(seen[0].header("if-none-match"), Some("*"));
        let sent = seen[0].text();
        assert!(sent.contains("DTSTART:20261106T113000Z\r\nDTEND:20261106T123000Z\r\n"));
        assert!(
            sent.contains("SUMMARY:Lunch\\, then a walk\\nATTENDEE:mailto:someone@example.com")
        );
        assert!(!sent.lines().any(|line| line.starts_with("ATTENDEE")));
        assert!(sent.split("\r\n").all(|line| line.len() <= 75), "{sent}");
    }

    #[tokio::test]
    async fn search_contacts_matches_name_or_email_and_reads_grouped_vcard_lines() {
        let fixture = Fixture::start(vec![multistatus("REPORT", "/dav/card/", "contacts")]).await;
        let book = format!("{}dav/card/", fixture.origin());
        let input = json!({"address_book": book, "text": "Ada & <co>"});
        let found = Dav
            .call(&local(&fixture), "search_contacts", input)
            .await
            .unwrap();
        let ada = &found["contacts"][0];
        assert_eq!(ada["name"], "Ada Lovelace");
        assert_eq!(
            ada["emails"],
            json!(["ada@example.com", "ada.home@example.org"])
        );
        assert_eq!(ada["phones"], json!(["+44 20 7946 0000"]));
        assert_eq!(ada["org"], "Analytical Engines, Ltd, Research");
        let card = "/1234567890/carddavhome/card/A1B2C3D4-0000-1111-2222-333344445555.vcf";
        assert!(ada["href"].as_str().unwrap().ends_with(card));
        let sent = fixture.finish().await[0].text();
        let filter = r#"<c:prop-filter name="EMAIL"><c:text-match collation="i;unicode-casemap" match-type="contains">Ada &amp; &lt;co&gt;</c:text-match>"#;
        assert!(sent.contains(filter), "{sent}");
    }

    /// A REPORT answer holding one half-hour event per `(uid, start)`.
    fn report(events: &[(&str, i64)]) -> Exchange {
        let responses: String = events
            .iter()
            .map(|(uid, start)| {
                let (start, end) = (ical(*start), ical(start + 1800));
                let ics = format!("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART:{start}\r\nDTEND:{end}\r\nSUMMARY:{uid}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n");
                format!(r#"<d:response><d:href>/dav/victor/personal/{uid}.ics</d:href><d:propstat><d:prop><cal:calendar-data>{ics}</cal:calendar-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"#)
            })
            .collect();
        let xml = format!(
            r#"<d:multistatus xmlns:d="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav">{responses}</d:multistatus>"#
        );
        Exchange::raw("REPORT", "/dav/victor/personal/", 207, &xml)
    }

    #[tokio::test]
    async fn poll_records_a_baseline_then_reports_only_newly_upcoming_events() {
        let now = now_nanos() as i64 / 1_000_000_000;
        let mut first = discovery();
        first.push(report(&[("standup", now + 300), ("ongoing", now - 600)]));
        let fixture = Fixture::start(first).await;
        let baseline = Dav.poll(&local(&fixture), None).await.unwrap();
        fixture.finish().await;
        assert!(baseline.events.is_empty());
        let cursor = baseline.cursor.unwrap();
        let standup = format!("standup@{}", stamp(now + 300));
        assert_eq!(cursor, json!([standup]).to_string());

        let mut second = discovery();
        let upcoming = [
            ("standup", now + 300),
            ("review", now + 600),
            ("later", now + 3600),
        ];
        second.push(report(&upcoming));
        let fixture = Fixture::start(second).await;
        let next = Dav.poll(&local(&fixture), Some(&cursor)).await.unwrap();
        fixture.finish().await;
        let ids: Vec<&str> = next.events.iter().map(|event| event.id.as_str()).collect();
        assert_eq!(ids, [format!("review@{}", stamp(now + 600))]);
        let summary = format!("review starts at {}", stamp(now + 600));
        assert_eq!(next.events[0].summary, summary);
        let remembered: Vec<String> = serde_json::from_str(&next.cursor.unwrap()).unwrap();
        assert_eq!(remembered.len(), 2);
    }

    #[test]
    fn rfc_3339_times_become_utc() {
        let utc = |text: &str| seconds(text).map(stamp);
        assert_eq!(
            utc("2026-11-05T09:30:00+02:00").unwrap(),
            "2026-11-05T07:30:00Z"
        );
        assert_eq!(
            utc("2024-02-29T23:59:59.5-00:30").unwrap(),
            "2024-03-01T00:29:59Z"
        );
        assert_eq!(seconds("1970-01-01T00:00:00Z"), Ok(0));
        let utc = seconds("2026-11-05T07:30:00z").unwrap();
        assert_eq!(ical(utc), "20261105T073000Z");
        let bad = [
            "2026-13-01T00:00:00Z",
            "tomorrow",
            "2026-11-05T09:30:00",
            "2026-11-05T09:30:00+2",
        ];
        for bad in bad {
            assert!(matches!(seconds(bad), Err(Error::Invalid(_))), "{bad}");
        }
    }
}

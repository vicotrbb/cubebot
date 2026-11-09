//! CalDAV and CardDAV (RFC 4791, RFC 6352) with an app-specific password: iCloud, Fastmail,
//! Nextcloud, Radicale and other servers. Collections are found from the server's address
//! through the principal (RFC 5397), and the credential is only ever sent within that site.
use crate::recurrence::{length, local, occurrences};
use crate::{
    Account, Answer, Auth, Category, Choice, Connector, Context, Error, Event, EventSpec, Field,
    FieldKind, Level, Manifest, Poll, Product, Risk, Step, ToolSpec,
};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
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
pub(crate) const EVENTS: usize = 100;
/// The most occurrences one `list_events` may ask for.
const MOST_EVENTS: usize = 2000;
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
            description: "Events of a calendar between two times; repeating events come as their occurrences.",
            risk: Risk::Read,
            product: Some("calendar"),
            input: r#"{"type":"object","properties":{"calendar":{"type":"string","description":"A calendar's href from list_calendars."},"start":{"type":"string","description":"RFC 3339, such as 2026-11-05T00:00:00Z."},"end":{"type":"string","description":"RFC 3339."},"limit":{"type":"integer","minimum":1,"maximum":2000,"description":"The most occurrences to answer, 100 when absent; more says whether others were left out."}},"required":["calendar","start","end"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "create_event",
            description: "Add an event to a calendar. Nobody is invited.",
            risk: Risk::Write,
            product: Some("calendar"),
            input: r#"{"type":"object","properties":{"calendar":{"type":"string","description":"A calendar's href from list_calendars."},"summary":{"type":"string"},"start":{"type":"string","description":"RFC 3339, such as 2026-11-05T09:30:00+01:00, or YYYY-MM-DD all day."},"end":{"type":"string","description":"RFC 3339, or YYYY-MM-DD all day (exclusive)."},"description":{"type":"string"},"location":{"type":"string"}},"required":["calendar","summary","start","end"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "update_event",
            description: "Change an event, or with its occurrence only that occurrence of a repeating event; with its etag, only if it has not changed since it was read. With an occurrence and all, the whole series changes, and new times move every occurrence as much as that one moves.",
            risk: Risk::Write,
            product: Some("calendar"),
            input: r#"{"type":"object","properties":{"event":{"type":"string","description":"The event's href from list_events."},"etag":{"type":"string"},"occurrence":{"type":"string","description":"The occurrence from list_events, to change only that one."},"time_zone":{"type":"string","description":"The occurrence's time_zone from list_events."},"all":{"type":"boolean","description":"Change the whole series the occurrence belongs to."},"summary":{"type":"string"},"start":{"type":"string","description":"RFC 3339, or YYYY-MM-DD all day; given together with end."},"end":{"type":"string","description":"RFC 3339, or YYYY-MM-DD all day (exclusive)."},"description":{"type":"string"},"location":{"type":"string"}},"required":["event"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "delete_event",
            description: "Delete an event, or with its occurrence only that occurrence of a repeating event; with its etag, only if it has not changed since it was read.",
            risk: Risk::Write,
            product: Some("calendar"),
            input: r#"{"type":"object","properties":{"event":{"type":"string","description":"The event's href from list_events."},"etag":{"type":"string"},"occurrence":{"type":"string","description":"The occurrence from list_events, to delete only that one."},"time_zone":{"type":"string","description":"The occurrence's time_zone from list_events."}},"required":["event"],"additionalProperties":false}"#,
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
    limit: Option<usize>,
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
struct Update {
    event: String,
    etag: Option<String>,
    occurrence: Option<String>,
    time_zone: Option<String>,
    #[serde(default)]
    all: bool,
    summary: Option<String>,
    start: Option<String>,
    end: Option<String>,
    description: Option<String>,
    location: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Delete {
    event: String,
    etag: Option<String>,
    occurrence: Option<String>,
    time_zone: Option<String>,
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
                "update_event" => update(cx, input).await,
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
pub(crate) fn authed(
    cx: &Context,
    method: Method,
    base: &Url,
    url: &Url,
) -> Result<RequestBuilder, Error> {
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
pub(crate) async fn read(
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
pub(crate) struct Entry {
    pub(crate) href: String,
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
    pub(crate) fn text(&self, name: &str) -> Option<&str> {
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
    pub(crate) fn is(&self, kind: &str) -> bool {
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
    let limit = between.limit.unwrap_or(EVENTS);
    if !(1..=MOST_EVENTS).contains(&limit) {
        return Err(Error::Invalid(format!("limit is 1 to {MOST_EVENTS}")));
    }
    let (start, end) = (seconds(&between.start)?, seconds(&between.end)?);
    let base = base(cx, &CALENDARS)?;
    let calendar = address(&between.calendar)?;
    let mut events = events_between(cx, &base, calendar, (start, end), limit + 1).await?;
    let more = events.len() > limit;
    events.truncate(limit);
    Ok(json!({"events": events, "more": more}))
}

/// Events overlapping `start..end`, asking the server to expand repeating ones (RFC 4791 §9.6.5)
/// and expanding those it answers unexpanded.
async fn events_between(
    cx: &Context,
    base: &Url,
    calendar: Url,
    (start, end): (i64, i64),
    limit: usize,
) -> Result<Vec<Value>, Error> {
    let (from, until) = (ical(start), ical(end));
    let query = format!(
        r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/><c:calendar-data><c:expand start="{from}" end="{until}"/></c:calendar-data></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"><c:time-range start="{from}" end="{until}"/></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"#
    );
    let (at, found) = read(cx, base, "REPORT", calendar, "1", &query).await?;
    let events = found.iter().flat_map(|entry| {
        let href = at.join(&entry.href).map(String::from).unwrap_or_default();
        let etag = entry.text("getetag").map(str::to_owned);
        occurrences(
            entry.text("calendar-data").unwrap_or_default(),
            (start, end),
            limit,
        )
        .into_iter()
        .map(move |mut event| {
            event["href"] = json!(href);
            event["etag"] = json!(etag);
            event
        })
    });
    Ok(events.take(limit).collect())
}

async fn create(cx: &Context, input: Value) -> Result<Value, Error> {
    let create: Create = crate::input(input)?;
    let (start, end) = period(&create.start, &create.end)?;
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
        .body(icalendar(&uid, &create, &start, &end));
    let response = cx.send(request).await?;
    Ok(json!({"href": url.as_str(), "uid": uid, "etag": etag(&response)}))
}

async fn update(cx: &Context, input: Value) -> Result<Value, Error> {
    let update: Update = crate::input(input)?;
    let times = times(update.start.as_deref(), update.end.as_deref())?;
    let texts = [&update.summary, &update.location, &update.description];
    if times.is_none() && texts.iter().all(|text| text.is_none()) {
        return Err(Error::Invalid("nothing to change".into()));
    }
    let occurrence = update.occurrence.as_deref();
    let occurrence = occurrence
        .map(|value| Occurrence::new(value, update.time_zone.as_deref()))
        .transpose()?;
    let base = base(cx, &CALENDARS)?;
    let url = address(&update.event)?;
    let (ics, read) = fetch(cx, &base, &url).await?;
    let mut parts = parts(&ics);
    let edit = Edit {
        update: &update,
        times,
        ics: &ics,
    };
    match (&occurrence, update.all) {
        (Some(occurrence), false) => edit.one(&mut parts, occurrence)?,
        (occurrence, _) => edit.all(&mut parts, occurrence.as_ref())?,
    }
    let response = store(cx, &base, &url, update.etag.clone().or(read), &parts).await?;
    Ok(json!({"href": url.as_str(), "etag": etag(&response)}))
}

async fn delete(cx: &Context, input: Value) -> Result<Value, Error> {
    let delete: Delete = crate::input(input)?;
    if let Some(value) = delete.occurrence.as_deref() {
        let occurrence = Occurrence::new(value, delete.time_zone.as_deref())?;
        return skip(cx, &delete, &occurrence).await;
    }
    let base = base(cx, &CALENDARS)?;
    let mut request = authed(cx, Method::DELETE, &base, &address(&delete.event)?)?;
    if let Some(etag) = delete.etag {
        request = request.header(header::IF_MATCH, etag);
    }
    cx.send(request).await?;
    Ok(json!({"deleted": true}))
}

/// Deletes one occurrence of a series: an EXDATE in the master instead of any override.
async fn skip(cx: &Context, delete: &Delete, occurrence: &Occurrence) -> Result<Value, Error> {
    let base = base(cx, &CALENDARS)?;
    let url = address(&delete.event)?;
    let (ics, read) = fetch(cx, &base, &url).await?;
    let mut parts = parts(&ics);
    let master = master(&parts)?;
    let exdate = line(
        "EXDATE",
        &occurrence.params(&parts[master]),
        &occurrence.value,
    );
    parts[master].insert(1, exdate);
    bump(&mut parts[master]);
    if let Some(moved) = overridden(&parts, occurrence) {
        parts.remove(moved);
    }
    store(cx, &base, &url, delete.etag.clone().or(read), &parts).await?;
    Ok(json!({"deleted": true}))
}

/// An event's iCalendar text and the etag it was read at.
async fn fetch(cx: &Context, base: &Url, url: &Url) -> Result<(String, Option<String>), Error> {
    let response = cx.send(authed(cx, Method::GET, base, url)?).await?;
    let read = etag(&response).map(str::to_owned);
    Ok((text(response).await?, read))
}

/// Puts the edited event back, only over the version it was read at when that is known.
async fn store(
    cx: &Context,
    base: &Url,
    url: &Url,
    matched: Option<String>,
    parts: &[Vec<Line>],
) -> Result<Response, Error> {
    let mut request = authed(cx, Method::PUT, base, url)?
        .header(header::CONTENT_TYPE, "text/calendar; charset=utf-8")
        .body(calendar(parts));
    if let Some(matched) = matched {
        request = request.header(header::IF_MATCH, matched);
    }
    cx.send_unchanged(request, "the event changed since it was read")
        .await
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
        for event in events_between(cx, &base, calendar, (now, now + SOON), EVENTS).await? {
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
/// events have no start time and never are.
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

pub(crate) fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// One content line (RFC 5545 §3.1, RFC 6350 §3.3).
#[derive(Clone)]
pub(crate) struct Line {
    /// Upper-cased, without a group prefix such as Apple's `item1.`.
    pub(crate) name: String,
    pub(crate) params: String,
    pub(crate) value: String,
}

/// The unfolded content lines of an iCalendar or vCard text.
pub(crate) fn lines(text: &str) -> Vec<Line> {
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

/// The top-level content lines of each VEVENT of an iCalendar text, leaving out nested
/// components such as alarms.
pub(crate) fn vevents(ics: &str) -> Vec<Vec<Line>> {
    let mut events = Vec::new();
    let mut current: Option<Vec<Line>> = None;
    let mut nested = 0usize;
    for line in lines(ics) {
        let inside = current.is_some();
        match (line.name.as_str(), line.value.as_str()) {
            ("BEGIN", "VEVENT") if !inside => current = Some(Vec::new()),
            ("END", "VEVENT") if nested == 0 => events.extend(current.take()),
            ("BEGIN", _) if inside => nested += 1,
            ("END", _) if inside => nested = nested.saturating_sub(1),
            _ if nested == 0 => {
                if let Some(event) = current.as_mut() {
                    event.push(line);
                }
            }
            _ => (),
        }
    }
    events
}

/// The descriptive fields of an event; [`crate::recurrence`] adds its times.
pub(crate) fn field(event: &mut Value, line: &Line) {
    let (key, value) = match line.name.as_str() {
        "UID" => ("uid", json!(clip(&line.value))),
        "SUMMARY" => ("summary", json!(unescape(&line.value))),
        "LOCATION" => ("location", json!(unescape(&line.value))),
        "DESCRIPTION" => ("description", json!(unescape(&line.value))),
        "RRULE" => ("repeats", json!(clip(&line.value))),
        _ => return,
    };
    event[key] = value;
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

fn icalendar(uid: &str, create: &Create, start: &When, end: &When) -> String {
    let time = |name, when| {
        let line = time(name, when, None, "");
        match line.params.as_str() {
            "" => format!("{name}:{}", line.value),
            params => format!("{name};{params}:{}", line.value),
        }
    };
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_owned(),
        "VERSION:2.0".into(),
        "PRODID:-//Cubebot//Connectors//EN".into(),
        "BEGIN:VEVENT".into(),
        format!("UID:{uid}"),
        format!("DTSTAMP:{}", ical(now_nanos() as i64 / 1_000_000_000)),
        time("DTSTART", start),
        time("DTEND", end),
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

/// A start or end: an all-day date, or seconds since the epoch.
#[derive(PartialEq, PartialOrd)]
enum When {
    Day(NaiveDate),
    At(i64),
}

impl When {
    /// Seconds since the epoch, a date's at midnight UTC.
    fn at(&self) -> i64 {
        match self {
            Self::Day(day) => day.and_time(NaiveTime::MIN).and_utc().timestamp(),
            Self::At(at) => *at,
        }
    }
}

fn when(text: &str) -> Result<When, Error> {
    if text.len() != 10 {
        return seconds(text).map(When::At);
    }
    let day = NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map_err(|_| Error::Invalid(format!("{text:?} is not a date such as 2026-11-05")))?;
    Ok(When::Day(day))
}

/// The new start and end, which change together.
fn times(start: Option<&str>, end: Option<&str>) -> Result<Option<(When, When)>, Error> {
    match (start, end) {
        (None, None) => Ok(None),
        (Some(start), Some(end)) => period(start, end).map(Some),
        _ => Err(Error::Invalid("start and end are changed together".into())),
    }
}

/// A start and an end after it, both dates or both times.
fn period(start: &str, end: &str) -> Result<(When, When), Error> {
    let (start, end) = (when(start)?, when(end)?);
    if matches!(
        (&start, &end),
        (When::Day(_), When::At(_)) | (When::At(_), When::Day(_))
    ) {
        return Err(Error::Invalid(
            "start and end are both dates or both times".into(),
        ));
    }
    if end <= start {
        return Err(Error::Invalid("the end is not after the start".into()));
    }
    Ok((start, end))
}

/// How long from `start` to `end`, as a DURATION value.
fn lasting(start: &When, end: &When) -> String {
    let length = end.at() - start.at();
    match start {
        When::Day(_) => format!("P{}D", length / 86_400),
        When::At(_) => format!("PT{length}S"),
    }
}

/// One occurrence of a series, as its RECURRENCE-ID or EXDATE names it.
struct Occurrence {
    value: String,
    /// The `TZID=` parameter of the time zone given with it.
    zone: Option<String>,
}

impl Occurrence {
    /// Checked, as both become part of a content line.
    fn new(value: &str, time_zone: Option<&str>) -> Result<Self, Error> {
        if instant(value).is_none() {
            return Err(Error::Invalid(format!(
                "{value:?} is not an occurrence from list_events"
            )));
        }
        Ok(Self {
            value: value.into(),
            zone: time_zone.map(tzid).transpose()?,
        })
    }

    /// Its parameters: a local time is in the zone given with it, else in the series' own.
    fn params(&self, master: &[Line]) -> String {
        match self.value.len() {
            8 => "VALUE=DATE".into(),
            15 => self
                .zone
                .as_deref()
                .or_else(|| zone(master))
                .unwrap_or_default()
                .to_owned(),
            _ => String::new(),
        }
    }
}

/// The `TZID=` parameter of an event's DTSTART.
fn zone(event: &[Line]) -> Option<&str> {
    let at = property(event, "DTSTART")?;
    event[at]
        .params
        .split(';')
        .find(|param| param.starts_with("TZID="))
}

fn tzid(zone: &str) -> Result<String, Error> {
    if zone.contains(|c: char| c.is_control() || c == '"') {
        return Err(Error::Invalid(format!(
            "{zone:?} is not a time zone from list_events"
        )));
    }
    Ok(match zone.contains([':', ';', ',']) {
        true => format!("TZID=\"{zone}\""),
        false => format!("TZID={zone}"),
    })
}

/// A RECURRENCE-ID, EXDATE or RDATE value, `20261105`, `20261105T093000` or `20261105T093000Z`,
/// as a time without a zone.
fn instant(value: &str) -> Option<NaiveDateTime> {
    match value.len() {
        8 => NaiveDate::parse_from_str(value, "%Y%m%d")
            .ok()?
            .and_hms_opt(0, 0, 0),
        _ => NaiveDateTime::parse_from_str(value, form(value)).ok(),
    }
}

fn form(value: &str) -> &'static str {
    match value.len() {
        8 => "%Y%m%d",
        16 => "%Y%m%dT%H%M%SZ",
        _ => "%Y%m%dT%H%M%S",
    }
}

/// A calendar resource as runs of content lines, each VEVENT a run of its own from its BEGIN
/// line to its END line, so an edit keeps everything else, such as time zones and alarms.
fn parts(ics: &str) -> Vec<Vec<Line>> {
    let mut parts = vec![Vec::new()];
    let mut depth = 0usize;
    for line in lines(ics) {
        let event = line.value == "VEVENT";
        match line.name.as_str() {
            "BEGIN" if depth > 0 || event => {
                if depth == 0 {
                    parts.push(Vec::new());
                }
                depth += 1;
            }
            "END" if depth > 0 => depth -= 1,
            _ => (),
        }
        let closed = depth == 0 && line.name == "END" && event;
        if let Some(part) = parts.last_mut() {
            part.push(line);
        }
        if closed {
            parts.push(Vec::new());
        }
    }
    parts
}

fn calendar(parts: &[Vec<Line>]) -> String {
    let text = |line: &Line| match line.params.as_str() {
        "" => format!("{}:{}", line.name, line.value),
        params => format!("{};{params}:{}", line.name, line.value),
    };
    parts
        .iter()
        .flatten()
        .map(|line| fold(&text(line)))
        .collect()
}

fn is_event(part: &[Line]) -> bool {
    part.first()
        .is_some_and(|line| line.name == "BEGIN" && line.value == "VEVENT")
}

/// The series' own VEVENT, the one that is not an override.
fn master(parts: &[Vec<Line>]) -> Result<usize, Error> {
    let found = parts
        .iter()
        .position(|part| is_event(part) && property(part, "RECURRENCE-ID").is_none());
    found.ok_or_else(|| Error::NotFound("the resource holds no event to change".into()))
}

/// The override already written for `occurrence`.
fn overridden(parts: &[Vec<Line>], occurrence: &Occurrence) -> Option<usize> {
    parts.iter().position(|part| {
        is_event(part)
            && property(part, "RECURRENCE-ID").is_some_and(|at| part[at].value == occurrence.value)
    })
}

/// The indexes of an event's own lines, leaving out its nested components such as alarms.
fn own(event: &[Line]) -> impl Iterator<Item = usize> + '_ {
    let mut depth = 0i32;
    event.iter().enumerate().filter_map(move |(at, line)| {
        match line.name.as_str() {
            "BEGIN" => depth += 1,
            "END" => depth -= 1,
            _ => return (depth == 1).then_some(at),
        }
        None
    })
}

fn property(event: &[Line], name: &str) -> Option<usize> {
    own(event).find(|&at| event[at].name == name)
}

fn line(name: &str, params: &str, value: &str) -> Line {
    Line {
        name: name.into(),
        params: params.into(),
        value: value.into(),
    }
}

/// Replaces the event's own property of that name, or adds it.
fn set(event: &mut Vec<Line>, line: Line) {
    match property(event, &line.name) {
        Some(at) => event[at] = line,
        None => event.insert(1, line),
    }
}

fn remove(event: &mut Vec<Line>, names: &[&str]) {
    let found: Vec<usize> = own(event)
        .filter(|&at| names.contains(&event[at].name.as_str()))
        .collect();
    for at in found.into_iter().rev() {
        event.remove(at);
    }
}

/// Marks the event as a new version (RFC 5545 §3.8.7.4) so clients take it.
fn bump(event: &mut Vec<Line>) {
    let sequence = property(event, "SEQUENCE").and_then(|at| event[at].value.parse::<u32>().ok());
    let next = sequence.unwrap_or(0) + 1;
    set(event, line("SEQUENCE", "", &next.to_string()));
    let now = now_nanos() as i64 / 1_000_000_000;
    set(event, line("DTSTAMP", "", &ical(now)));
}

/// A DTSTART or DTEND: a date, a local time in the zone of `zone` (a `TZID=` parameter), or UTC.
fn time(name: &str, when: &When, zone: Option<&str>, ics: &str) -> Line {
    let at = match when {
        When::Day(day) => return line(name, "VALUE=DATE", &day.format("%Y%m%d").to_string()),
        When::At(at) => *at,
    };
    let tzid = |param: &str| Some(param.strip_prefix("TZID=")?.trim_matches('"').to_owned());
    let local = zone.and_then(|param| Some((param, local(ics, &tzid(param)?, at)?)));
    match local {
        Some((param, value)) => line(name, param, &value),
        None => line(name, "", &ical(at)),
    }
}

/// An update applied to the lines of the event resource `ics`.
struct Edit<'a> {
    update: &'a Update,
    times: Option<(When, When)>,
    ics: &'a str,
}

impl Edit<'_> {
    /// Changes the whole series through its master; overrides keep their own changes. New times
    /// are the master's, or with an occurrence that occurrence's, moving the series as much.
    fn all(&self, parts: &mut [Vec<Line>], occurrence: Option<&Occurrence>) -> Result<(), Error> {
        let master = master(parts)?;
        let start = |event: &[Line]| property(event, "DTSTART").map(|at| event[at].value.clone());
        let from = start(&parts[master]);
        let event = &mut parts[master];
        self.texts(event);
        match (&self.times, occurrence) {
            (Some((start, end)), Some(occurrence)) => self.slide(event, occurrence, start, end)?,
            (Some((start, end)), None) => self.retime(event, start, end),
            (None, _) => (),
        }
        bump(event);
        if let (Some(from), Some(to)) = (from, start(&parts[master])) {
            shift(parts, &from, &to);
        }
        Ok(())
    }

    /// Changes one occurrence through its override, made from the master if there is none yet.
    fn one(&self, parts: &mut Vec<Vec<Line>>, occurrence: &Occurrence) -> Result<(), Error> {
        let at = match overridden(parts, occurrence) {
            Some(at) => at,
            None => {
                let made = self.override_of(&parts[master(parts)?], occurrence);
                let after = parts.iter().rposition(|part| is_event(part)).unwrap_or(0) + 1;
                parts.insert(after, made);
                after
            }
        };
        let event = &mut parts[at];
        self.texts(event);
        if let Some((start, end)) = &self.times {
            self.retime(event, start, end);
        }
        bump(event);
        Ok(())
    }

    /// The master as one occurrence at its original time, without its repetition.
    fn override_of(&self, master: &[Line], occurrence: &Occurrence) -> Vec<Line> {
        let mut event = master.to_vec();
        let repetition = ["RRULE", "RDATE", "EXDATE", "EXRULE", "DTEND", "DURATION"];
        remove(&mut event, &repetition);
        let (params, value) = (occurrence.params(master), &occurrence.value);
        set(&mut event, line("RECURRENCE-ID", &params, value));
        set(&mut event, line("DTSTART", &params, value));
        if let Some(length) = length(self.ics) {
            set(&mut event, line("DURATION", "", &length));
        }
        event
    }

    fn texts(&self, event: &mut Vec<Line>) {
        let update = self.update;
        let texts = [
            ("SUMMARY", &update.summary),
            ("LOCATION", &update.location),
            ("DESCRIPTION", &update.description),
        ];
        for (name, text) in texts {
            if let Some(text) = text {
                set(event, line(name, "", &escape_text(text)));
            }
        }
    }

    /// New times in the event's own zone.
    fn retime(&self, event: &mut Vec<Line>, start: &When, end: &When) {
        let zone = zone(event).map(str::to_owned);
        let (start, end) = (
            time("DTSTART", start, zone.as_deref(), self.ics),
            time("DTEND", end, zone.as_deref(), self.ics),
        );
        remove(event, &["DURATION"]);
        set(event, start);
        set(event, end);
    }

    /// Moves the master's start as far as `occurrence` moves to `start`, and makes every
    /// occurrence last from `start` to `end`.
    fn slide(
        &self,
        master: &mut Vec<Line>,
        occurrence: &Occurrence,
        start: &When,
        end: &When,
    ) -> Result<(), Error> {
        let params = occurrence.params(master);
        let to = time("DTSTART", start, Some(&params), self.ics).value;
        // A floating occurrence stays floating.
        let to = match occurrence.value.len() {
            15 => to.trim_end_matches('Z'),
            _ => &to,
        };
        let unlike =
            || Error::Invalid("an all-day series moves to dates, a timed one to times".into());
        let delta = delta(&occurrence.value, to).ok_or_else(unlike)?;
        let at = property(master, "DTSTART").ok_or_else(unlike)?;
        master[at].value = moved(&master[at].value, delta);
        remove(master, &["DTEND", "DURATION"]);
        set(master, line("DURATION", "", &lasting(start, end)));
        Ok(())
    }
}

/// How far `to` is from `from`, both RECURRENCE-ID values of one form.
fn delta(from: &str, to: &str) -> Option<TimeDelta> {
    let (from_at, to_at) = (instant(from)?, instant(to)?);
    (from.len() == to.len()).then(|| to_at - from_at)
}

/// Moves the series' rule and exceptions along with its start, so they keep naming the same
/// occurrences.
fn shift(parts: &mut [Vec<Line>], from: &str, to: &str) {
    let (Some(delta), Some(from), Some(to)) = (delta(from, to), instant(from), instant(to)) else {
        return;
    };
    let days = (to.date() - from.date()).num_days();
    for event in parts.iter_mut().filter(|part| is_event(part)) {
        let dated: Vec<usize> = own(event)
            .filter(|&at| {
                matches!(
                    event[at].name.as_str(),
                    "RRULE" | "EXDATE" | "RDATE" | "RECURRENCE-ID"
                )
            })
            .collect();
        for at in dated {
            let line = &mut event[at];
            line.value = match line.name.as_str() {
                "RRULE" => rule(&line.value, delta, days),
                _ => {
                    let values = line.value.split(',').map(|value| moved(value, delta));
                    values.collect::<Vec<_>>().join(",")
                }
            };
        }
    }
}

const WEEKDAYS: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];

/// A rule moved `days` days along with its series: its weekdays and its UNTIL. shortcut:
/// BYMONTHDAY, BYYEARDAY and BYMONTH stay; move them too if series move across those.
fn rule(rule: &str, delta: TimeDelta, days: i64) -> String {
    let weekday = |day: &str| {
        let (ordinal, code) = day.split_at(day.len().saturating_sub(2));
        let at = WEEKDAYS.iter().position(|weekday| *weekday == code);
        at.map_or_else(
            || day.to_owned(),
            |at| {
                format!(
                    "{ordinal}{}",
                    WEEKDAYS[(at as i64 + days).rem_euclid(7) as usize]
                )
            },
        )
    };
    let part = |part: &str| match part.split_once('=') {
        Some(("BYDAY", list)) => {
            let list: Vec<String> = list.split(',').map(weekday).collect();
            format!("BYDAY={}", list.join(","))
        }
        Some(("UNTIL", until)) => format!("UNTIL={}", moved(until, delta)),
        _ => part.to_owned(),
    };
    rule.split(';').map(part).collect::<Vec<_>>().join(";")
}

/// A value moved by `delta` in its own form; one that is not a time, such as a period, stays.
fn moved(value: &str, delta: TimeDelta) -> String {
    instant(value).map_or_else(
        || value.to_owned(),
        |at| (at + delta).format(form(value)).to_string(),
    )
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

pub(crate) fn now_nanos() -> u128 {
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
pub(crate) fn stamp(seconds: i64) -> String {
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
        testing::{Exchange, Fixture, Seen},
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
    async fn list_events_stops_at_its_limit_and_says_there_is_more() {
        let answer = || multistatus("REPORT", "/dav/victor/personal/", "events");
        let fixture = Fixture::start(vec![answer(), answer()]).await;
        let calendar = format!("{}dav/victor/personal/", fixture.origin());
        let window = json!({"calendar": calendar, "start": "2026-11-06T00:00:00Z", "end": "2026-11-08T00:00:00Z"});
        let mut limited = window.clone();
        limited["limit"] = json!(1);
        let cut = Dav
            .call(&local(&fixture), "list_events", limited)
            .await
            .unwrap();
        assert_eq!(
            (cut["events"].as_array().unwrap().len(), &cut["more"]),
            (1, &json!(true))
        );
        let all = Dav
            .call(&local(&fixture), "list_events", window)
            .await
            .unwrap();
        assert_eq!(
            (all["events"].as_array().unwrap().len(), &all["more"]),
            (2, &json!(false))
        );
        let refused = json!({"calendar": calendar, "start": "2026-11-06T00:00:00Z", "end": "2026-11-08T00:00:00Z", "limit": 2001});
        assert!(matches!(
            Dav.call(&local(&fixture), "list_events", refused).await,
            Err(Error::Invalid(_))
        ));
        fixture.finish().await;
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
        assert_eq!(planning["start"], "2026-11-06T08:30:00Z");
        assert_eq!(planning["time_zone"], "Europe/Berlin");
        assert_eq!(planning["occurrence"], Value::Null);
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
        assert_eq!(holiday["occurrence"], "20261107");
        let seen = fixture.finish().await;
        assert_eq!(seen[0].header("depth"), Some("1"));
        let range = r#"<c:time-range start="20261106T000000Z" end="20261108T000000Z"/>"#;
        assert!(seen[0].text().contains(range));
    }

    #[tokio::test]
    async fn list_events_expands_a_master_the_server_did_not() {
        let path = "/1234567890/calendars/home/";
        let fixture = Fixture::start(vec![multistatus("REPORT", path, "master")]).await;
        let calendar = format!("{}{}", fixture.origin(), &path[1..]);
        let input = json!({"calendar": calendar, "start": "2026-10-12T00:00:00Z", "end": "2026-11-03T00:00:00Z"});
        let listed = Dav
            .call(&local(&fixture), "list_events", input)
            .await
            .unwrap();
        let events = listed["events"].as_array().unwrap();
        let shown: Vec<(&str, &str)> = events
            .iter()
            .map(|event| {
                (
                    event["start"].as_str().unwrap(),
                    event["occurrence"].as_str().unwrap(),
                )
            })
            .collect();
        let expected = [
            ("2026-10-12T08:00:00Z", "20261012T100000"),
            ("2026-11-02T09:00:00Z", "20261102T100000"),
            ("2026-10-20T12:00:00Z", "20261019T100000"),
        ];
        assert_eq!(shown, expected);
        assert_eq!(events[2]["summary"], "Team sync (Tuesday)");
        assert_eq!(events[1]["time_zone"], "Europe/Berlin");
        assert_eq!(events[1]["etag"], "\"lu4q2k7m\"");
        assert_eq!(
            events[1]["href"],
            format!("{calendar}0C3E7A52-9B1F-4D6A-8E2C-5F4B3A291D07.ics")
        );
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

    const SERIES: &str = "/dav/victor/personal/series.ics";

    /// The weekly series of the `master` recording: a master with an EXDATE and one override.
    fn series(etag: &str) -> Exchange {
        let xml = recorded("master");
        let ics = xml.split("<![CDATA[").nth(1).unwrap().split("]]>").next();
        Exchange::raw("GET", SERIES, 200, ics.unwrap()).header("etag", etag)
    }

    async fn written(input: Value, tool: &str, status: u16) -> (Result<Value, Error>, Vec<Seen>) {
        let fixture = Fixture::start(vec![
            series("\"lu4q2k7m\""),
            Exchange::raw("PUT", SERIES, status, "").header("etag", "\"next\""),
        ])
        .await;
        let mut input = input;
        input["event"] = json!(format!("{}{}", fixture.origin(), &SERIES[1..]));
        let answer = Dav.call(&local(&fixture), tool, input).await;
        (answer, fixture.finish().await)
    }

    /// The October occurrences of what was written: `(start, end, summary)`.
    fn october(sent: &str) -> Vec<(String, String, String)> {
        let (start, end) = (
            seconds("2026-10-01T00:00:00Z"),
            seconds("2026-11-03T00:00:00Z"),
        );
        let shown = occurrences(sent, (start.unwrap(), end.unwrap()), EVENTS);
        let text = |event: &Value, key: &str| event[key].as_str().unwrap().to_owned();
        let mut shown: Vec<_> = shown
            .iter()
            .map(|e| (text(e, "start"), text(e, "end"), text(e, "summary")))
            .collect();
        shown.sort();
        shown
    }

    fn weekly(changed: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
        let mut weeks = vec![
            ("2026-10-05T08:00:00Z", "2026-10-05T08:30:00Z", "Team sync"),
            ("2026-10-12T08:00:00Z", "2026-10-12T08:30:00Z", "Team sync"),
            (
                "2026-10-20T12:00:00Z",
                "2026-10-20T12:30:00Z",
                "Team sync (Tuesday)",
            ),
            ("2026-11-02T09:00:00Z", "2026-11-02T09:30:00Z", "Team sync"),
        ];
        weeks.retain(|week| !changed.iter().any(|c| c.0 == "-" && c.1 == week.0));
        weeks.extend(changed.iter().filter(|c| c.0 != "-"));
        let mut weeks: Vec<_> = weeks
            .iter()
            .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
            .collect();
        weeks.sort();
        weeks
    }

    #[tokio::test]
    async fn update_event_rewrites_the_resource_with_if_match() {
        let input = json!({"summary": "Team sync, weekly", "start": "2026-10-05T08:15:00Z", "end": "2026-10-05T09:00:00Z"});
        let (answer, seen) = written(input, "update_event", 204).await;
        let answer = answer.unwrap();
        assert_eq!(answer["etag"], "\"next\"");
        assert!(answer["href"].as_str().unwrap().ends_with(SERIES));
        assert_eq!(seen[1].header("if-match"), Some("\"lu4q2k7m\""));
        let sent = seen[1].text();
        assert!(
            sent.contains("DTSTART;TZID=Europe/Berlin:20261005T101500\r\n"),
            "{sent}"
        );
        assert!(sent.contains("SEQUENCE:1\r\n") && sent.contains("BEGIN:VTIMEZONE"));
        assert!(sent.contains("EXDATE;TZID=Europe/Berlin:20261026T101500\r\n"));
        assert!(sent.contains("RECURRENCE-ID;TZID=Europe/Berlin:20261019T101500\r\n"));
        let moved = "Team sync, weekly";
        let expected = [
            ("2026-10-05T08:15:00Z", "2026-10-05T09:00:00Z", moved),
            ("2026-10-12T08:15:00Z", "2026-10-12T09:00:00Z", moved),
            (
                "2026-10-20T12:00:00Z",
                "2026-10-20T12:30:00Z",
                "Team sync (Tuesday)",
            ),
            ("2026-11-02T09:15:00Z", "2026-11-02T10:00:00Z", moved),
        ];
        let expected: Vec<_> = expected
            .iter()
            .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
            .collect();
        assert_eq!(october(&sent), expected);
    }

    #[tokio::test]
    async fn update_event_with_occurrence_adds_a_recurrence_id_override() {
        let occurrence = json!({"occurrence": "20261012T100000", "time_zone": "Europe/Berlin", "etag": "\"lu4q2k7m\""});
        let mut input = occurrence.clone();
        input["summary"] = json!("Team sync (with Ada)");
        let (answer, seen) = written(input, "update_event", 204).await;
        answer.unwrap();
        let sent = seen[1].text();
        assert!(sent.contains("RECURRENCE-ID;TZID=Europe/Berlin:20261012T100000\r\n"));
        assert_eq!(sent.matches("RRULE:FREQ=WEEKLY").count(), 1, "{sent}");
        let with_ada = (
            "2026-10-12T08:00:00Z",
            "2026-10-12T08:30:00Z",
            "Team sync (with Ada)",
        );
        let mut kept = weekly(&[("-", "2026-10-12T08:00:00Z", ""), with_ada]);
        assert_eq!(october(&sent), kept);

        let mut input = occurrence;
        input["occurrence"] = json!("20261019T100000");
        input["start"] = json!("2026-10-21T15:00:00+02:00");
        input["end"] = json!("2026-10-21T16:00:00+02:00");
        let (answer, seen) = written(input, "update_event", 204).await;
        answer.unwrap();
        let sent = seen[1].text();
        assert_eq!(sent.matches("RECURRENCE-ID").count(), 1, "{sent}");
        assert!(sent.contains("SEQUENCE:2\r\n"), "{sent}");
        kept = weekly(&[
            ("-", "2026-10-20T12:00:00Z", ""),
            (
                "2026-10-21T13:00:00Z",
                "2026-10-21T14:00:00Z",
                "Team sync (Tuesday)",
            ),
        ]);
        assert_eq!(october(&sent), kept);
    }

    #[tokio::test]
    async fn delete_event_with_occurrence_adds_an_exdate_and_keeps_the_series() {
        let input = json!({"occurrence": "20261019T100000", "time_zone": "Europe/Berlin"});
        let (answer, seen) = written(input, "delete_event", 204).await;
        assert_eq!(answer.unwrap(), json!({"deleted": true}));
        let methods: Vec<_> = seen.iter().map(|s| s.method.as_str()).collect();
        assert_eq!(methods, ["GET", "PUT"]);
        let sent = seen[1].text();
        assert!(sent.contains("EXDATE;TZID=Europe/Berlin:20261019T100000\r\n"));
        assert!(!sent.contains("RECURRENCE-ID"), "{sent}");
        assert_eq!(october(&sent), weekly(&[("-", "2026-10-20T12:00:00Z", "")]));
    }

    #[tokio::test]
    async fn a_changed_etag_is_refused() {
        let input = json!({"etag": "\"older\"", "summary": "Team sync"});
        let (answer, seen) = written(input, "update_event", 412).await;
        let changed = Error::Invalid("the event changed since it was read".into());
        assert_eq!(answer, Err(changed));
        assert_eq!(seen[1].header("if-match"), Some("\"older\""));
        let bad = json!({"occurrence": "20261019T100000\r\nRRULE:FREQ=DAILY"});
        let fixture = Fixture::start(vec![]).await;
        let mut input = bad;
        input["event"] = json!(format!("{}{}", fixture.origin(), &SERIES[1..]));
        let refused = Dav
            .call(&local(&fixture), "delete_event", input.clone())
            .await;
        assert!(matches!(refused, Err(Error::Invalid(_))));
        input.as_object_mut().unwrap().remove("occurrence");
        let unchanged = Dav.call(&local(&fixture), "update_event", input).await;
        assert_eq!(unchanged, Err(Error::Invalid("nothing to change".into())));
        fixture.finish().await;
    }

    #[tokio::test]
    async fn an_occurrence_without_its_time_zone_is_in_the_series_zone() {
        let input = json!({"occurrence": "20261019T100000"});
        let (answer, seen) = written(input, "delete_event", 204).await;
        answer.unwrap();
        let sent = seen[1].text();
        assert!(sent.contains("EXDATE;TZID=Europe/Berlin:20261019T100000\r\n"));
        assert_eq!(october(&sent), weekly(&[("-", "2026-10-20T12:00:00Z", "")]));

        let input = json!({"occurrence": "20261012T100000", "summary": "Team sync (with Ada)"});
        let (answer, seen) = written(input, "update_event", 204).await;
        answer.unwrap();
        let sent = seen[1].text();
        assert!(sent.contains("RECURRENCE-ID;TZID=Europe/Berlin:20261012T100000\r\n"));
        let with_ada = (
            "2026-10-12T08:00:00Z",
            "2026-10-12T08:30:00Z",
            "Team sync (with Ada)",
        );
        let kept = weekly(&[("-", "2026-10-12T08:00:00Z", ""), with_ada]);
        assert_eq!(october(&sent), kept);
    }

    /// The 2026-10-26 instance is an EXDATE in the recording, so the 10-12 one is moved.
    #[tokio::test]
    async fn moving_one_occurrence_with_all_shifts_the_series_by_its_delta() {
        let input = json!({
            "occurrence": "20261012T100000",
            "time_zone": "Europe/Berlin",
            "all": true,
            "start": "2026-10-13T11:00:00+02:00",
            "end": "2026-10-13T11:45:00+02:00",
        });
        let (answer, seen) = written(input, "update_event", 204).await;
        answer.unwrap();
        let sent = seen[1].text();
        assert!(
            sent.contains("DTSTART;TZID=Europe/Berlin:20261006T110000\r\n"),
            "{sent}"
        );
        assert!(sent.contains("RRULE:FREQ=WEEKLY;BYDAY=TU\r\n"));
        assert!(sent.contains("EXDATE;TZID=Europe/Berlin:20261027T110000\r\n"));
        assert!(sent.contains("RECURRENCE-ID;TZID=Europe/Berlin:20261020T110000\r\n"));
        let tuesdays = [
            ("2026-10-06T09:00:00Z", "2026-10-06T09:45:00Z", "Team sync"),
            ("2026-10-13T09:00:00Z", "2026-10-13T09:45:00Z", "Team sync"),
            (
                "2026-10-20T12:00:00Z",
                "2026-10-20T12:30:00Z",
                "Team sync (Tuesday)",
            ),
        ];
        let tuesdays: Vec<_> = tuesdays
            .iter()
            .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
            .collect();
        assert_eq!(october(&sent), tuesdays);
    }

    #[tokio::test]
    async fn create_event_makes_an_all_day_event() {
        let mut input = json!({"summary": "Holiday", "start": "2026-11-07", "end": "2026-11-08"});
        input["calendar"] = json!("");
        let uid = uid(&crate::input(input.clone()).unwrap());
        let path = format!("/dav/victor/personal/{uid}.ics");
        let fixture = Fixture::start(vec![Exchange::raw("PUT", &path, 201, "")]).await;
        input["calendar"] = json!(format!("{}dav/victor/personal/", fixture.origin()));
        let cx = local(&fixture);
        let mut mixed = input.clone();
        mixed["end"] = json!("2026-11-07T10:00:00Z");
        let refused = Dav.call(&cx, "create_event", mixed).await;
        assert!(matches!(refused, Err(Error::Invalid(_))));
        Dav.call(&cx, "create_event", input).await.unwrap();
        let sent = fixture.finish().await[0].text();
        let days = "DTSTART;VALUE=DATE:20261107\r\nDTEND;VALUE=DATE:20261108\r\n";
        assert!(sent.contains(days), "{sent}");
        let shown = occurrences(&sent, (1_790_000_000, 1_800_000_000), EVENTS);
        assert_eq!(shown[0]["start"], "2026-11-07");
        assert_eq!(shown[0]["end"], "2026-11-08");
        assert_eq!(shown[0]["all_day"], true);
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

//! RSS and Atom feeds the person lists, read without any sign-in (RSS 2.0:
//! https://www.rssboard.org/rss-specification, Atom: https://www.rfc-editor.org/rfc/rfc4287,
//! RSS 1.0: https://web.resource.org/rss/1.0/spec). Tools read only the configured feeds, so an
//! agent cannot use the connection to fetch other addresses.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Event, EventSpec, Field, FieldKind,
    Level, Manifest, Poll, Risk, ToolSpec,
};
use roxmltree::{Document, Node};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// Items of a feed considered, in the feed's order, for a tool and for polling.
const ITEMS: usize = 50;
/// The most characters of an item's summary the model gets.
const SUMMARY: usize = 600;
/// Feeds polled for new items: the cursor holds 8 hex digits per remembered item and must stay
/// within the orchestrator's 16 KiB, so the later feeds are readable but not watched.
const POLLED: usize = 20;
const ACCEPT: &str =
    "application/rss+xml, application/atom+xml, application/xml;q=0.9, text/xml;q=0.8, */*;q=0.5";

pub struct Rss;

static MANIFEST: Manifest = Manifest {
    id: "rss",
    name: "RSS and Atom feeds",
    description: "Read the latest items of the RSS and Atom feeds you list and hear about new ones.",
    category: Category::Work,
    auth: &[Auth::Public],
    config: &[Field {
        key: "feeds",
        label: "Feed addresses",
        kind: FieldKind::Lines,
        required: true,
        placeholder: "https://blog.rust-lang.org/feed.xml",
        help: "One RSS or Atom feed address per line. New items are watched in the first 20.",
    }],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_feeds",
            description: "The configured feeds with their numbers, titles and newest item.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_feed",
            description: "The latest items of a configured feed: title, link, date and a short summary.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"feed":{"type":["integer","string"],"description":"The feed's number from list_feeds, or its address as configured."},"limit":{"type":"integer","minimum":1,"maximum":50}},"required":["feed"],"additionalProperties":false}"#,
        },
    ],
    events: &[EventSpec {
        kind: "item.published",
        description: "A new item appeared in one of the first 20 feeds.",
        product: None,
    }],
    level: Level::Read,
    guide: &[],
    warning: Some(
        "Use each feed's final address: redirects are not followed, so a feed that moved (such as from http to https) fails until its new address is listed.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    feed: Value,
    limit: Option<usize>,
}

/// A parsed feed as the model sees it.
struct Feed {
    title: String,
    items: Vec<Value>,
}

impl Connector for Rss {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let feeds = feeds(cx);
            for (index, url) in feeds.iter().enumerate() {
                fetch(cx, url)
                    .await
                    .map_err(|error| numbered(index, error))?;
            }
            let name = match feeds.len() {
                1 => "1 feed".to_owned(),
                count => format!("{count} feeds"),
            };
            Ok(Account { name })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "list_feeds" => list(cx, input).await,
                "read_feed" => read(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(poll(cx, cursor))
    }
}

fn feeds(cx: &Context) -> Vec<String> {
    let listed = cx.config()["feeds"].as_array().into_iter().flatten();
    listed
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// A feed's error, naming it by number since errors never carry addresses. A feed asking for a
/// sign-in is not the connection's credential failing: it has none.
fn numbered(index: usize, error: Error) -> Error {
    let named = |message: String| format!("feed {}: {message}", index + 1);
    match error {
        Error::Unauthorized => Error::Forbidden(named("it asks for a sign-in".into())),
        Error::Forbidden(message) => Error::Forbidden(named(message)),
        Error::NotFound(message) => Error::NotFound(named(message)),
        Error::Rejected(message) => Error::Rejected(named(message)),
        Error::Unavailable(message) => Error::Unavailable(named(message)),
        other => other,
    }
}

async fn fetch(cx: &Context, url: &str) -> Result<Feed, Error> {
    let request = cx.get(url)?.header(reqwest::header::ACCEPT, ACCEPT);
    let (body, _) = cx.bytes(request).await?;
    // shortcut: bytes are read as UTF-8, so a Latin-1 feed shows replacement characters; decode
    // the declared encoding if such feeds matter.
    parse(&String::from_utf8_lossy(&body))
}

/// RSS 2.0, RSS 1.0 (RDF) and Atom, by local element names so namespaces do not matter.
fn parse(xml: &str) -> Result<Feed, Error> {
    let not_a_feed = || Error::Rejected("the answer is not an RSS or Atom feed".into());
    let document = Document::parse(xml).map_err(|_| not_a_feed())?;
    let root = document.root_element();
    if !matches!(root.tag_name().name(), "rss" | "feed" | "RDF") {
        return Err(not_a_feed());
    }
    let head = child(root, "channel").unwrap_or(root);
    let items = root
        .descendants()
        .filter(|node| matches!(node.tag_name().name(), "item" | "entry"))
        .take(ITEMS)
        .map(item)
        .collect();
    Ok(Feed {
        title: child(head, "title").map(text_of).unwrap_or_default(),
        items,
    })
}

fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == name)
}

/// The first of `names` the node has, in that order of preference.
fn first<'a, 'i>(node: Node<'a, 'i>, names: &[&str]) -> Option<Node<'a, 'i>> {
    names.iter().find_map(|name| child(node, name))
}

/// An element's text; Atom's `type="html"` and RSS descriptions hold escaped HTML.
fn text_of(node: Node) -> String {
    let raw: String = node
        .descendants()
        .filter(Node::is_text)
        .filter_map(|text| text.text())
        .collect();
    html_text(&raw)
}

/// Atom links are `href` attributes on `rel="alternate"` (or unmarked) links; RSS's are text.
fn link(item: Node) -> Option<String> {
    let links = item
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "link");
    for link in links {
        let alternate = link.attribute("rel").is_none_or(|rel| rel == "alternate");
        match link.attribute("href") {
            Some(href) if alternate => return Some(href.to_owned()),
            Some(_) => continue,
            None => {}
        }
        let text = link.text().unwrap_or_default().trim();
        if !text.is_empty() {
            return Some(text.to_owned());
        }
    }
    None
}

fn item(node: Node) -> Value {
    let title = first(node, &["title"]).map(text_of);
    let link = link(node);
    let date = first(node, &["pubDate", "published", "updated", "date"]).map(text_of);
    let summary = first(node, &["description", "summary", "encoded", "content"]).map(text_of);
    let summary: Option<String> = summary.map(|text| clip(&text, SUMMARY));
    // The id keeps items apart across polls; a feed without guids falls back to the link or title.
    let id = first(node, &["guid", "id"])
        .map(text_of)
        .filter(|id| !id.is_empty())
        .or_else(|| link.clone())
        .unwrap_or_else(|| format!("{title:?}{date:?}"));
    json!({"id": id, "title": title, "link": link, "date": date, "summary": summary})
}

fn clip(text: &str, longest: usize) -> String {
    match text.char_indices().nth(longest) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

/// Readable text from an HTML fragment: tags dropped, block ends as line breaks, script and style
/// left out, common entities decoded. Shared with other connectors that receive HTML.
// shortcut: not an HTML parser, so a `>` inside an attribute ends the tag early; use one if real
// pages read badly.
pub(crate) fn html_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        text.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            rest = "";
            break;
        };
        let tag = rest[open + 1..open + close].to_ascii_lowercase();
        rest = &rest[open + close + 1..];
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        if matches!(name.as_str(), "script" | "style") && !tag.starts_with('/') {
            let end = format!("</{name}");
            let lower = rest.to_ascii_lowercase();
            rest = lower.find(&end).map_or("", |at| &rest[at..]);
        } else if matches!(
            name.as_str(),
            "br" | "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
        ) {
            text.push('\n');
        }
    }
    text.push_str(rest);
    let text = entities(&text);
    let lines: Vec<String> = text
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect();
    lines.join("\n")
}

fn entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest.find(';').filter(|end| *end <= 10);
        let decoded = end.and_then(|end| entity(&rest[1..end]));
        match (end, decoded) {
            (Some(end), Some(c)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The named entities feeds and articles commonly use.
const NAMED: &[(&str, char)] = &[
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
    ("nbsp", ' '),
    ("mdash", '—'),
    ("ndash", '–'),
    ("hellip", '…'),
    ("rsquo", '’'),
    ("lsquo", '‘'),
    ("rdquo", '”'),
    ("ldquo", '“'),
];

fn entity(name: &str) -> Option<char> {
    let number = match name.strip_prefix('#') {
        Some(hex) if hex.starts_with(['x', 'X']) => u32::from_str_radix(&hex[1..], 16).ok(),
        Some(decimal) => decimal.parse().ok(),
        None => None,
    };
    if let Some(number) = number {
        return char::from_u32(number);
    }
    NAMED
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, c)| *c)
}

async fn list(cx: &Context, input: Value) -> Result<Value, Error> {
    crate::input::<Empty>(input)?;
    let mut listed = Vec::new();
    for (index, url) in feeds(cx).iter().enumerate() {
        let mut shown = match fetch(cx, url).await {
            Ok(feed) => json!({"title": feed.title, "newest": feed.items.first()}),
            Err(error) => json!({"error": numbered(index, error).to_string()}),
        };
        shown["number"] = json!(index + 1);
        shown["address"] = json!(url);
        listed.push(shown);
    }
    Ok(json!({"feeds": listed}))
}

async fn read(cx: &Context, input: Value) -> Result<Value, Error> {
    let read: Read = crate::input(input)?;
    let feeds = feeds(cx);
    let index = match &read.feed {
        Value::Number(number) => number
            .as_u64()
            .and_then(|number| usize::try_from(number).ok()?.checked_sub(1)),
        Value::String(url) => feeds.iter().position(|feed| feed == url.trim()),
        _ => None,
    };
    let Some((index, url)) = index.and_then(|index| Some((index, feeds.get(index)?))) else {
        return Err(Error::Invalid(
            "feed must be a number from list_feeds or a configured address".into(),
        ));
    };
    let feed = fetch(cx, url)
        .await
        .map_err(|error| numbered(index, error))?;
    let limit = read.limit.unwrap_or(20).clamp(1, ITEMS);
    let items: Vec<&Value> = feed.items.iter().take(limit).collect();
    Ok(json!({"title": feed.title, "items": items}))
}

/// Eight hex digits of an item's id: enough to tell a feed's items apart, small enough that the
/// cursor of [`POLLED`] feeds stays within bounds.
fn short(id: &str) -> String {
    Sha256::digest(id.as_bytes())[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The cursor maps each feed's short hash to the short ids of its items last seen, concatenated.
/// A feed missing from it, such as one just added, only records its items; a feed that fails
/// keeps what it had, so the others still report.
async fn poll(cx: &Context, cursor: Option<&str>) -> Result<Poll, Error> {
    let before: Map<String, Value> = cursor
        .and_then(|cursor| serde_json::from_str(cursor).ok())
        .unwrap_or_default();
    let mut after = Map::new();
    let mut events = Vec::new();
    let (mut failure, mut fetched) = (None, false);
    let feeds = feeds(cx);
    for (index, url) in feeds.iter().take(POLLED).enumerate() {
        let key = short(url);
        let known = before.get(&key).and_then(Value::as_str);
        let feed = match fetch(cx, url).await {
            Ok(feed) => feed,
            Err(error) => {
                failure.get_or_insert(numbered(index, error));
                after.extend(known.map(|known| (key, json!(known))));
                continue;
            }
        };
        fetched = true;
        let mut seen = String::new();
        for item in &feed.items {
            let id = short(item["id"].as_str().unwrap_or_default());
            let new = known.is_some_and(|known| !chunks(known).any(|old| old == id));
            if new {
                events.push(published(&feed.title, item));
            }
            seen.push_str(&id);
        }
        after.insert(key, json!(seen));
    }
    if let (Some(error), false) = (failure, fetched) {
        return Err(error);
    }
    Ok(Poll {
        events: if cursor.is_some() { events } else { Vec::new() },
        cursor: Some(Value::Object(after).to_string()),
    })
}

fn chunks(known: &str) -> impl Iterator<Item = &str> {
    (0..known.len() / 8).filter_map(move |at| known.get(at * 8..at * 8 + 8))
}

fn published(feed: &str, item: &Value) -> Event {
    let title = item["title"].as_str().unwrap_or("A new item");
    Event {
        kind: "item.published",
        id: item["id"].as_str().unwrap_or_default().to_owned(),
        summary: clip(&format!("{feed}: {title}"), 200),
        data: json!({"feed": feed, "item": item}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    const TWIR: &str = "https://this-week-in-rust.org/rss.xml";
    const RELEASES: &str = "https://github.com/rust-lang/rust/releases.atom";

    fn cx(fixture: &Fixture, feeds: &[&str]) -> Context {
        let credential = Credential::Fields {
            fields: Default::default(),
        };
        Context::new(json!({"feeds": feeds}), credential).with_origin(&fixture.origin())
    }

    fn recorded(name: &str) -> String {
        let path = format!("{}/fixtures/rss/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(path).unwrap()
    }

    fn twir() -> Exchange {
        Exchange::raw(
            "GET",
            "/this-week-in-rust.org/rss.xml",
            200,
            &recorded("twir.xml"),
        )
    }

    fn releases(body: &str) -> Exchange {
        Exchange::raw("GET", "/github.com/rust-lang/rust/releases.atom", 200, body)
    }

    #[tokio::test]
    async fn read_feed_shapes_rss_items_and_atom_entries_by_number_or_address() {
        let fixture = Fixture::start(vec![twir(), releases(&recorded("releases.atom"))]).await;
        let cx = cx(&fixture, &[TWIR, RELEASES]);
        let read = Rss
            .call(&cx, "read_feed", json!({"feed": 1, "limit": 1}))
            .await
            .unwrap();
        assert_eq!(read["title"], "This Week in Rust");
        let items = read["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            json!({
                "id": "tag:this-week-in-rust.org,2026-11-04:/blog/2026/11/04/this-week-in-rust-676/",
                "title": "This Week in Rust 676",
                "link": "https://this-week-in-rust.org/blog/2026/11/04/this-week-in-rust-676/",
                "date": "Wed, 04 Nov 2026 00:00:00 -0500",
                "summary": "Hello and welcome to another issue of This Week in Rust & friends!\nThis is a weekly summary of its progress and community.",
            })
        );
        let atom = Rss
            .call(&cx, "read_feed", json!({"feed": RELEASES}))
            .await
            .unwrap();
        assert_eq!(atom["title"], "Release notes from rust");
        let entry = &atom["items"][0];
        assert_eq!(entry["id"], "tag:github.com,2008:Repository/724712/1.99.1");
        assert_eq!(entry["title"], "Rust 1.99.1");
        assert_eq!(
            entry["link"],
            "https://github.com/rust-lang/rust/releases/tag/1.99.1"
        );
        assert_eq!(entry["date"], "2026-10-16T04:47:44Z");
        assert_eq!(
            entry["summary"],
            "library: fix missing symbol wcslen on UEFI\nrustc: fix miscompilation of FFI bool return values"
        );
        let seen = fixture.finish().await;
        assert!(seen[0].header("authorization").is_none());
        assert!(seen[0].header("accept").unwrap().contains("rss+xml"));
    }

    #[tokio::test]
    async fn tools_refuse_unlisted_feeds_and_unknown_fields_without_a_request() {
        let fixture = Fixture::start(Vec::new()).await;
        let cx = cx(&fixture, &[TWIR]);
        for input in [
            json!({"feed": 2}),
            json!({"feed": 0}),
            json!({"feed": "https://169.254.169.254/latest/meta-data"}),
            json!({"feed": 1, "url": TWIR}),
        ] {
            let refused = Rss.call(&cx, "read_feed", input.clone()).await;
            assert!(matches!(refused, Err(Error::Invalid(_))), "{input}");
        }
        let refused = Rss.call(&cx, "list_feeds", json!({"all": true})).await;
        assert!(matches!(refused, Err(Error::Invalid(_))));
        assert!(fixture.finish().await.is_empty());
    }

    #[tokio::test]
    async fn list_feeds_names_each_feed_and_reports_a_broken_one_by_number() {
        let fixture = Fixture::start(vec![
            twir(),
            Exchange::raw(
                "GET",
                "/github.com/rust-lang/rust/releases.atom",
                200,
                "<html>",
            ),
        ])
        .await;
        let listed = Rss
            .call(&cx(&fixture, &[TWIR, RELEASES]), "list_feeds", json!({}))
            .await
            .unwrap();
        assert_eq!(listed["feeds"][0]["number"], 1);
        assert_eq!(listed["feeds"][0]["title"], "This Week in Rust");
        assert_eq!(
            listed["feeds"][0]["newest"]["title"],
            "This Week in Rust 676"
        );
        assert_eq!(listed["feeds"][0]["address"], TWIR);
        assert_eq!(
            listed["feeds"][1]["error"],
            "refused: feed 2: the answer is not an RSS or Atom feed"
        );
        fixture.finish().await;
    }

    #[tokio::test]
    async fn errors_name_the_feed_and_a_feed_sign_in_is_not_the_connections() {
        let cases = [
            (
                401,
                Error::Forbidden("feed 1: it asks for a sign-in".into()),
            ),
            (403, Error::Forbidden("feed 1: no".into())),
            (404, Error::NotFound("feed 1: no".into())),
            (429, Error::Unavailable("feed 1: rate limited: no".into())),
            (301, Error::Rejected("feed 1: no".into())),
        ];
        for (status, expected) in cases {
            let answer = Exchange::raw("GET", "/this-week-in-rust.org/rss.xml", status, "no")
                .header("retry-after", "3600");
            let fixture = Fixture::start(vec![answer]).await;
            let read = Rss
                .call(&cx(&fixture, &[TWIR]), "read_feed", json!({"feed": 1}))
                .await;
            assert_eq!(read, Err(expected), "{status}");
            fixture.finish().await;
        }
    }

    fn entry(id: u32) -> String {
        format!(
            "<entry><id>tag:github.com,2008:Repository/724712/{id}</id><updated>2025-10-30T15:21:44Z</updated><link rel=\"alternate\" type=\"text/html\" href=\"https://github.com/rust-lang/rust/releases/tag/{id}\"/><title>Release {id}</title><content type=\"html\">&lt;p&gt;Notes {id}&lt;/p&gt;</content></entry>"
        )
    }

    fn atom(ids: &[u32]) -> String {
        let entries: String = ids.iter().map(|id| entry(*id)).collect();
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><feed xmlns=\"http://www.w3.org/2005/Atom\"><id>tag:github.com,2008:https://github.com/rust-lang/rust/releases</id><title>Release notes from rust</title><updated>2025-10-30T15:21:44Z</updated>{entries}</feed>"
        )
    }

    #[tokio::test]
    async fn poll_records_a_baseline_then_reports_each_new_item_once() {
        let fixture = Fixture::start(vec![
            releases(&atom(&[2, 1])),
            releases(&atom(&[3, 2, 1])),
            releases(&atom(&[3, 2, 1])),
        ])
        .await;
        let cx = cx(&fixture, &[RELEASES]);
        let baseline = Rss.poll(&cx, None).await.unwrap();
        assert!(baseline.events.is_empty());
        let cursor = baseline.cursor.unwrap();
        assert_eq!(cursor, json!({short(RELEASES): format!("{}{}", short("tag:github.com,2008:Repository/724712/2"), short("tag:github.com,2008:Repository/724712/1"))}).to_string());
        let next = Rss.poll(&cx, Some(&cursor)).await.unwrap();
        assert_eq!(next.events.len(), 1);
        let event = &next.events[0];
        assert_eq!(event.kind, "item.published");
        assert_eq!(event.id, "tag:github.com,2008:Repository/724712/3");
        assert_eq!(event.summary, "Release notes from rust: Release 3");
        assert_eq!(event.data["item"]["summary"], "Notes 3");
        let again = Rss.poll(&cx, next.cursor.as_deref()).await.unwrap();
        assert!(again.events.is_empty());
        fixture.finish().await;
    }

    #[tokio::test]
    async fn a_failing_feed_keeps_its_place_while_the_others_report() {
        let fixture = Fixture::start(vec![
            twir(),
            Exchange::raw(
                "GET",
                "/github.com/rust-lang/rust/releases.atom",
                500,
                "down",
            ),
        ])
        .await;
        let cx = cx(&fixture, &[TWIR, RELEASES]);
        let cursor = json!({short(RELEASES): "0011aabb"}).to_string();
        let polled = Rss.poll(&cx, Some(&cursor)).await.unwrap();
        assert!(polled.events.is_empty(), "the blog was not watched before");
        let after: Value = serde_json::from_str(&polled.cursor.unwrap()).unwrap();
        assert_eq!(after[short(RELEASES)], "0011aabb");
        assert_eq!(after[short(TWIR)].as_str().unwrap().len(), 16);
        fixture.finish().await;
    }

    #[test]
    fn html_text_keeps_words_and_line_breaks_but_not_markup_or_scripts() {
        let html = "<p>Hello&nbsp;<b>world</b> &amp; &#8220;friends&#x201D;</p><script>alert('<p>')</script><style>p{}</style><ul><li>One</li><li>Two &bogus; &</li></ul>";
        assert_eq!(
            html_text(html),
            "Hello world & \u{201c}friends\u{201d}\nOne\nTwo &bogus; &"
        );
    }

    /// Run with `cargo test -p cubebot-connectors rss -- --ignored` to read the real feeds.
    #[tokio::test]
    #[ignore = "reaches the internet"]
    async fn live_feeds_parse() {
        let credential = Credential::Fields {
            fields: Default::default(),
        };
        let feeds = json!([TWIR, RELEASES, "https://blog.rust-lang.org/feed.xml"]);
        let cx = Context::new(json!({"feeds": feeds}), credential);
        assert_eq!(Rss.check(&cx).await.unwrap().name, "3 feeds");
        for feed in [1, 2, 3] {
            let read = Rss
                .call(&cx, "read_feed", json!({"feed": feed, "limit": 2}))
                .await
                .unwrap();
            println!("{read:#}");
            assert!(
                read["items"][0]["link"]
                    .as_str()
                    .unwrap()
                    .starts_with("https://")
            );
        }
        let baseline = Rss.poll(&cx, None).await.unwrap();
        let again = Rss.poll(&cx, baseline.cursor.as_deref()).await.unwrap();
        assert!(again.events.is_empty());
    }
}

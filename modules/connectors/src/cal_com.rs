//! Cal.com, hosted or self-hosted, through an API key on API v2 (https://cal.com/docs/api-reference/v2/introduction,
//! https://cal.com/docs/api-reference/v2/bookings/get-all-bookings). v2 pins a `cal-api-version` per
//! endpoint; without it an endpoint answers in an older shape. The key goes only to the configured
//! address, and redirects are never followed.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Event, EventSpec, Field, FieldKind,
    Level, Manifest, PRIVATE_ADDRESS, Poll, Risk, Step, ToolSpec,
};
use reqwest::{Method, RequestBuilder};
use serde::Deserialize;
use serde_json::{Value, json};

const HOSTED: &str = "https://api.cal.com";
const BOOKINGS_VERSION: &str = "2026-05-01";
const BOOKING_VERSION: &str = "2026-02-25";
const EVENT_TYPES_VERSION: &str = "2026-06-12";
/// The most recently changed bookings each poll looks at.
const POLLED: usize = 50;
/// `uid@status` pairs the cursor remembers; well above [`POLLED`] so a booking still in the
/// window is never forgotten and reported again.
const REMEMBERED: usize = 300;
const LONGEST: usize = 2_000;

pub struct CalCom;

static MANIFEST: Manifest = Manifest {
    id: "cal-com",
    name: "Cal.com",
    description: "See event types and bookings in Cal.com, cancel bookings, and hear of new ones.",
    category: Category::MailCalendar,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "API key",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "cal_live_…",
            help: "From Settings › Developer › API keys.",
        }],
    }],
    config: &[Field {
        key: "url",
        label: "API address",
        kind: FieldKind::Url,
        required: false,
        placeholder: HOSTED,
        help: "Leave empty for cal.com. For a self-hosted install, the address of its API v2 server, without /v2.",
    }],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_event_types",
            description: "Your event types: their ids, titles, lengths and booking links.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_bookings",
            description: "Bookings, optionally by status, start window or attendee; pass next for more.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"status":{"type":"string","enum":["upcoming","recurring","past","cancelled","unconfirmed"]},"from":{"type":"string","description":"Starting at or after this ISO 8601 time."},"to":{"type":"string","description":"Ending at or before this ISO 8601 time."},"attendee":{"type":"string","description":"An attendee's email address."},"limit":{"type":"integer","minimum":1,"maximum":100},"next":{"type":"string","description":"The next cursor of a previous answer."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_booking",
            description: "A booking's details: times, attendees, location, answers and status.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"booking":{"type":"string","description":"The booking's uid."}},"required":["booking"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "cancel_booking",
            description: "Cancel a booking; Cal.com emails the attendees the cancellation and reason.",
            risk: Risk::Send,
            product: None,
            input: r#"{"type":"object","properties":{"booking":{"type":"string","description":"The booking's uid."},"reason":{"type":"string"}},"required":["booking"],"additionalProperties":false}"#,
        },
    ],
    events: &[
        EventSpec {
            kind: "booking.created",
            description: "Someone booked a meeting.",
            product: None,
        },
        EventSpec {
            kind: "booking.cancelled",
            description: "A booking was cancelled.",
            product: None,
        },
    ],
    level: Level::Write,
    guide: &[
        Step {
            title: "Create an API key",
            body: "In Cal.com open Settings › Developer › API keys and choose New. Name it Cubebot and pick an expiry, or Never expires, then copy the key (it starts with cal_) and paste it below.",
            link: Some("https://app.cal.com/settings/developer/api-keys"),
            copy: None,
        },
        Step {
            title: "Self-hosted only: give the API's address",
            body: "Cal.com's API v2 runs as its own server. Enter its address below, such as https://cal.example.com:5555; leave it empty for cal.com.",
            link: None,
            copy: None,
        },
        PRIVATE_ADDRESS,
    ],
    warning: Some(
        "The key acts as you with everything your account can do, and stops working when it expires; make a new one then.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    status: Option<String>,
    from: Option<String>,
    to: Option<String>,
    attendee: Option<String>,
    limit: Option<u8>,
    next: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Booking {
    booking: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cancel {
    booking: String,
    reason: Option<String>,
}

impl Connector for CalCom {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let me = answer(cx, request(cx, Method::GET, "/me", None)?).await?;
            let name = me["email"]
                .as_str()
                .or(me["username"].as_str())
                .unwrap_or("Cal.com account");
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "list_event_types" => event_types(cx, input).await,
                "list_bookings" => list(cx, input).await,
                "read_booking" => read(cx, input).await,
                "cancel_booking" => cancel(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(poll(cx, cursor))
    }
}

fn request(
    cx: &Context,
    method: Method,
    path: &str,
    version: Option<&str>,
) -> Result<RequestBuilder, Error> {
    let base = cx.setting("url").unwrap_or(HOSTED).trim_end_matches('/');
    let request = cx
        .request(method, &format!("{base}/v2{path}"))?
        .bearer_auth(cx.token()?);
    Ok(match version {
        Some(version) => request.header("cal-api-version", version),
        None => request,
    })
}

/// The `data` of an answer; v2 wraps every answer in `{status, data}`.
async fn answer(cx: &Context, request: RequestBuilder) -> Result<Value, Error> {
    let mut body = cx.json(request).await?;
    if body["status"] == "error" {
        let message = body["error"]["message"]
            .as_str()
            .unwrap_or("no reason given");
        return Err(Error::Rejected(message.chars().take(300).collect()));
    }
    Ok(body["data"].take())
}

/// A booking uid: letters, digits, `-` and `_`, so it stays one path segment.
fn uid(text: &str) -> Result<&str, Error> {
    let text = text.trim();
    let plain = text
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    match plain && (1..=100).contains(&text.len()) {
        true => Ok(text),
        false => Err(Error::Invalid(format!("{text:?} is not a booking uid"))),
    }
}

async fn event_types(cx: &Context, input: Value) -> Result<Value, Error> {
    crate::input::<Empty>(input)?;
    let request = request(cx, Method::GET, "/event-types", Some(EVENT_TYPES_VERSION))?;
    let types = answer(cx, request).await?;
    let shown: Vec<Value> = types
        .as_array()
        .into_iter()
        .flatten()
        .map(|kind| {
            json!({
                "id": kind["id"],
                "title": kind["title"],
                "slug": kind["slug"],
                "minutes": kind["lengthInMinutes"],
                "hidden": kind["hidden"],
                "url": kind["bookingUrl"],
            })
        })
        .collect();
    Ok(json!({"event_types": shown}))
}

async fn list(cx: &Context, input: Value) -> Result<Value, Error> {
    let list: List = crate::input(input)?;
    let limit = list.limit.unwrap_or(25).clamp(1, 100).to_string();
    let mut query = vec![("limit", limit.as_str())];
    let filters = [
        ("status", &list.status),
        ("afterStart", &list.from),
        ("beforeEnd", &list.to),
        ("attendeeEmail", &list.attendee),
        ("cursor", &list.next),
    ];
    query.extend(
        filters
            .iter()
            .filter_map(|(name, value)| Some((*name, value.as_deref()?))),
    );
    let request = request(cx, Method::GET, "/bookings", Some(BOOKINGS_VERSION))?.query(&query);
    let mut body = cx.json(request).await?;
    let bookings: Vec<Value> = body["data"]
        .as_array()
        .into_iter()
        .flatten()
        .map(booking)
        .collect();
    let next = body["pagination"]["nextCursor"].take();
    Ok(json!({"bookings": bookings, "next": next}))
}

async fn read(cx: &Context, input: Value) -> Result<Value, Error> {
    let wanted = crate::input::<Booking>(input)?.booking;
    let path = format!("/bookings/{}", uid(&wanted)?);
    let found = answer(cx, request(cx, Method::GET, &path, Some(BOOKING_VERSION))?).await?;
    // A recurring booking's uid answers with all its occurrences.
    let detailed = |item: &Value| {
        let mut shown = booking(item);
        let description = item["description"].as_str().unwrap_or_default();
        shown["description"] = json!(clip(description));
        shown["answers"] = item["bookingFieldsResponses"].clone();
        shown
    };
    Ok(match found.as_array() {
        Some(all) => json!({"occurrences": all.iter().map(detailed).collect::<Vec<_>>()}),
        None => detailed(&found),
    })
}

async fn cancel(cx: &Context, input: Value) -> Result<Value, Error> {
    let cancel: Cancel = crate::input(input)?;
    let path = format!("/bookings/{}/cancel", uid(&cancel.booking)?);
    let mut body = json!({});
    if let Some(reason) = cancel.reason {
        body["cancellationReason"] = json!(reason);
    }
    let request = request(cx, Method::POST, &path, Some(BOOKING_VERSION))?.json(&body);
    let done = answer(cx, request).await?;
    let first = done.as_array().and_then(|all| all.first()).unwrap_or(&done);
    Ok(json!({"cancelled": cancel.booking, "status": first["status"]}))
}

fn booking(item: &Value) -> Value {
    let people = |list: &Value| -> Vec<Value> {
        list.as_array()
            .into_iter()
            .flatten()
            .map(|person| json!({"name": person["name"], "email": person["email"]}))
            .collect()
    };
    json!({
        "uid": item["uid"],
        "title": item["title"],
        "status": item["status"],
        "start": item["start"],
        "end": item["end"],
        "event_type": item["eventType"]["slug"],
        "location": item["location"],
        "attendees": people(&item["attendees"]),
        "hosts": people(&item["hosts"]),
    })
}

fn clip(text: &str) -> String {
    match text.char_indices().nth(LONGEST) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.into(),
    }
}

/// The most recently changed bookings, compared with the `uid@status` pairs already seen: a uid
/// never seen is a new booking, and a newly cancelled status a cancellation.
async fn poll(cx: &Context, cursor: Option<&str>) -> Result<Poll, Error> {
    let mut seen: Vec<String> = cursor
        .and_then(|cursor| serde_json::from_str(cursor).ok())
        .unwrap_or_default();
    let limit = POLLED.to_string();
    let request = request(cx, Method::GET, "/bookings", Some(BOOKINGS_VERSION))?
        .query(&[("sortUpdatedAt", "desc"), ("limit", &limit)]);
    let found = answer(cx, request).await?;
    let mut events = Vec::new();
    // Oldest change first, so events arrive in the order they happened.
    for item in found.as_array().into_iter().flatten().rev() {
        let (Some(id), Some(status)) = (item["uid"].as_str(), item["status"].as_str()) else {
            continue;
        };
        let key = format!("{id}@{status}");
        if seen.contains(&key) {
            continue;
        }
        let known = seen.iter().any(|old| old.starts_with(&format!("{id}@")));
        if let Some(event) = change(item, &key, status, known) {
            events.push(event);
        }
        seen.push(key);
    }
    let excess = seen.len().saturating_sub(REMEMBERED);
    seen.drain(..excess);
    Ok(Poll {
        events: if cursor.is_some() { events } else { Vec::new() },
        cursor: Some(json!(seen).to_string()),
    })
}

fn change(item: &Value, key: &str, status: &str, known: bool) -> Option<Event> {
    let (kind, verb) = match (status, known) {
        ("cancelled", _) => ("booking.cancelled", "cancelled"),
        (_, false) => ("booking.created", "booked"),
        _ => return None,
    };
    let title = item["title"].as_str().unwrap_or("A meeting");
    let start = item["start"].as_str().unwrap_or_default();
    Some(Event {
        kind,
        id: key.into(),
        summary: format!("{title} at {start} was {verb}"),
        data: booking(item),
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
            fields: [("token".into(), "cal_live_secret".into())].into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    fn recorded(name: &str) -> Value {
        let path = format!(
            "{}/fixtures/cal-com/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn bookings(list: &[(&str, &str)]) -> Value {
        let data: Vec<Value> = list
            .iter()
            .map(|(uid, status)| {
                let mut item = recorded("bookings")["data"][0].clone();
                item["uid"] = json!(uid);
                item["status"] = json!(status);
                item
            })
            .collect();
        json!({"status": "success", "data": data, "pagination": {"nextCursor": null, "hasMore": false}})
    }

    #[tokio::test]
    async fn list_bookings_filters_pages_and_pins_the_version() {
        let fixture = Fixture::start(vec![Exchange::new(
            "GET",
            "/api.cal.com/v2/bookings",
            200,
            recorded("bookings"),
        )])
        .await;
        let input = json!({"status": "upcoming", "from": "2026-11-01T00:00:00Z", "limit": 10, "next": "eyJ2IjoxfQ"});
        let listed = CalCom
            .call(&cx(&fixture), "list_bookings", input)
            .await
            .unwrap();
        assert_eq!(listed["bookings"][0]["uid"], "2NtaeaVcKfpmSZ4CthFdfk");
        assert_eq!(
            listed["bookings"][0]["attendees"],
            json!([{"name": "John Doe", "email": "john@example.com"}])
        );
        assert_eq!(listed["next"], "eyJ2IjoxLCJzb3J0VXVpZCI6IjAx");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].target,
            "/api.cal.com/v2/bookings?limit=10&status=upcoming&afterStart=2026-11-01T00%3A00%3A00Z&cursor=eyJ2IjoxfQ"
        );
        assert_eq!(seen[0].header("cal-api-version"), Some(BOOKINGS_VERSION));
        assert_eq!(
            seen[0].header("authorization"),
            Some("Bearer cal_live_secret")
        );
    }

    #[tokio::test]
    async fn read_and_cancel_go_to_the_self_hosted_address() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/cal/v2/bookings/2NtaeaVcKfpmSZ4CthFdfk",
                200,
                json!({"status": "success", "data": recorded("bookings")["data"][0]}),
            ),
            Exchange::new(
                "POST",
                "/cal/v2/bookings/2NtaeaVcKfpmSZ4CthFdfk/cancel",
                200,
                json!({"status": "success", "data": {"uid": "2NtaeaVcKfpmSZ4CthFdfk", "status": "cancelled"}}),
            ),
        ])
        .await;
        let credential = Credential::Fields {
            fields: [("token".into(), "cal_secret".into())].into(),
        };
        let cx = Context::new(
            json!({"url": format!("{}cal/", fixture.origin())}),
            credential,
        );
        let read = CalCom
            .call(
                &cx,
                "read_booking",
                json!({"booking": "2NtaeaVcKfpmSZ4CthFdfk"}),
            )
            .await
            .unwrap();
        assert_eq!(read["description"], "Intro call");
        assert_eq!(read["event_type"], "consultation");
        let input = json!({"booking": "2NtaeaVcKfpmSZ4CthFdfk", "reason": "Double booked"});
        let cancelled = CalCom.call(&cx, "cancel_booking", input).await.unwrap();
        assert_eq!(cancelled["status"], "cancelled");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[1].json(),
            json!({"cancellationReason": "Double booked"})
        );
        assert_eq!(seen[1].header("cal-api-version"), Some(BOOKING_VERSION));
    }

    #[tokio::test]
    async fn event_types_are_listed_compactly() {
        let fixture = Fixture::start(vec![Exchange::new(
            "GET",
            "/api.cal.com/v2/event-types",
            200,
            json!({"status": "success", "data": [{"id": 50, "title": "Consultation", "slug": "consultation", "lengthInMinutes": 60, "hidden": false, "bookingUrl": "https://cal.com/jane/consultation", "locations": [], "bookingFields": [], "ownerId": 1, "users": []}]}),
        )])
        .await;
        let types = CalCom
            .call(&cx(&fixture), "list_event_types", json!({}))
            .await
            .unwrap();
        assert_eq!(
            types["event_types"][0],
            json!({"id": 50, "title": "Consultation", "slug": "consultation", "minutes": 60, "hidden": false, "url": "https://cal.com/jane/consultation"})
        );
        let seen = fixture.finish().await;
        assert_eq!(seen[0].header("cal-api-version"), Some(EVENT_TYPES_VERSION));
    }

    #[tokio::test]
    async fn refusals_map_to_errors_and_bad_input_never_reaches_cal_com() {
        let path = "/api.cal.com/v2/bookings/abc";
        let refusal = json!({"status": "error", "error": {"code": "NotFoundException", "message": "Booking with uid=abc was not found"}});
        let statuses = [401, 403, 404, 429];
        let mut exchanges: Vec<Exchange> = statuses
            .iter()
            .map(|status| {
                Exchange::new("GET", path, *status, refusal.clone()).header("retry-after", "300")
            })
            .collect();
        exchanges.push(Exchange::new("GET", path, 200, refusal.clone()));
        let fixture = Fixture::start(exchanges).await;
        let mut got = Vec::new();
        for _ in 0..=statuses.len() {
            let answer = CalCom
                .call(&cx(&fixture), "read_booking", json!({"booking": "abc"}))
                .await;
            got.push(answer.unwrap_err());
        }
        assert_eq!(got[0], Error::Unauthorized);
        assert!(matches!(got[1], Error::Forbidden(_)));
        assert_eq!(
            got[2],
            Error::NotFound("Booking with uid=abc was not found".into())
        );
        assert!(matches!(got[3], Error::Unavailable(_)));
        assert!(matches!(&got[4], Error::Rejected(message) if message.contains("not found")));
        let bad = CalCom
            .call(&cx(&fixture), "cancel_booking", json!({"booking": "../me"}))
            .await;
        assert!(matches!(bad, Err(Error::Invalid(_))));
        let unknown = CalCom
            .call(&cx(&fixture), "list_bookings", json!({"page": 2}))
            .await;
        assert!(matches!(unknown, Err(Error::Invalid(_))));
        assert_eq!(fixture.finish().await.len(), statuses.len() + 1);
    }

    #[tokio::test]
    async fn poll_records_a_baseline_then_reports_new_and_cancelled_bookings_once() {
        let path = "/api.cal.com/v2/bookings?sortUpdatedAt=desc&limit=50";
        let fixture = Fixture::start(vec![
            Exchange::new("GET", path, 200, bookings(&[("a", "accepted")])),
            Exchange::new(
                "GET",
                path,
                200,
                bookings(&[("b", "accepted"), ("a", "cancelled")]),
            ),
            Exchange::new(
                "GET",
                path,
                200,
                bookings(&[("b", "accepted"), ("a", "cancelled")]),
            ),
        ])
        .await;
        let baseline = CalCom.poll(&cx(&fixture), None).await.unwrap();
        assert!(baseline.events.is_empty());
        let cursor = baseline.cursor.unwrap();
        let next = CalCom.poll(&cx(&fixture), Some(&cursor)).await.unwrap();
        let kinds: Vec<(&str, &str)> = next
            .events
            .iter()
            .map(|event| (event.kind, event.id.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("booking.cancelled", "a@cancelled"),
                ("booking.created", "b@accepted")
            ]
        );
        assert_eq!(
            next.events[1].summary,
            "Consultation at 2025-03-07T10:00:00Z was booked"
        );
        let again = CalCom
            .poll(&cx(&fixture), next.cursor.as_deref())
            .await
            .unwrap();
        assert!(again.events.is_empty());
        fixture.finish().await;
    }
}

//! Calendly bookings through a personal access token (API v2,
//! https://developer.calendly.com/api-docs/overview/api/reference; tokens:
//! https://developer.calendly.com/docs/authentication/how-to-authenticate-with-personal-access-tokens;
//! scopes: https://developer.calendly.com/docs/authentication/scopes). New bookings are polled,
//! since webhook subscriptions need a paid plan while reading works on every plan
//! (https://developer.calendly.com/api-docs/overview/rate-limits).
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Event, EventSpec, Field, FieldKind,
    Level, Manifest, Poll, Risk, Step, ToolSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const API: &str = "https://api.calendly.com";
/// Upcoming events looked at per poll.
const UPCOMING: usize = 100;
/// Events whose invitees are fetched per poll, keeping each poll's work bounded.
const CHANGED: usize = 10;
/// The most characters of an answer to a booking question the model gets.
const LONGEST: usize = 2000;

pub struct Calendly;

static MANIFEST: Manifest = Manifest {
    id: "calendly",
    name: "Calendly",
    description: "See your Calendly event types and bookings, and hear when someone books.",
    category: Category::MailCalendar,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "Personal access token",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "eyJraWQiOi…",
            help: "With the users:read, event_types:read and scheduled_events:read scopes.",
        }],
    }],
    config: &[],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_event_types",
            description: "Who the account is and its event types, with their booking links.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"active":{"type":"boolean","description":"Only active ones, or only inactive ones."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_events",
            description: "Scheduled events in a time range, soonest first, with how many invitees each has.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"from":{"type":"string","description":"Events starting at or after this UTC time, such as 2026-11-05T00:00:00Z."},"to":{"type":"string","description":"Events starting before this UTC time."},"status":{"type":"string","enum":["active","canceled"]},"invitee_email":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100},"cursor":{"type":"string","description":"The next cursor from a previous answer."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_event",
            description: "A scheduled event with its invitees and their answers to the booking questions.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"event":{"type":"string","description":"The event's id or uri from list_events."}},"required":["event"],"additionalProperties":false}"#,
        },
    ],
    events: &[EventSpec {
        kind: "invitee.created",
        description: "Someone booked one of your upcoming events.",
        product: None,
    }],
    level: Level::Read,
    guide: &[
        Step {
            title: "Create a personal access token",
            body: "In Calendly open the Profile menu, choose Developer, then Personal access tokens, and choose Create personal access token. If Developer is missing, the tokens are under Integrations › API & Webhooks.",
            link: Some("https://calendly.com/app/developer?tab=personal_access_tokens"),
            copy: None,
        },
        Step {
            title: "Tick its scopes",
            body: "Name the token, then select the three read scopes below. Calendly emails a verification code; enter it.",
            link: None,
            copy: Some("users:read event_types:read scheduled_events:read"),
        },
        Step {
            title: "Copy the token",
            body: "Calendly shows the token only once: copy it and paste it below.",
            link: None,
            copy: None,
        },
    ],
    warning: Some(
        "A token made by an organization admin reads the whole organization's data, but this connection lists only your own events. Free plans allow 50 API requests a minute.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Types {
    active: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Range {
    from: Option<String>,
    to: Option<String>,
    status: Option<String>,
    invitee_email: Option<String>,
    limit: Option<u8>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    event: String,
}

/// What a poll remembers: the time invitees count as new after, and the upcoming events'
/// `<id>@<invitee total>`, so only an event that gained an invitee has its invitees fetched.
#[derive(Default, Deserialize, Serialize)]
struct Cursor {
    after: String,
    seen: Vec<String>,
}

impl Connector for Calendly {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let me = me(cx).await?;
            let name = me["email"].as_str().or(me["name"].as_str());
            Ok(Account {
                name: name.unwrap_or("Calendly account").into(),
            })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "list_event_types" => types(cx, input).await,
                "list_events" => events(cx, input).await,
                "read_event" => read(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(poll(cx, cursor))
    }
}

type Query = Vec<(&'static str, String)>;

async fn get(cx: &Context, path: &str, query: Query) -> Result<Value, Error> {
    let request = cx
        .get(&format!("{API}{path}"))?
        .bearer_auth(cx.token()?)
        .query(&query);
    cx.json(request).await
}

async fn me(cx: &Context) -> Result<Value, Error> {
    Ok(get(cx, "/users/me", Vec::new()).await?["resource"].take())
}

fn list(value: &Value) -> impl Iterator<Item = &Value> {
    value["collection"].as_array().into_iter().flatten()
}

/// The last segment of a Calendly uri, or the id itself, checked so it can sit in a path.
fn id(text: &str) -> Result<&str, Error> {
    let id = text
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    let valid =
        (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    valid
        .then_some(id)
        .ok_or_else(|| Error::Invalid(format!("{text:?} is not a Calendly event id or uri")))
}

async fn types(cx: &Context, input: Value) -> Result<Value, Error> {
    let types: Types = crate::input(input)?;
    let me = me(cx).await?;
    let mut query = vec![
        ("user", me["uri"].as_str().unwrap_or_default().to_owned()),
        ("count", "100".to_owned()),
    ];
    query.extend(types.active.map(|active| ("active", active.to_string())));
    let found = get(cx, "/event_types", query).await?;
    let shown: Vec<Value> = list(&found)
        .map(|kind| {
            json!({
                "uri": kind["uri"], "name": kind["name"], "active": kind["active"],
                "duration": kind["duration"], "kind": kind["kind"],
                "scheduling_url": kind["scheduling_url"],
            })
        })
        .collect();
    Ok(json!({
        "user": {"name": me["name"], "email": me["email"], "timezone": me["timezone"], "scheduling_url": me["scheduling_url"]},
        "event_types": shown,
    }))
}

async fn events(cx: &Context, input: Value) -> Result<Value, Error> {
    let range: Range = crate::input(input)?;
    let me = me(cx).await?;
    let mut query = vec![
        ("user", me["uri"].as_str().unwrap_or_default().to_owned()),
        ("count", range.limit.unwrap_or(25).clamp(1, 100).to_string()),
        ("sort", "start_time:asc".to_owned()),
    ];
    query.extend(range.from.map(|from| ("min_start_time", from)));
    query.extend(range.to.map(|to| ("max_start_time", to)));
    query.extend(range.status.map(|status| ("status", status)));
    query.extend(range.invitee_email.map(|email| ("invitee_email", email)));
    query.extend(range.cursor.map(|cursor| ("page_token", cursor)));
    let found = get(cx, "/scheduled_events", query).await?;
    let shown: Vec<Value> = list(&found).map(event).collect();
    Ok(json!({"events": shown, "next": found["pagination"]["next_page_token"]}))
}

fn event(event: &Value) -> Value {
    let uri = event["uri"].as_str().unwrap_or_default();
    let location = &event["location"];
    json!({
        "id": id(uri).ok(), "uri": uri, "name": event["name"], "status": event["status"],
        "start": event["start_time"], "end": event["end_time"], "event_type": event["event_type"],
        "location": {"type": location["type"], "where": location["location"], "join_url": location["join_url"]},
        "invitees": event["invitees_counter"]["active"],
    })
}

fn invitee(invitee: &Value) -> Value {
    let answers: Vec<Value> = invitee["questions_and_answers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|qa| {
            let answer = qa["answer"].as_str().unwrap_or_default();
            json!({"question": qa["question"], "answer": answer.chars().take(LONGEST).collect::<String>()})
        })
        .collect();
    json!({
        "uri": invitee["uri"], "name": invitee["name"], "email": invitee["email"],
        "status": invitee["status"], "timezone": invitee["timezone"],
        "created_at": invitee["created_at"], "rescheduled": invitee["rescheduled"],
        "no_show": !invitee["no_show"].is_null(), "answers": answers,
    })
}

async fn read(cx: &Context, input: Value) -> Result<Value, Error> {
    let read: Read = crate::input(input)?;
    let id = id(&read.event)?;
    let found = get(cx, &format!("/scheduled_events/{id}"), Vec::new()).await?;
    let query = vec![("count", "100".to_owned())];
    let invitees = get(cx, &format!("/scheduled_events/{id}/invitees"), query).await?;
    let mut shown = event(&found["resource"]);
    shown["invitees"] = json!(list(&invitees).map(invitee).collect::<Vec<_>>());
    shown["more_invitees"] = json!(invitees["pagination"]["next_page_token"].is_string());
    Ok(shown)
}

/// Now as Calendly writes times, with microseconds so the strings order as the times do.
fn now() -> String {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    let seconds = since.map(|since| since.as_secs()).unwrap_or_default();
    crate::dav::stamp(i64::try_from(seconds).unwrap_or_default()).replace('Z', ".000000Z")
}

fn key(event: &Value) -> String {
    let uri = event["uri"].as_str().unwrap_or_default();
    let total = event["invitees_counter"]["total"]
        .as_f64()
        .unwrap_or_default();
    format!("{}@{total}", id(uri).unwrap_or_default())
}

/// Upcoming events that gained an invitee since the last poll have their invitees fetched; those
/// created after the cursor's time are new bookings. A first poll only records the events.
async fn poll(cx: &Context, cursor: Option<&str>) -> Result<Poll, Error> {
    let started = now();
    let me = me(cx).await?;
    let query = vec![
        ("user", me["uri"].as_str().unwrap_or_default().to_owned()),
        ("status", "active".to_owned()),
        ("min_start_time", started.clone()),
        ("sort", "start_time:asc".to_owned()),
        ("count", UPCOMING.to_string()),
    ];
    let upcoming = get(cx, "/scheduled_events", query).await?;
    let before: Option<Cursor> = cursor.and_then(|cursor| serde_json::from_str(cursor).ok());
    let Some(before) = before else {
        let seen = list(&upcoming).map(key).collect();
        return Ok(Poll {
            events: Vec::new(),
            cursor: Some(
                json!(Cursor {
                    after: started,
                    seen
                })
                .to_string(),
            ),
        });
    };
    let (events, after) = booked(cx, &upcoming, &before, started).await?;
    Ok(Poll {
        events,
        cursor: Some(json!(after).to_string()),
    })
}

async fn booked(
    cx: &Context,
    upcoming: &Value,
    before: &Cursor,
    started: String,
) -> Result<(Vec<Event>, Cursor), Error> {
    let mut after = Cursor {
        after: started,
        seen: Vec::new(),
    };
    let mut events = Vec::new();
    let mut fetched = 0;
    for found in list(upcoming) {
        let key = key(found);
        if before.seen.contains(&key) {
            after.seen.push(key);
            continue;
        }
        if fetched == CHANGED {
            // shortcut: the rest wait for the next poll, so the time must not move past them;
            // an event fetched now that changes again before then may repeat a booking.
            after.after = before.after.clone();
            continue;
        }
        fetched += 1;
        let path = format!(
            "/scheduled_events/{}/invitees",
            id(found["uri"].as_str().unwrap_or_default())?
        );
        let query = vec![
            ("status", "active".to_owned()),
            ("sort", "created_at:desc".to_owned()),
            ("count", "100".to_owned()),
        ];
        let invitees = get(cx, &path, query).await?;
        let new = list(&invitees).filter(|invitee| {
            invitee["created_at"].as_str().unwrap_or_default() > before.after.as_str()
        });
        events.extend(new.map(|invitee| created(found, invitee)));
        after.seen.push(key);
    }
    events.reverse();
    Ok((events, after))
}

fn created(found: &Value, booked: &Value) -> Event {
    let who = booked["name"].as_str().unwrap_or("Someone");
    let what = found["name"].as_str().unwrap_or("an event");
    let when = found["start_time"].as_str().unwrap_or_default();
    Event {
        kind: "invitee.created",
        id: booked["uri"].as_str().unwrap_or_default().to_owned(),
        summary: format!("{who} booked {what} at {when}"),
        data: json!({"event": event(found), "invitee": invitee(booked)}),
    }
}

#[cfg(test)]
mod tests {
    //! Answers are trimmed from the examples in Calendly's API reference.
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    const TOKEN: &str = "eyJraWQiOiJ0ZXN0In0.calendly-test-token";
    const USER: &str = "https://api.calendly.com/users/AAAAAAAAAAAAAAAA";

    fn cx(fixture: &Fixture) -> Context {
        let credential = Credential::Fields {
            fields: [("token".into(), TOKEN.into())].into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    fn recorded(name: &str) -> Value {
        let path = format!(
            "{}/fixtures/calendly/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn me() -> Exchange {
        let check = recorded("check");
        Exchange::new(
            "GET",
            "/api.calendly.com/users/me",
            200,
            check["exchanges"][0]["body"].clone(),
        )
    }

    fn at(path: &str) -> String {
        format!("/api.calendly.com{path}")
    }

    fn query(target: &str) -> Vec<(String, String)> {
        let url = reqwest::Url::parse(&format!("http://x{target}")).unwrap();
        url.query_pairs().into_owned().collect()
    }

    #[tokio::test]
    async fn list_event_types_names_the_user_and_their_booking_links() {
        let fixture = Fixture::start(vec![
            me(),
            Exchange::new("GET", &at("/event_types"), 200, recorded("event_types")),
        ])
        .await;
        let listed = Calendly
            .call(&cx(&fixture), "list_event_types", json!({"active": true}))
            .await
            .unwrap();
        assert_eq!(listed["user"]["email"], "ana@example.com");
        assert_eq!(
            listed["event_types"][0],
            json!({"uri": "https://api.calendly.com/event_types/BBBBBBBBBBBBBBBB", "name": "30 Minute Meeting", "active": true, "duration": 30, "kind": "solo", "scheduling_url": "https://calendly.com/ana/30min"})
        );
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].header("authorization"),
            Some(&*format!("Bearer {TOKEN}"))
        );
        assert_eq!(
            query(&seen[1].target),
            [
                ("user".into(), USER.into()),
                ("count".into(), "100".into()),
                ("active".into(), "true".into())
            ]
        );
    }

    #[tokio::test]
    async fn list_events_filters_a_range_and_pages_with_a_cursor() {
        let fixture = Fixture::start(vec![
            me(),
            Exchange::new("GET", &at("/scheduled_events"), 200, recorded("events")),
        ])
        .await;
        let input = json!({"from": "2026-11-05T00:00:00Z", "to": "2026-11-12T00:00:00Z", "status": "active", "limit": 5, "cursor": "sNjq4TvMDfUHEl7zHRR0k0E1PCEJWvdi"});
        let listed = Calendly
            .call(&cx(&fixture), "list_events", input)
            .await
            .unwrap();
        assert_eq!(
            listed["events"][0],
            json!({
                "id": "GBGBDCAADAEDCRZ2", "uri": "https://api.calendly.com/scheduled_events/GBGBDCAADAEDCRZ2",
                "name": "30 Minute Meeting", "status": "active",
                "start": "2026-11-06T15:00:00.000000Z", "end": "2026-11-06T15:30:00.000000Z",
                "event_type": "https://api.calendly.com/event_types/BBBBBBBBBBBBBBBB",
                "location": {"type": "zoom", "where": null, "join_url": "https://calendly.zoom.us/j/123"},
                "invitees": 1,
            })
        );
        assert_eq!(listed["next"], "VJs2rfDYeY8ahZpq0QI1O114LJkNjd7H");
        let seen = fixture.finish().await;
        assert_eq!(
            query(&seen[1].target),
            [
                ("user".into(), USER.into()),
                ("count".into(), "5".into()),
                ("sort".into(), "start_time:asc".into()),
                ("min_start_time".into(), "2026-11-05T00:00:00Z".into()),
                ("max_start_time".into(), "2026-11-12T00:00:00Z".into()),
                ("status".into(), "active".into()),
                (
                    "page_token".into(),
                    "sNjq4TvMDfUHEl7zHRR0k0E1PCEJWvdi".into()
                ),
            ]
        );
    }

    #[tokio::test]
    async fn read_event_takes_a_uri_and_adds_its_invitees() {
        let event = recorded("events")["collection"][0].clone();
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                &at("/scheduled_events/GBGBDCAADAEDCRZ2"),
                200,
                json!({"resource": event}),
            ),
            Exchange::new(
                "GET",
                &at("/scheduled_events/GBGBDCAADAEDCRZ2/invitees"),
                200,
                recorded("invitees"),
            ),
        ])
        .await;
        let input = json!({"event": "https://api.calendly.com/scheduled_events/GBGBDCAADAEDCRZ2"});
        let read = Calendly
            .call(&cx(&fixture), "read_event", input)
            .await
            .unwrap();
        assert_eq!(read["name"], "30 Minute Meeting");
        assert_eq!(
            read["invitees"][0],
            json!({
                "uri": "https://api.calendly.com/scheduled_events/GBGBDCAADAEDCRZ2/invitees/IIIIIIIIIIIIIIII",
                "name": "John Doe", "email": "john@example.com", "status": "active",
                "timezone": "America/New_York", "created_at": "2026-11-05T13:00:00.000000Z",
                "rescheduled": false, "no_show": false,
                "answers": [{"question": "What should we talk about?", "answer": "The roadmap"}],
            })
        );
        assert_eq!(read["more_invitees"], false);
        fixture.finish().await;
    }

    #[tokio::test]
    async fn bad_input_is_refused_before_reaching_calendly() {
        let fixture = Fixture::start(Vec::new()).await;
        let cx = cx(&fixture);
        for (tool, input) in [
            ("read_event", json!({"event": "../users/me?"})),
            ("read_event", json!({"event": ""})),
            ("list_events", json!({"since": "yesterday"})),
            ("list_event_types", json!({"active": "yes"})),
        ] {
            let refused = Calendly.call(&cx, tool, input.clone()).await;
            assert!(matches!(refused, Err(Error::Invalid(_))), "{tool} {input}");
        }
        assert!(fixture.finish().await.is_empty());
    }

    #[tokio::test]
    async fn refusals_map_to_errors() {
        let cases = [
            (
                401,
                json!({"title": "Unauthenticated", "message": "The access token is invalid"}),
                Error::Unauthorized,
            ),
            (
                403,
                json!({"title": "Insufficient scope", "message": "This operation requires the scopes listed in the 'required_scopes' array.", "required_scopes": ["users:read"]}),
                Error::Forbidden(
                    "This operation requires the scopes listed in the 'required_scopes' array."
                        .into(),
                ),
            ),
            (
                404,
                json!({"title": "Resource Not Found", "message": "The server could not find the requested resource."}),
                Error::NotFound("The server could not find the requested resource.".into()),
            ),
            (
                429,
                json!({"title": "Too Many Requests", "message": "Rate limit exceeded"}),
                Error::Unavailable("rate limited: Rate limit exceeded".into()),
            ),
        ];
        for (status, body, expected) in cases {
            let answer = Exchange::new("GET", "/api.calendly.com/users/me", status, body)
                .header("retry-after", "3600");
            let fixture = Fixture::start(vec![answer]).await;
            assert_eq!(
                Calendly.check(&cx(&fixture)).await,
                Err(expected),
                "{status}"
            );
            fixture.finish().await;
        }
    }

    fn upcoming(total: u32) -> Value {
        let mut events = recorded("events");
        events["collection"][0]["invitees_counter"]["total"] = json!(total);
        events
    }

    #[tokio::test]
    async fn poll_records_a_baseline_then_reports_each_new_booking_once() {
        let mut invitees = recorded("invitees");
        let mut second = invitees["collection"][0].clone();
        second["uri"] = json!(
            "https://api.calendly.com/scheduled_events/GBGBDCAADAEDCRZ2/invitees/JJJJJJJJJJJJJJJJ"
        );
        second["name"] = json!("Ana Lima");
        second["email"] = json!("ana.lima@example.com");
        second["created_at"] = json!("2026-11-05T14:00:00.000000Z");
        invitees["collection"] = json!([second, invitees["collection"][0]]);
        let fixture = Fixture::start(vec![
            me(),
            Exchange::new("GET", &at("/scheduled_events"), 200, upcoming(1)),
            me(),
            Exchange::new("GET", &at("/scheduled_events"), 200, upcoming(2)),
            Exchange::new(
                "GET",
                &at("/scheduled_events/GBGBDCAADAEDCRZ2/invitees"),
                200,
                invitees,
            ),
            me(),
            Exchange::new("GET", &at("/scheduled_events"), 200, upcoming(2)),
        ])
        .await;
        let cx = cx(&fixture);
        let baseline = Calendly.poll(&cx, None).await.unwrap();
        assert!(baseline.events.is_empty());
        let mut cursor: Value = serde_json::from_str(baseline.cursor.as_deref().unwrap()).unwrap();
        assert_eq!(cursor["seen"], json!(["GBGBDCAADAEDCRZ2@1"]));
        // As if the baseline ran between the two bookings.
        cursor["after"] = json!("2026-11-05T13:30:00.000000Z");
        let next = Calendly.poll(&cx, Some(&cursor.to_string())).await.unwrap();
        assert_eq!(next.events.len(), 1, "{:?}", next.events);
        let event = &next.events[0];
        assert_eq!(event.kind, "invitee.created");
        assert_eq!(
            event.id,
            "https://api.calendly.com/scheduled_events/GBGBDCAADAEDCRZ2/invitees/JJJJJJJJJJJJJJJJ"
        );
        assert_eq!(
            event.summary,
            "Ana Lima booked 30 Minute Meeting at 2026-11-06T15:00:00.000000Z"
        );
        assert_eq!(event.data["invitee"]["email"], "ana.lima@example.com");
        let again = Calendly.poll(&cx, next.cursor.as_deref()).await.unwrap();
        assert!(
            again.events.is_empty(),
            "an unchanged event is not fetched again"
        );
        let seen = fixture.finish().await;
        let listed = query(&seen[1].target);
        assert_eq!(listed[1], ("status".into(), "active".into()));
        assert_eq!(listed[2].0, "min_start_time");
        assert!(listed[2].1.ends_with(".000000Z"));
        assert_eq!(
            query(&seen[4].target),
            [
                ("status".into(), "active".into()),
                ("sort".into(), "created_at:desc".into()),
                ("count".into(), "100".into())
            ]
        );
    }

    #[test]
    fn now_orders_with_calendly_times() {
        let now = now();
        assert_eq!(now.len(), "2026-11-05T13:00:00.000000Z".len());
        assert!(now.as_str() > "2026-11-05T13:00:00.000000Z");
    }
}

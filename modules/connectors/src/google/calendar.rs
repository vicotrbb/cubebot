//! Google Calendar (https://developers.google.com/calendar/api/v3/reference). Only invitations
//! and responses notify anyone; creating and deleting are silent.
use super::{at, clip, now, rfc3339};
use crate::{Context, Error, Event};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

const CALENDAR: &str = "https://www.googleapis.com/calendar/v3";
/// How far ahead `event.starting` looks, in seconds.
const AHEAD: i64 = 15 * 60;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Events {
    calendar: Option<String>,
    from: Option<String>,
    to: Option<String>,
    query: Option<String>,
    limit: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FreeBusy {
    from: String,
    to: String,
    #[serde(default)]
    calendars: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    calendar: Option<String>,
    summary: String,
    start: String,
    end: String,
    time_zone: Option<String>,
    description: Option<String>,
    location: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invite {
    calendar: Option<String>,
    event: String,
    attendees: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Respond {
    calendar: Option<String>,
    event: String,
    response: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct One {
    calendar: Option<String>,
    event: String,
}

pub(super) async fn call(cx: &Context, name: &str, input: Value) -> Result<Value, Error> {
    match name {
        "list" => list(cx).await,
        "events" => events(cx, input).await,
        "freebusy" => freebusy(cx, input).await,
        "create_event" => create(cx, input).await,
        "invite" => invite(cx, input).await,
        "respond" => respond(cx, input).await,
        "delete_event" => {
            let one: One = crate::input(input)?;
            let path = event_path(one.calendar.as_deref(), &one.event);
            let request = cx.request(Method::DELETE, &path)?;
            cx.send(request.query(&[("sendUpdates", "none")])).await?;
            Ok(json!({"deleted": one.event}))
        }
        _ => Err(Error::UnknownTool),
    }
}

fn events_path(calendar: Option<&str>) -> String {
    at(
        CALENDAR,
        &["calendars", calendar.unwrap_or("primary"), "events"],
    )
}

fn event_path(calendar: Option<&str>, event: &str) -> String {
    at(&events_path(calendar), &[event])
}

async fn list(cx: &Context) -> Result<Value, Error> {
    let found = cx
        .json(cx.get(&format!("{CALENDAR}/users/me/calendarList"))?)
        .await?;
    let calendars: Vec<Value> = found["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|calendar| {
            json!({
                "id": calendar["id"],
                "summary": calendar["summary"],
                "primary": calendar["primary"],
                "access": calendar["accessRole"],
                "time_zone": calendar["timeZone"],
            })
        })
        .collect();
    Ok(json!({"calendars": calendars}))
}

async fn events(cx: &Context, input: Value) -> Result<Value, Error> {
    let wanted: Events = crate::input(input)?;
    let limit = wanted.limit.unwrap_or(25).clamp(1, 100).to_string();
    let from = wanted.from.unwrap_or_else(|| rfc3339(now()));
    let mut query = vec![
        ("singleEvents", "true"),
        ("orderBy", "startTime"),
        ("maxResults", &limit),
        ("timeMin", &from),
    ];
    if let Some(to) = &wanted.to {
        query.push(("timeMax", to));
    }
    if let Some(text) = &wanted.query {
        query.push(("q", text));
    }
    let request = cx.get(&events_path(wanted.calendar.as_deref()))?;
    let found = cx.json(request.query(&query)).await?;
    let items = found["items"].as_array().into_iter().flatten();
    Ok(json!({"events": items.map(event).collect::<Vec<_>>()}))
}

/// An event as the model needs it, its description cut short.
fn event(event: &Value) -> Value {
    let attendees: Vec<Value> = event["attendees"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|guest| json!({"email": guest["email"], "response": guest["responseStatus"]}))
        .collect();
    json!({
        "id": event["id"],
        "summary": event["summary"],
        "start": when(&event["start"]),
        "end": when(&event["end"]),
        "location": event["location"],
        "status": event["status"],
        "organizer": event["organizer"]["email"],
        "attendees": attendees,
        "meet": event["hangoutLink"],
        "link": event["htmlLink"],
        "description": clip(event["description"].as_str().unwrap_or_default(), 1000),
    })
}

fn when(time: &Value) -> Value {
    match &time["dateTime"] {
        Value::Null => time["date"].clone(),
        timed => timed.clone(),
    }
}

async fn freebusy(cx: &Context, input: Value) -> Result<Value, Error> {
    let wanted: FreeBusy = crate::input(input)?;
    let calendars = match wanted.calendars.is_empty() {
        true => vec!["primary".to_owned()],
        false => wanted.calendars,
    };
    let items: Vec<Value> = calendars.iter().map(|id| json!({"id": id})).collect();
    let body = json!({"timeMin": wanted.from, "timeMax": wanted.to, "items": items});
    let found = cx
        .json(cx.post(&format!("{CALENDAR}/freeBusy"))?.json(&body))
        .await?;
    Ok(json!({"calendars": found["calendars"]}))
}

async fn create(cx: &Context, input: Value) -> Result<Value, Error> {
    let create: Create = crate::input(input)?;
    let zone = create.time_zone.as_deref();
    let body = json!({
        "summary": create.summary,
        "start": time(&create.start, zone),
        "end": time(&create.end, zone),
        "description": create.description,
        "location": create.location,
    });
    let request = cx
        .post(&events_path(create.calendar.as_deref()))?
        .query(&[("sendUpdates", "none")]);
    let made = cx.json(request.json(&body)).await?;
    Ok(event(&made))
}

/// A date alone makes an all-day event.
fn time(text: &str, zone: Option<&str>) -> Value {
    match text.len() {
        10 => json!({"date": text}),
        _ => json!({"dateTime": text, "timeZone": zone}),
    }
}

/// Adds guests and emails them; the guests already invited are kept.
async fn invite(cx: &Context, input: Value) -> Result<Value, Error> {
    let invite: Invite = crate::input(input)?;
    let path = event_path(invite.calendar.as_deref(), &invite.event);
    let found = cx.json(cx.get(&path)?).await?;
    let mut attendees = found["attendees"].as_array().cloned().unwrap_or_default();
    for email in invite.attendees {
        let known = attendees
            .iter()
            .any(|guest| guest["email"].as_str() == Some(&email));
        if !known {
            attendees.push(json!({"email": email}));
        }
    }
    update(cx, &path, attendees).await
}

/// Sets this account's own answer and tells the organizer.
async fn respond(cx: &Context, input: Value) -> Result<Value, Error> {
    let respond: Respond = crate::input(input)?;
    let path = event_path(respond.calendar.as_deref(), &respond.event);
    let found = cx.json(cx.get(&path)?).await?;
    let mut attendees = found["attendees"].as_array().cloned().unwrap_or_default();
    let me = attendees
        .iter_mut()
        .find(|guest| guest["self"] == true)
        .ok_or_else(|| Error::Invalid("this account is not a guest of that event".into()))?;
    me["responseStatus"] = json!(respond.response);
    update(cx, &path, attendees).await
}

async fn update(cx: &Context, path: &str, attendees: Vec<Value>) -> Result<Value, Error> {
    let request = cx
        .request(Method::PATCH, path)?
        .query(&[("sendUpdates", "all")]);
    let changed = cx
        .json(request.json(&json!({"attendees": attendees})))
        .await?;
    Ok(event(&changed))
}

/// Primary calendar events starting within [`AHEAD`]. The cursor holds the ones already
/// reported that have not started yet, each as `id@start` so a moved event is reported again.
pub(super) async fn poll(
    cx: &Context,
    cursor: &Value,
    events: &mut Vec<Event>,
) -> Result<Value, Error> {
    let now = now();
    let request = cx.get(&events_path(None))?.query(&[
        ("singleEvents", "true"),
        ("orderBy", "startTime"),
        ("maxResults", "50"),
        ("timeMin", &rfc3339(now)),
        ("timeMax", &rfc3339(now + AHEAD)),
    ]);
    let found = cx.json(request).await?;
    // The window holds events that end after now, so those already under way are left out.
    let starting: Vec<(String, &Value)> = found["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let (id, start) = (item["id"].as_str()?, item["start"]["dateTime"].as_str()?);
            let at = mail_parser::DateTime::parse_rfc3339(start)?.to_timestamp();
            (at >= now).then(|| (format!("{id}@{start}"), item))
        })
        .collect();
    if let Some(reported) = cursor.as_array() {
        let fresh = starting
            .iter()
            .filter(|(key, _)| !reported.iter().any(|seen| seen == key.as_str()));
        events.extend(fresh.map(|(key, item)| starts(key, item)));
    }
    Ok(json!(
        starting.into_iter().map(|(key, _)| key).collect::<Vec<_>>()
    ))
}

fn starts(key: &str, item: &Value) -> Event {
    let data = event(item);
    let summary = data["summary"].as_str().unwrap_or("An event");
    let start = data["start"].as_str().unwrap_or_default();
    Event {
        kind: "event.starting",
        id: key.to_owned(),
        summary: clip(&format!("{summary} starts at {start}"), 200),
        data,
    }
}

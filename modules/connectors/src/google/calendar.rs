//! Google Calendar (https://developers.google.com/calendar/api/v3/reference). Only invitations
//! and responses notify anyone; creating, changing and deleting are silent.
use super::{at, clip, now, rfc3339};
use crate::{Context, Error, Event};
use chrono::{NaiveDateTime, TimeDelta, TimeZone};
use chrono_tz::Tz;
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Map, Value, json};

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
    limit: Option<u16>,
    page: Option<String>,
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
struct Change {
    calendar: Option<String>,
    event: String,
    all: Option<bool>,
    summary: Option<String>,
    start: Option<String>,
    end: Option<String>,
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
        "update_event" => change(cx, input).await,
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
    let limit = wanted.limit.unwrap_or(25).clamp(1, 250).to_string();
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
    if let Some(page) = &wanted.page {
        query.push(("pageToken", page));
    }
    let request = cx.get(&events_path(wanted.calendar.as_deref()))?;
    let found = cx.json(request.query(&query)).await?;
    let calendar = wanted.calendar.as_deref().unwrap_or("primary");
    let items = found["items"].as_array().into_iter().flatten();
    let events: Vec<Value> = items.map(|item| in_calendar(item, calendar)).collect();
    Ok(json!({"events": events, "next_page": found["nextPageToken"]}))
}

/// [`event`] with the calendar it was read from.
fn in_calendar(item: &Value, calendar: &str) -> Value {
    let mut made = event(item);
    made["calendar"] = json!(calendar);
    made
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
        "series": event["recurringEventId"],
        "all_day": event["start"]["date"].is_string(),
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
        _ => match zone {
            Some(zone) => json!({"dateTime": text, "timeZone": zone}),
            None => json!({"dateTime": text}),
        },
    }
}

/// Google merges a patched start into the old one, so the key of the other kind is cleared.
fn exclusive(mut time: Value) -> Value {
    let other = if time["date"].is_null() {
        "date"
    } else {
        "dateTime"
    };
    time[other] = Value::Null;
    time
}

/// Patches the event, or with `all` the series its occurrence belongs to; nobody is notified.
async fn change(cx: &Context, input: Value) -> Result<Value, Error> {
    let change: Change = crate::input(input)?;
    let (mut body, times) = fields(&change)?;
    let calendar = change.calendar.as_deref();
    let mut path = event_path(calendar, &change.event);
    let mut series = None;
    if change.all == Some(true) {
        let instance = cx.json(cx.get(&path)?).await?;
        if let Some(master) = instance["recurringEventId"].as_str() {
            path = event_path(calendar, master);
            series = Some(instance);
        }
    }
    if let Some(times) = times {
        let master = match &series {
            Some(_) => cx.json(cx.get(&path)?).await?,
            None => Value::Null,
        };
        let pair = series.as_ref().map(|instance| (instance, &master));
        let (start, end) = moved(pair, times, change.time_zone.as_deref())?;
        body.insert("start".into(), exclusive(start));
        body.insert("end".into(), exclusive(end));
    }
    let request = cx
        .request(Method::PATCH, &path)?
        .query(&[("sendUpdates", "none")]);
    let changed = cx.json(request.json(&body)).await?;
    Ok(in_calendar(&changed, calendar.unwrap_or("primary")))
}

/// A start and an end, as the caller wrote them.
type Times<'a> = (&'a str, &'a str);

/// The fields to patch apart from the times, and the times when both are given.
fn fields(change: &Change) -> Result<(Map<String, Value>, Option<Times<'_>>), Error> {
    let mut body = Map::new();
    for (key, text) in [
        ("summary", &change.summary),
        ("description", &change.description),
        ("location", &change.location),
    ] {
        if let Some(text) = text {
            body.insert(key.into(), json!(text));
        }
    }
    let times = match (&change.start, &change.end) {
        (Some(start), Some(end)) => Some((start.as_str(), end.as_str())),
        (None, None) => None,
        _ => return Err(Error::Invalid("start and end are given together".into())),
    };
    if body.is_empty() && times.is_none() {
        return Err(Error::Invalid("nothing to change".into()));
    }
    Ok((body, times))
}

/// The new start and end: as given, or for a series the first occurrence moved as far as the
/// edited occurrence on the wall clock of the series' own zone (`zone` only when it has none, as
/// an all-day series), keeping the new length.
fn moved(
    series: Option<(&Value, &Value)>,
    (start, end): Times,
    zone: Option<&str>,
) -> Result<(Value, Value), Error> {
    let Some((instance, master)) = series else {
        return Ok((time(start, zone), time(end, zone)));
    };
    let name = master["start"]["timeZone"]
        .as_str()
        .or(zone)
        .ok_or_else(|| Error::Invalid("the series has no time zone; pass time_zone".into()))?;
    let tz: Tz = name
        .parse()
        .map_err(|_| Error::Invalid(format!("{name} is not a time zone")))?;
    let local = |text: &str| -> Result<NaiveDateTime, Error> {
        let at = seconds(text)?;
        let moment = tz.timestamp_opt(at, 0).single();
        moment
            .map(|m| m.naive_local())
            .ok_or_else(|| Error::Invalid(format!("{text} is not a time")))
    };
    let known = |time: &Value| local(when(time).as_str().unwrap_or_default());
    let first = known(&master["start"])? + (local(start)? - known(&instance["originalStartTime"])?);
    let length = TimeDelta::seconds(seconds(end)? - seconds(start)?);
    let render = |at: NaiveDateTime, like: &str| match like.len() {
        10 => json!({"date": at.format("%Y-%m-%d").to_string()}),
        _ => time(&at.format("%Y-%m-%dT%H:%M:%S").to_string(), Some(name)),
    };
    Ok((render(first, start), render(first + length, end)))
}

/// A date or an RFC 3339 time as seconds since the epoch.
fn seconds(time: &str) -> Result<i64, Error> {
    let full = match time.len() {
        10 => format!("{time}T00:00:00Z"),
        _ => time.to_owned(),
    };
    mail_parser::DateTime::parse_rfc3339(&full)
        .map(|parsed| parsed.to_timestamp())
        .ok_or_else(|| Error::Invalid(format!("{time} is not a date or an RFC 3339 time")))
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

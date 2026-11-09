//! What each calendar connector names its tools, how its answers read (one normalizer per
//! connector) and the input each write takes.
use super::{
    ChangeEvent, EventDetail, Item, NewEvent, Owned, RemoveEvent, Scope, Source, shown, text, utc,
};
use crate::connections::{ConnectError, Connections};
use chrono::DateTime;
use chrono_tz::Tz;
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
use std::time::Duration;
use uuid::Uuid;

/// Pages of one calendar's events read at most, in case a vendor never stops paging.
const PAGES: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Vendor {
    Google,
    Microsoft,
    Dav,
}

pub(super) struct Tools {
    pub list: &'static str,
    pub events: &'static str,
    pub create: &'static str,
    pub update: &'static str,
    pub delete: &'static str,
}

/// Google and Microsoft name their calendar tools alike.
static CALENDAR_TOOLS: Tools = Tools {
    list: "calendar_list",
    events: "calendar_events",
    create: "calendar_create_event",
    update: "calendar_update_event",
    delete: "calendar_delete_event",
};

static DAV_TOOLS: Tools = Tools {
    list: "list_calendars",
    events: "list_events",
    create: "create_event",
    update: "update_event",
    delete: "delete_event",
};

/// A calendar a connection lists.
struct Listed {
    id: String,
    name: String,
    color: Option<String>,
    /// The account may change its events.
    writable: bool,
}

/// Where an event was read, and whether it can be written there.
pub(super) struct Place<'a> {
    connection: Uuid,
    calendar: &'a str,
    source: &'a str,
    editable: bool,
}

/// How an event names itself in writes.
#[derive(Default)]
struct Ids {
    event: String,
    series: Option<String>,
    occurrence: Option<String>,
    etag: Option<String>,
    time_zone: Option<String>,
}

impl Vendor {
    pub(super) fn of(connector: &str) -> Option<Self> {
        match connector {
            "google" => Some(Self::Google),
            "microsoft" => Some(Self::Microsoft),
            "dav" => Some(Self::Dav),
            _ => None,
        }
    }

    fn connector(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Microsoft => "microsoft",
            Self::Dav => "dav",
        }
    }

    pub(super) fn tools(self) -> &'static Tools {
        match self {
            Self::Dav => &DAV_TOOLS,
            Self::Google | Self::Microsoft => &CALENDAR_TOOLS,
        }
    }

    fn calendars(self, answer: &Value) -> Vec<Listed> {
        let (list, id, name) = match self {
            Self::Google => ("calendars", "id", "summary"),
            Self::Microsoft => ("calendars", "id", "name"),
            Self::Dav => ("collections", "href", "name"),
        };
        let listed = answer[list].as_array().into_iter().flatten();
        listed
            .filter_map(|calendar| {
                let id = text(&calendar[id])?;
                Some(Listed {
                    name: text(&calendar[name]).unwrap_or_else(|| id.clone()),
                    color: text(&calendar["color"]).filter(|color| color.starts_with('#')),
                    writable: match self {
                        Self::Google => {
                            matches!(calendar["access"].as_str(), Some("owner" | "writer"))
                        }
                        Self::Microsoft => calendar["can_edit"] == true,
                        Self::Dav => true,
                    },
                    id,
                })
            })
            .collect()
    }

    fn events_input(self, calendar: &str, (from, to): (i64, i64), page: Option<&str>) -> Value {
        let (from, to) = (utc(from), utc(to));
        match self {
            Self::Google => {
                json!({"calendar": calendar, "from": from, "to": to, "limit": 250, "page": page})
            }
            // Its default is 50; it pages up to 500 itself.
            Self::Microsoft => {
                json!({"calendar": calendar, "start": from, "end": to, "limit": 500})
            }
            Self::Dav => json!({"calendar": calendar, "start": from, "end": to, "limit": 2000}),
        }
    }

    fn item(self, event: &Value, place: &Place) -> Option<Item> {
        match self {
            Self::Google => normalize_google(event, place),
            Self::Microsoft => normalize_microsoft(event, place),
            Self::Dav => normalize_dav(event, place),
        }
    }

    pub(super) fn create_input(self, new: &NewEvent, zone: &str) -> Result<Value, ConnectError> {
        Ok(match self {
            Self::Google => json!({
                "calendar": new.calendar, "summary": new.title, "start": new.start, "end": new.end,
                "time_zone": zone, "description": new.description, "location": new.location,
            }),
            Self::Microsoft => json!({
                "calendar": new.calendar, "subject": new.title, "start": local(&new.start, zone)?,
                "end": local(&new.end, zone)?, "all_day": new.all_day, "time_zone": zone,
                "body": new.description, "location": new.location,
            }),
            Self::Dav => json!({
                "calendar": new.calendar, "summary": new.title, "start": new.start, "end": new.end,
                "description": new.description, "location": new.location,
            }),
        })
    }

    /// The update of the event or occurrence named; with `all` the connector changes the
    /// series it belongs to, moving every occurrence as far as this one moves.
    pub(super) fn update_input(
        self,
        change: &ChangeEvent,
        zone: &str,
    ) -> Result<Value, ConnectError> {
        let mut input = match self {
            Self::Google => json!({
                "calendar": change.calendar, "event": change.event, "summary": change.title,
                "start": change.start, "end": change.end, "time_zone": zone,
                "description": change.description, "location": change.location,
            }),
            Self::Microsoft => {
                let wall = |time: &Option<String>| {
                    time.as_deref().map(|time| local(time, zone)).transpose()
                };
                let dated = change.start.as_ref().map(|start| start.len() == 10);
                json!({
                    "event": change.event, "subject": change.title, "start": wall(&change.start)?,
                    "end": wall(&change.end)?, "all_day": change.all_day.or(dated), "time_zone": zone,
                    "body": change.description, "location": change.location,
                })
            }
            // The occurrence's own zone names it, never the viewer's.
            Self::Dav => json!({
                "event": change.event, "etag": change.etag, "occurrence": change.occurrence,
                "time_zone": change.time_zone, "summary": change.title, "start": change.start,
                "end": change.end, "description": change.description, "location": change.location,
            }),
        };
        if change.scope == Scope::All {
            input["all"] = json!(true);
        }
        Ok(input)
    }

    pub(super) fn delete_input(self, remove: &RemoveEvent) -> Value {
        let all = remove.scope == Scope::All;
        match self {
            Self::Google => {
                let event = match all {
                    true => remove.series.as_ref().unwrap_or(&remove.event),
                    false => &remove.event,
                };
                json!({"calendar": remove.calendar, "event": event})
            }
            Self::Microsoft => json!({"event": remove.event, "all": all}),
            // Without its occurrence the whole event goes, series and all.
            Self::Dav if all => json!({"event": remove.event, "etag": remove.etag}),
            Self::Dav => json!({
                "event": remove.event, "etag": remove.etag, "occurrence": remove.occurrence,
                "time_zone": remove.time_zone,
            }),
        }
    }
}

/// How long one connection's read may take before its source says it did not answer.
const BUDGET: Duration = Duration::from_secs(12);
/// Calendars of one connection read at the same time.
const AT_ONCE: usize = 6;

/// One connection's calendars as sources and their events in the range. A failure shows as the
/// problem of the connection's source, or of the calendar's when only that calendar failed.
pub(super) async fn read(
    connections: &Connections,
    owned: &Owned,
    bounds: (i64, i64),
) -> (Vec<Source>, Vec<Item>) {
    within(BUDGET, owned, calendars_of(connections, owned, bounds)).await
}

/// The read, or the connection's source with a problem when it takes longer than `budget`.
async fn within(
    budget: Duration,
    owned: &Owned,
    read: impl Future<Output = (Vec<Source>, Vec<Item>)>,
) -> (Vec<Source>, Vec<Item>) {
    tokio::time::timeout(budget, read)
        .await
        .unwrap_or_else(|_| {
            let source = troubled(owned, "Didn't answer in time.".into());
            (vec![source], Vec::new())
        })
}

async fn calendars_of(
    connections: &Connections,
    owned: &Owned,
    bounds: (i64, i64),
) -> (Vec<Source>, Vec<Item>) {
    let tools = owned.vendor.tools();
    let listed = match owned.status.as_str() {
        "sign_in_again" => Err(ConnectError::SignInAgain),
        _ => connections.call_tool(owned.id, tools.list, json!({})).await,
    };
    let calendars = match listed {
        Ok(answer) => owned.vendor.calendars(&answer),
        Err(error) => return (vec![troubled(owned, problem(&error))], Vec::new()),
    };
    let writable = owned.offered.contains(&tools.create);
    // Collected first: a lazy map would make the future not `Send` for every lifetime.
    let read: Vec<_> = calendars
        .iter()
        .map(|calendar| events_of(connections, owned, &calendar.id, bounds))
        .collect();
    let found: Vec<_> = stream::iter(read).buffered(AT_ONCE).collect().await;
    let (mut sources, mut items) = (Vec::new(), Vec::new());
    for (calendar, found) in calendars.iter().zip(found) {
        let id = format!("calendar:{}:{}", owned.id, calendar.id);
        let place = Place {
            connection: owned.id,
            calendar: &calendar.id,
            source: &id,
            editable: writable && calendar.writable,
        };
        let mut source = source(owned, id.clone(), &calendar.name);
        source.color.clone_from(&calendar.color);
        source.writable = place.editable;
        match found {
            Ok((events, more)) => {
                let read = events
                    .iter()
                    .filter_map(|event| owned.vendor.item(event, &place));
                items.extend(read);
                // Never drop events silently: say the range was cut short.
                source.problem = more
                    .then(|| format!("Showing the first {} events of this range.", events.len()));
            }
            Err(error) => source.problem = Some(problem(&error)),
        }
        sources.push(source);
    }
    (sources, items)
}

/// The connection's own source, when none of its calendars could be read.
fn troubled(owned: &Owned, problem: String) -> Source {
    let mut source = source(owned, format!("calendar:{}", owned.id), &owned.name);
    source.problem = Some(problem);
    source
}

fn source(owned: &Owned, id: String, name: &str) -> Source {
    Source {
        id,
        kind: "calendar",
        name: name.to_owned(),
        account: Some(owned.name.clone()),
        connector: Some(owned.vendor.connector().to_owned()),
        connection: Some(owned.id),
        color: None,
        writable: false,
        problem: None,
    }
}

/// Every event of one calendar in the range, page after page.
async fn events_of(
    connections: &Connections,
    owned: &Owned,
    calendar: &str,
    bounds: (i64, i64),
) -> Result<(Vec<Value>, bool), ConnectError> {
    let mut found = Vec::new();
    let mut page = None;
    for _ in 0..PAGES {
        let input = owned.vendor.events_input(calendar, bounds, page.as_deref());
        let tool = owned.vendor.tools().events;
        let answer = connections.call_tool(owned.id, tool, input).await?;
        found.extend(answer["events"].as_array().into_iter().flatten().cloned());
        page = text(&answer["next_page"]);
        if page.is_none() {
            return Ok((found, answer["more"] == true));
        }
    }
    Ok((found, true))
}

fn problem(error: &ConnectError) -> String {
    match error {
        ConnectError::SignInAgain => "The account refused the credential; sign in again.".into(),
        ConnectError::Refused(text) | ConnectError::Invalid(text) => text.clone(),
        ConnectError::Identity(_) => "The connection could not be opened.".into(),
        // A read changes nothing, so only the reason is worth saying: "Could not reach the server."
        ConnectError::Unavailable(reason) => sentence(reason),
    }
}

/// A capital first letter and a full stop, unless the reason already ends one.
fn sentence(reason: &str) -> String {
    let mut chars = reason.chars();
    let first: String = chars
        .next()
        .map(char::to_uppercase)
        .into_iter()
        .flatten()
        .collect();
    let stop = if reason.ends_with(['.', '!', '?']) {
        ""
    } else {
        "."
    };
    format!("{first}{}{stop}", chars.as_str())
}

/// An instance of a recurring event has its own id and names its series; cancelled ones are gone.
fn normalize_google(event: &Value, place: &Place) -> Option<Item> {
    if event["status"] == "cancelled" {
        return None;
    }
    let ids = Ids {
        event: text(&event["id"])?,
        series: text(&event["series"]),
        ..Ids::default()
    };
    item(event, "summary", ids, place)
}

/// Times come in UTC; an occurrence has its own id and names its series.
fn normalize_microsoft(event: &Value, place: &Place) -> Option<Item> {
    let ids = Ids {
        event: text(&event["id"])?,
        series: text(&event["series"]),
        ..Ids::default()
    };
    item(event, "subject", ids, place)
}

/// An occurrence is its event's href with the occurrence it is, in the event's own zone.
fn normalize_dav(event: &Value, place: &Place) -> Option<Item> {
    let ids = Ids {
        event: text(&event["href"])?,
        occurrence: text(&event["occurrence"]),
        etag: text(&event["etag"]),
        time_zone: text(&event["time_zone"]),
        ..Ids::default()
    };
    item(event, "summary", ids, place)
}

fn item(event: &Value, title: &str, ids: Ids, place: &Place) -> Option<Item> {
    let start = shown(event["start"].as_str()?)?;
    let id = match &ids.occurrence {
        Some(occurrence) => format!("{}:{}:{occurrence}", place.source, ids.event),
        None => format!("{}:{}", place.source, ids.event),
    };
    Some(Item {
        kind: "event",
        id,
        source: place.source.to_owned(),
        title: text(&event[title]).unwrap_or_default(),
        all_day: start.len() == 10,
        start,
        end: event["end"].as_str().and_then(shown),
        editable: place.editable,
        event: Some(EventDetail {
            connection: place.connection,
            calendar: place.calendar.to_owned(),
            event: ids.event,
            series: ids.series,
            occurrence: ids.occurrence,
            etag: ids.etag,
            time_zone: ids.time_zone,
            location: text(&event["location"]),
            description: text(&event["description"]),
            link: text(&event["link"]),
        }),
        routine: None,
    })
}

/// The event just made, from what was asked and the ids the vendor answered.
pub(super) fn created(new: &NewEvent, made: &Value) -> Item {
    let source = format!("calendar:{}:{}", new.connection, new.calendar);
    let event = text(&made["id"])
        .or_else(|| text(&made["href"]))
        .unwrap_or_default();
    Item {
        kind: "event",
        id: format!("{source}:{event}"),
        title: new.title.clone(),
        start: shown(&new.start).unwrap_or_else(|| new.start.clone()),
        end: shown(&new.end).or_else(|| Some(new.end.clone())),
        all_day: new.all_day,
        editable: true,
        event: Some(EventDetail {
            connection: new.connection,
            calendar: new.calendar.clone(),
            event,
            series: None,
            occurrence: None,
            etag: text(&made["etag"]),
            time_zone: None,
            location: new.location.clone(),
            description: new.description.clone(),
            link: text(&made["link"]),
        }),
        routine: None,
        source,
    }
}

/// Microsoft takes wall-clock times in the zone it is told; a date stays a date.
fn local(time: &str, zone: &str) -> Result<String, ConnectError> {
    if time.len() == 10 {
        return Ok(time.to_owned());
    }
    let invalid = || ConnectError::Invalid(format!("{time} is not an RFC 3339 time"));
    let tz: Tz = zone.parse().map_err(|_| invalid())?;
    let at = DateTime::parse_from_rfc3339(time).map_err(|_| invalid())?;
    Ok(at
        .with_timezone(&tz)
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::Owned;

    #[test]
    fn a_read_problem_is_one_sentence() {
        let said = |reason: &str| problem(&ConnectError::Unavailable(reason.into()));
        assert_eq!(
            said("could not reach the server"),
            "Could not reach the server."
        );
        assert_eq!(said("rate limited: throttled."), "Rate limited: throttled.");
    }

    fn place(source: &str) -> Place<'_> {
        Place {
            connection: Uuid::nil(),
            calendar: "work",
            source,
            editable: true,
        }
    }

    fn change(fields: Value) -> ChangeEvent {
        let mut body =
            json!({"connection": Uuid::nil(), "calendar": "work", "event": "e1", "scope": "one"});
        body.as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        serde_json::from_value(body).unwrap()
    }

    fn remove(fields: Value) -> RemoveEvent {
        let mut body =
            json!({"connection": Uuid::nil(), "calendar": "work", "event": "e1", "scope": "one"});
        body.as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        serde_json::from_value(body).unwrap()
    }

    #[tokio::test]
    async fn a_connection_that_does_not_answer_in_time_says_so() {
        let owned = Owned {
            id: Uuid::nil(),
            vendor: Vendor::Google,
            name: "work@example.com".into(),
            offered: Vec::new(),
            status: "connected".into(),
        };
        let budget = Duration::from_millis(10);
        let (sources, items) = within(budget, &owned, std::future::pending()).await;
        let source = &sources[0];
        assert_eq!(
            (source.id.as_str(), source.problem.as_deref(), items.len()),
            (
                "calendar:00000000-0000-0000-0000-000000000000",
                Some("Didn't answer in time."),
                0
            )
        );
        let quick = within(budget, &owned, async { (Vec::new(), Vec::new()) }).await;
        assert!(quick.0.is_empty());
    }

    #[test]
    fn microsoft_events_read_in_utc_and_all_day_as_dates() {
        let timed = json!({"id": "AAMk1", "subject": "Review", "start": "2026-11-05T12:00:00.0000000",
            "end": "2026-11-05T13:30:00.0000000", "series": "AAMkS", "all_day": false, "link": "https://outlook.office.com/1"});
        let item = Vendor::Microsoft
            .item(&timed, &place("calendar:x:work"))
            .unwrap();
        assert_eq!(
            (item.start.as_str(), item.end.as_deref(), item.all_day),
            ("2026-11-05T12:00:00Z", Some("2026-11-05T13:30:00Z"), false)
        );
        let detail = item.event.unwrap();
        assert_eq!(
            (detail.event.as_str(), detail.series.as_deref()),
            ("AAMk1", Some("AAMkS"))
        );
        let day = json!({"id": "AAMk2", "subject": "Off", "start": "2026-11-06", "end": "2026-11-07", "all_day": true});
        let item = Vendor::Microsoft
            .item(&day, &place("calendar:x:work"))
            .unwrap();
        assert_eq!((item.start.as_str(), item.all_day), ("2026-11-06", true));
    }

    #[test]
    fn a_dav_occurrence_is_its_href_and_occurrence_in_its_own_zone() {
        let event = json!({"href": "https://dav.example.com/cal/standup.ics", "uid": "standup", "summary": "Standup",
            "start": "2026-11-10T09:00:00Z", "end": "2026-11-10T09:15:00Z", "all_day": false,
            "occurrence": "20261110T100000", "time_zone": "Europe/Berlin", "etag": "\"7\""});
        let item = Vendor::Dav.item(&event, &place("calendar:x:work")).unwrap();
        assert_eq!(
            item.id,
            "calendar:x:work:https://dav.example.com/cal/standup.ics:20261110T100000"
        );
        let detail = item.event.unwrap();
        assert_eq!(
            (
                detail.occurrence.as_deref(),
                detail.time_zone.as_deref(),
                detail.etag.as_deref()
            ),
            (
                Some("20261110T100000"),
                Some("Europe/Berlin"),
                Some("\"7\"")
            )
        );
    }

    #[test]
    fn dav_writes_name_the_occurrence_in_its_own_zone_and_all_the_series() {
        let one = change(
            json!({"etag": "\"7\"", "occurrence": "20261110T100000", "time_zone": "Europe/Berlin", "title": "Sync"}),
        );
        let input = Vendor::Dav.update_input(&one, "").unwrap();
        assert_eq!(
            (
                &input["event"],
                &input["occurrence"],
                &input["time_zone"],
                &input["all"]
            ),
            (
                &json!("e1"),
                &json!("20261110T100000"),
                &json!("Europe/Berlin"),
                &Value::Null
            )
        );
        let all = change(
            json!({"occurrence": "20261110T100000", "time_zone": "Europe/Berlin", "scope": "all", "title": "Sync"}),
        );
        assert_eq!(Vendor::Dav.update_input(&all, "").unwrap()["all"], true);
        let one = remove(json!({"occurrence": "20261110T100000", "time_zone": "Europe/Berlin"}));
        assert_eq!(
            Vendor::Dav.delete_input(&one)["occurrence"],
            "20261110T100000"
        );
        let all = remove(
            json!({"occurrence": "20261110T100000", "time_zone": "Europe/Berlin", "scope": "all", "etag": "\"7\""}),
        );
        assert_eq!(
            Vendor::Dav.delete_input(&all),
            json!({"event": "e1", "etag": "\"7\""})
        );
    }

    #[test]
    fn microsoft_writes_take_wall_clock_times_in_the_viewers_zone() {
        let moved = change(
            json!({"scope": "all", "start": "2026-11-10T12:00:00Z", "end": "2026-11-10T13:00:00Z"}),
        );
        let input = Vendor::Microsoft
            .update_input(&moved, "America/Sao_Paulo")
            .unwrap();
        assert_eq!(
            (
                &input["start"],
                &input["end"],
                &input["all_day"],
                &input["all"],
                &input["time_zone"]
            ),
            (
                &json!("2026-11-10T09:00:00"),
                &json!("2026-11-10T10:00:00"),
                &json!(false),
                &json!(true),
                &json!("America/Sao_Paulo")
            )
        );
        let all = remove(json!({"scope": "all", "series": "AAMkS"}));
        assert_eq!(
            Vendor::Microsoft.delete_input(&all),
            json!({"event": "e1", "all": true})
        );
    }
}

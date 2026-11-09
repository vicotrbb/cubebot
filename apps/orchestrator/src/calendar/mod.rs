//! The calendar (docs/adr/console.md#a-calendar-of-the-persons-events-and-their-agents-schedules):
//! a person's own calendar connections' events and their own agents' scheduled routines on one
//! timeline. Nothing is stored: every read asks the vendors, and routines are read from their
//! schedules and turns.
mod events;
mod routines;

use crate::{
    connections::{ConnectError, Connections},
    identity::{Actor, IdentityError},
};
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use cubebot_connectors::Level;
use events::Vendor;
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// The longest range one read covers, in seconds.
const LONGEST: i64 = 62 * 86_400;

#[derive(Serialize, utoipa::ToSchema)]
pub struct CalendarRange {
    pub sources: Vec<Source>,
    /// In order of start.
    pub items: Vec<Item>,
}

/// One calendar of a connection, or one agent.
#[derive(Serialize, utoipa::ToSchema)]
pub struct Source {
    /// `calendar:<connection>:<calendar>`, `calendar:<connection>` for a connection whose
    /// calendars could not be listed, or `agent:<agent>`.
    pub id: String,
    /// `calendar` or `agent`.
    pub kind: &'static str,
    pub name: String,
    /// The connection's account a calendar belongs to.
    pub account: Option<String>,
    pub connector: Option<String>,
    pub connection: Option<Uuid>,
    /// A hex color the vendor gives the calendar.
    pub color: Option<String>,
    /// Its events can be created, changed and deleted from the calendar.
    pub writable: bool,
    /// Why its events are missing, worded for the person.
    pub problem: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Item {
    /// `event`, `routine` (an upcoming run), `run` (a run that happened) or `digest` (a day of a
    /// routine that runs more than hourly).
    pub kind: &'static str,
    /// Stable across reads.
    pub id: String,
    pub source: String,
    /// Untrusted vendor text for an event.
    pub title: String,
    /// RFC 3339 in UTC, or `YYYY-MM-DD` when `all_day`.
    pub start: String,
    /// Exclusive: the day after for an all-day item.
    pub end: Option<String>,
    pub all_day: bool,
    pub editable: bool,
    pub event: Option<EventDetail>,
    pub routine: Option<RoutineDetail>,
}

/// What a write names the event by. Google and Microsoft name an occurrence by its own `event`
/// id and give its `series`; CalDAV names it by the event's href and its `occurrence`.
#[derive(Serialize, utoipa::ToSchema)]
pub struct EventDetail {
    pub connection: Uuid,
    pub calendar: String,
    pub event: String,
    pub series: Option<String>,
    pub occurrence: Option<String>,
    pub etag: Option<String>,
    /// The event's own zone, which a write of a CalDAV occurrence sends back as `time_zone`.
    pub time_zone: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    /// The event in the vendor's own app.
    pub link: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct RoutineDetail {
    pub agent: Uuid,
    pub routine: Uuid,
    pub schedule: String,
    pub time_zone: String,
    /// A digest's runs that day.
    pub runs: Option<u32>,
    /// A run's turn status, or `failed`.
    pub outcome: Option<String>,
    pub turn: Option<Uuid>,
}

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct Window {
    /// RFC 3339; at most 62 days before `to`.
    pub from: String,
    pub to: String,
    /// The IANA zone days are counted in; the profile's, else UTC, when absent.
    pub time_zone: Option<String>,
}

/// A new event: RFC 3339 times, or `YYYY-MM-DD` dates (end exclusive) with `all_day`.
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewEvent {
    pub connection: Uuid,
    pub calendar: String,
    pub title: String,
    pub start: String,
    pub end: String,
    pub all_day: bool,
    /// The viewer's IANA zone; the profile's, else UTC, when absent.
    pub time_zone: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
}

/// `one` changes only the occurrence written, `all` the whole series it belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    One,
    All,
}

/// A change to an event, named as its [`EventDetail`] names it; fields absent stay as they are.
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeEvent {
    pub connection: Uuid,
    pub calendar: String,
    pub event: String,
    pub etag: Option<String>,
    pub occurrence: Option<String>,
    pub scope: Scope,
    pub series: Option<String>,
    pub title: Option<String>,
    /// Given together with `end`.
    pub start: Option<String>,
    pub end: Option<String>,
    pub all_day: Option<bool>,
    /// The event's own `time_zone` when its detail has one, else the viewer's zone.
    pub time_zone: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveEvent {
    pub connection: Uuid,
    pub calendar: String,
    pub event: String,
    pub etag: Option<String>,
    pub occurrence: Option<String>,
    pub scope: Scope,
    pub series: Option<String>,
    /// The event's own `time_zone` when its detail has one.
    pub time_zone: Option<String>,
}

#[derive(Clone)]
pub struct Calendar {
    pool: PgPool,
    /// None without a credential key: no connection can be opened.
    connections: Option<Connections>,
}

/// A calendar connection of the viewer.
struct Owned {
    id: Uuid,
    vendor: Vendor,
    name: String,
    /// The tools its level offers, without those switched off.
    offered: Vec<&'static str>,
    status: String,
}

impl Calendar {
    pub fn new(pool: PgPool, connections: Option<Connections>) -> Self {
        Self { pool, connections }
    }

    /// Everything in the range: the viewer's calendars' events and their agents' routines. A
    /// connection that fails reports its problem on its source; the rest still shows.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn range(
        &self,
        actor: &Actor,
        window: &Window,
    ) -> Result<CalendarRange, IdentityError> {
        let bounds = bounds(&window.from, &window.to)?;
        let zone = self.zone(actor, window.time_zone.as_deref()).await?;
        let (mut sources, mut items) = self.events(actor.id, bounds).await?;
        let (agents, routines) = routines::items(&self.pool, actor.id, bounds, zone).await?;
        sources.extend(agents);
        items.extend(routines);
        items.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.id.cmp(&b.id)));
        Ok(CalendarRange { sources, items })
    }

    /// The zone asked for, else the profile's, else UTC.
    async fn zone(&self, actor: &Actor, asked: Option<&str>) -> Result<Tz, IdentityError> {
        if let Some(name) = asked {
            return zone(name);
        }
        let profile = sqlx::query_scalar!(
            r#"SELECT profile->>'time_zone' AS "zone" FROM human_accounts WHERE id = $1"#,
            actor.id,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(profile
            .as_deref()
            .and_then(|name| name.parse().ok())
            .unwrap_or(Tz::UTC))
    }

    async fn events(
        &self,
        viewer: Uuid,
        bounds: (i64, i64),
    ) -> Result<(Vec<Source>, Vec<Item>), IdentityError> {
        let Some(connections) = &self.connections else {
            return Ok(Default::default());
        };
        let owned = self.owned(viewer, None).await?;
        let read = owned
            .iter()
            .filter(|owned| {
                let tools = owned.vendor.tools();
                [tools.list, tools.events]
                    .iter()
                    .all(|tool| owned.offered.contains(tool))
            })
            .map(|owned| events::read(connections, owned, bounds));
        let (mut sources, mut items) = (Vec::new(), Vec::new());
        for (found, events) in join_all(read).await {
            sources.extend(found);
            items.extend(events);
        }
        Ok((sources, items))
    }

    /// The viewer's calendar connections, or the one named.
    async fn owned(&self, viewer: Uuid, one: Option<Uuid>) -> Result<Vec<Owned>, IdentityError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, connector, slug, account, config, level, status,
                disabled_tools AS "disabled_tools!: sqlx::types::Json<Vec<String>>"
            FROM connections
            WHERE created_by = $1 AND ($2::uuid IS NULL OR id = $2)
            ORDER BY connector, slug
            "#,
            viewer,
            one,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let vendor = Vendor::of(&row.connector)?;
                let manifest = cubebot_connectors::find(&row.connector)?.manifest();
                let level = Level::parse(&row.level)?;
                let offered = manifest
                    .offered(&row.config, level, &row.disabled_tools.0)
                    .map(|tool| tool.name)
                    .collect();
                Some(Owned {
                    id: row.id,
                    vendor,
                    name: row.account.unwrap_or(row.slug),
                    offered,
                    status: row.status,
                })
            })
            .collect())
    }

    /// The viewer's connection and its connections handle when it offers `tool` at its level:
    /// 404 for someone else's connection, 403 when its level does not allow the write.
    async fn writable(
        &self,
        viewer: Uuid,
        connection: Uuid,
        tool: fn(&events::Tools) -> &'static str,
    ) -> Result<(&Connections, Vendor, &'static str), ConnectError> {
        let connections = self.connections.as_ref().ok_or(IdentityError::NotFound)?;
        let owned = self.owned(viewer, Some(connection)).await?;
        let owned = owned.first().ok_or(IdentityError::NotFound)?;
        let name = tool(owned.vendor.tools());
        if !owned.offered.contains(&name) {
            return Err(IdentityError::Forbidden.into());
        }
        Ok((connections, owned.vendor, name))
    }

    /// Creates the event on the connection's calendar and answers it as an item.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, connection = %new.connection))]
    pub async fn create_event(&self, actor: &Actor, new: &NewEvent) -> Result<Item, ConnectError> {
        times(&new.start, &new.end, Some(new.all_day))?;
        let (connections, vendor, tool) = self
            .writable(actor.id, new.connection, |tools| tools.create)
            .await?;
        let zone = self.write_zone(actor, new.time_zone.as_deref()).await?;
        let input = vendor.create_input(new, &zone)?;
        let made = connections.call_tool(new.connection, tool, input).await?;
        Ok(events::created(new, &made))
    }

    /// Changes the event, its occurrence, or with `all` the series the occurrence belongs to.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, connection = %change.connection))]
    pub async fn update_event(
        &self,
        actor: &Actor,
        change: &ChangeEvent,
    ) -> Result<(), ConnectError> {
        if let (Some(start), Some(end)) = (&change.start, &change.end) {
            times(start, end, change.all_day)?;
        } else if change.start.is_some() || change.end.is_some() {
            return Err(ConnectError::Invalid(
                "start and end are given together".into(),
            ));
        }
        let (connections, vendor, tool) = self
            .writable(actor.id, change.connection, |tools| tools.update)
            .await?;
        let zone = match vendor {
            Vendor::Dav => String::new(),
            _ => self.write_zone(actor, change.time_zone.as_deref()).await?,
        };
        let input = vendor.update_input(change, &zone)?;
        connections
            .call_tool(change.connection, tool, input)
            .await?;
        Ok(())
    }

    /// Deletes the event, its occurrence, or with `all` the whole series.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, connection = %remove.connection))]
    pub async fn delete_event(
        &self,
        actor: &Actor,
        remove: &RemoveEvent,
    ) -> Result<(), ConnectError> {
        let (connections, vendor, tool) = self
            .writable(actor.id, remove.connection, |tools| tools.delete)
            .await?;
        let input = vendor.delete_input(remove);
        connections
            .call_tool(remove.connection, tool, input)
            .await?;
        Ok(())
    }

    /// The zone a Google or Microsoft write is made in: the one given, else the viewer's.
    async fn write_zone(&self, actor: &Actor, given: Option<&str>) -> Result<String, ConnectError> {
        Ok(self.zone(actor, given).await?.name().to_owned())
    }
}

/// The range as Unix seconds: `from` before `to`, at most [`LONGEST`] apart.
fn bounds(from: &str, to: &str) -> Result<(i64, i64), IdentityError> {
    let (from, to) = (instant(from), instant(to));
    let (Some(from), Some(to)) = (from, to) else {
        return Err(IdentityError::InvalidInput);
    };
    (from < to && to - from <= LONGEST)
        .then_some((from, to))
        .ok_or(IdentityError::InvalidInput)
}

fn instant(text: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.timestamp())
}

fn zone(name: &str) -> Result<Tz, IdentityError> {
    crate::work::routines::cron::zone(name).ok_or(IdentityError::InvalidInput)
}

/// Checks a write's times: dates for an all-day event, RFC 3339 otherwise, and end after start.
fn times(start: &str, end: &str, all_day: Option<bool>) -> Result<(), ConnectError> {
    let dated = all_day.unwrap_or(start.len() == 10);
    let ordered = match dated {
        true => date(start).zip(date(end)).map(|(start, end)| start < end),
        false => instant(start)
            .zip(instant(end))
            .map(|(start, end)| start < end),
    };
    match ordered {
        Some(true) => Ok(()),
        Some(false) => Err(ConnectError::Invalid(
            "the event ends after it starts".into(),
        )),
        None if dated => Err(ConnectError::Invalid(
            "an all-day event takes dates, such as 2026-11-09".into(),
        )),
        None => Err(ConnectError::Invalid(
            "times are RFC 3339, such as 2026-11-09T09:00:00Z".into(),
        )),
    }
}

fn date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

/// Unix seconds as RFC 3339 in UTC.
fn utc(at: i64) -> String {
    DateTime::from_timestamp(at, 0)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// A vendor's start or end as an item's: a date as it is, a time in UTC.
fn shown(text: &str) -> Option<String> {
    if text.len() == 10 {
        return date(text).map(|_| text.to_owned());
    }
    instant(text)
        .or_else(|| {
            // Microsoft answers UTC without an offset, such as 2026-11-05T12:00:00.0000000.
            let naive = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f");
            naive.ok().map(|at| at.and_utc().timestamp())
        })
        .map(utc)
}

/// A text field of a vendor's answer.
fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

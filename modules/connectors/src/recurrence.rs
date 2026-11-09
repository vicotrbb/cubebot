//! Repeating events expanded from their master VEVENT (RFC 5545 §3.8.5), for CalDAV servers
//! that answer a calendar query without expanding them.
use crate::dav::{EVENTS, Line, field, stamp, vevents};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeDelta, TimeZone};
use rrule::{RRule, RRuleError, RRuleSet, Tz, Unvalidated};
use serde_json::{Value, json};
use std::str::FromStr;

/// A DTSTART, DTEND, RDATE, EXDATE or RECURRENCE-ID value.
#[derive(Clone)]
struct Moment {
    at: DateTime<Tz>,
    /// The TZID it was given in.
    zone: Option<String>,
    date: bool,
    utc: bool,
}

impl Moment {
    /// Floating times, and zones chrono-tz does not know, are taken as UTC.
    fn parse(params: &str, value: &str) -> Option<Self> {
        let zone = params
            .split(';')
            .find_map(|param| param.strip_prefix("TZID="))
            .map(|zone| zone.trim_matches('"').to_owned());
        if let Ok(date) = NaiveDate::parse_from_str(value, "%Y%m%d") {
            let at = Tz::UTC.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?);
            return Some(Self {
                at,
                zone,
                date: true,
                utc: false,
            });
        }
        let utc = value.ends_with('Z');
        let local =
            NaiveDateTime::parse_from_str(value.trim_end_matches('Z'), "%Y%m%dT%H%M%S").ok()?;
        let named = zone
            .as_deref()
            .and_then(|zone| chrono_tz::Tz::from_str(zone).ok());
        let tz = named.filter(|_| !utc).map_or(Tz::UTC, Tz::Tz);
        // A time skipped by a daylight-saving change is read as UTC rather than dropped.
        let at = tz
            .from_local_datetime(&local)
            .earliest()
            .unwrap_or_else(|| tz.from_utc_datetime(&local));
        Some(Self {
            at,
            zone,
            date: false,
            utc,
        })
    }

    fn each(line: &Line) -> impl Iterator<Item = Self> {
        line.value
            .split(',')
            .filter_map(|value| Self::parse(&line.params, value))
    }

    /// As a RECURRENCE-ID or EXDATE value: `20261110`, `20261110T090000` or `20261110T090000Z`.
    fn value(&self) -> String {
        let format = match (self.date, self.utc) {
            (true, _) => "%Y%m%d",
            (false, true) => "%Y%m%dT%H%M%SZ",
            (false, false) => "%Y%m%dT%H%M%S",
        };
        self.at.format(format).to_string()
    }

    fn shown(&self, at: DateTime<Tz>) -> String {
        if self.date {
            at.format("%Y-%m-%d").to_string()
        } else {
            stamp(at.timestamp())
        }
    }
}

/// One JSON event per occurrence overlapping `start..end` of the VEVENTs of `ics`, with the
/// descriptive fields, `start` and `end` (RFC 3339 in UTC, or `YYYY-MM-DD` when `all_day`),
/// `occurrence` and `time_zone`. `occurrence` is the occurrence's original start exactly as its
/// RECURRENCE-ID or EXDATE value carries it (`20261110T090000` in `time_zone`, `20261110T090000Z`,
/// or `20261110` all day), and `null` for an event that does not repeat. A rule that cannot be
/// read yields the master once rather than losing the event.
pub(crate) fn occurrences(ics: &str, start: i64, end: i64) -> Vec<Value> {
    let components = vevents(ics);
    let moved: Vec<(String, DateTime<Tz>)> = components
        .iter()
        .filter_map(|lines| {
            Some((
                text(lines, "UID")?.to_owned(),
                moment(lines, "RECURRENCE-ID")?.at,
            ))
        })
        .collect();
    components
        .iter()
        .flat_map(|lines| expand(lines, &moved, start, end))
        .collect()
}

fn expand(lines: &[Line], moved: &[(String, DateTime<Tz>)], start: i64, end: i64) -> Vec<Value> {
    let Some(first) = moment(lines, "DTSTART") else {
        return Vec::new();
    };
    let length = moment(lines, "DTEND").map_or(TimeDelta::days(i64::from(first.date)), |last| {
        last.at - first.at
    });
    let mut fields = json!({});
    lines.iter().for_each(|line| field(&mut fields, line));
    let event = |at, id: Option<&Moment>| {
        let mut event = fields.clone();
        event["start"] = json!(first.shown(at));
        event["end"] = json!(first.shown(at + length));
        event["all_day"] = json!(first.date);
        event["occurrence"] = json!(id.map(Moment::value));
        event["time_zone"] = json!(id.unwrap_or(&first).zone);
        event
    };
    let overlaps = |at: &DateTime<Tz>| {
        let (from, until) = (at.timestamp(), (*at + length).timestamp());
        from < end && (until > start || from == until && from >= start)
    };
    if let Some(id) = moment(lines, "RECURRENCE-ID") {
        let cancelled = text(lines, "STATUS") == Some("CANCELLED");
        let shown = !cancelled && overlaps(&first.at);
        return shown
            .then(|| event(first.at, Some(&id)))
            .into_iter()
            .collect();
    }
    if !lines
        .iter()
        .any(|line| line.name == "RRULE" || line.name == "RDATE")
    {
        return overlaps(&first.at)
            .then(|| event(first.at, None))
            .into_iter()
            .collect();
    }
    let uid = text(lines, "UID").unwrap_or_default();
    let overridden = moved.iter().filter(|(of, _)| of == uid).map(|(_, at)| *at);
    let Ok(set) = rule_set(lines, &first).map(|set| overridden.fold(set, RRuleSet::exdate)) else {
        return vec![event(first.at, None)];
    };
    let zone = first.at.timezone();
    let since = DateTime::from_timestamp(start, 0).unwrap_or_default() - length;
    let before = DateTime::from_timestamp(end, 0).unwrap_or_default();
    let dates = set
        .after(since.with_timezone(&zone))
        .before(before.with_timezone(&zone))
        .all(EVENTS as u16)
        .dates;
    dates
        .into_iter()
        .filter(overlaps)
        .map(|at| {
            event(
                at,
                Some(&Moment {
                    at,
                    ..first.clone()
                }),
            )
        })
        .collect()
}

fn rule_set(lines: &[Line], first: &Moment) -> Result<RRuleSet, RRuleError> {
    let mut set = RRuleSet::new(first.at);
    for line in lines {
        set = match line.name.as_str() {
            "RRULE" => set.rrule(
                RRule::<Unvalidated>::from_str(&utc_until(&line.value))?.validate(first.at)?,
            ),
            "RDATE" => Moment::each(line)
                .map(|moment| moment.at)
                .fold(set, RRuleSet::rdate),
            "EXDATE" => Moment::each(line)
                .map(|moment| moment.at)
                .fold(set, RRuleSet::exdate),
            _ => set,
        };
    }
    Ok(set)
}

/// The rule with a date or floating UNTIL made UTC, the only form `rrule` accepts with a
/// zoned start. shortcut: a floating UNTIL is read as UTC, off by the zone's offset for a
/// zoned start that breaks RFC 5545's rule to give it in UTC.
fn utc_until(rule: &str) -> String {
    let part = |part: &str| match part.strip_prefix("UNTIL=") {
        Some(until) if until.len() == 8 => format!("UNTIL={until}T235959Z"),
        Some(until) if !until.ends_with('Z') => format!("UNTIL={until}Z"),
        _ => part.to_owned(),
    };
    rule.split(';').map(part).collect::<Vec<_>>().join(";")
}

fn text<'a>(lines: &'a [Line], name: &str) -> Option<&'a str> {
    lines
        .iter()
        .find(|line| line.name == name)
        .map(|line| line.value.as_str())
}

fn moment(lines: &[Line], name: &str) -> Option<Moment> {
    let line = lines.iter().find(|line| line.name == name)?;
    Moment::parse(&line.params, &line.value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    fn at(time: &str) -> i64 {
        DateTime::parse_from_rfc3339(time).unwrap().timestamp()
    }

    /// The occurrences of a calendar holding `vevents` between two RFC 3339 times.
    fn expanded(vevents: &str, start: &str, end: &str) -> Vec<Value> {
        let ics = format!("BEGIN:VCALENDAR\nVERSION:2.0\n{vevents}END:VCALENDAR\n");
        occurrences(&ics, at(start), at(end))
    }

    fn starts(events: &[Value]) -> Vec<&str> {
        events
            .iter()
            .map(|event| event["start"].as_str().unwrap())
            .collect()
    }

    const NOVEMBER: (&str, &str) = ("2026-11-01T00:00:00Z", "2026-12-01T00:00:00Z");

    #[test]
    fn weekly_byday_with_count_stops_after_count() {
        let events = expanded(
            "BEGIN:VEVENT\nUID:sync\nSUMMARY:Sync\nDTSTART:20261102T090000Z\nDTEND:20261102T093000Z\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=3\nEND:VEVENT\n",
            NOVEMBER.0,
            NOVEMBER.1,
        );
        let expected = [
            "2026-11-02T09:00:00Z",
            "2026-11-04T09:00:00Z",
            "2026-11-09T09:00:00Z",
        ];
        assert_eq!(starts(&events), expected);
        assert_eq!(events[1]["end"], "2026-11-04T09:30:00Z");
        assert_eq!(events[1]["occurrence"], "20261104T090000Z");
        assert_eq!(events[1]["summary"], "Sync");
        assert_eq!(events[1]["all_day"], false);
        assert_eq!(events[1]["time_zone"], Value::Null);
    }

    #[test]
    fn until_is_inclusive_and_in_utc() {
        let events = expanded(
            "BEGIN:VEVENT\nUID:standup\nDTSTART;TZID=America/New_York:20261102T090000\nDTEND;TZID=America/New_York:20261102T091500\nRRULE:FREQ=DAILY;UNTIL=20261104T140000Z\nEND:VEVENT\n",
            NOVEMBER.0,
            NOVEMBER.1,
        );
        let expected = [
            "2026-11-02T14:00:00Z",
            "2026-11-03T14:00:00Z",
            "2026-11-04T14:00:00Z",
        ];
        assert_eq!(starts(&events), expected);
        assert_eq!(events[2]["occurrence"], "20261104T090000");
        assert_eq!(events[2]["time_zone"], "America/New_York");
    }

    #[test]
    fn exdate_removes_an_occurrence() {
        let events = expanded(
            "BEGIN:VEVENT\nUID:gym\nDTSTART:20261102T180000Z\nRRULE:FREQ=DAILY;COUNT=4\nEXDATE:20261103T180000Z,20261104T180000Z\nEND:VEVENT\n",
            NOVEMBER.0,
            NOVEMBER.1,
        );
        assert_eq!(
            starts(&events),
            ["2026-11-02T18:00:00Z", "2026-11-05T18:00:00Z"]
        );
    }

    const WEEKLY: &str = "BEGIN:VEVENT\nUID:review\nSUMMARY:Review\nDTSTART:20261102T090000Z\nDTEND:20261102T100000Z\nRRULE:FREQ=WEEKLY;COUNT=3\nEND:VEVENT\n";

    #[test]
    fn a_recurrence_id_override_moves_one_occurrence() {
        let moved = "BEGIN:VEVENT\nUID:review\nRECURRENCE-ID:20261109T090000Z\nSUMMARY:Review, moved\nDTSTART:20261110T150000Z\nDTEND:20261110T160000Z\nEND:VEVENT\n";
        let events = expanded(&format!("{WEEKLY}{moved}"), NOVEMBER.0, NOVEMBER.1);
        let mut shown = starts(&events);
        shown.sort();
        let expected = [
            "2026-11-02T09:00:00Z",
            "2026-11-10T15:00:00Z",
            "2026-11-16T09:00:00Z",
        ];
        assert_eq!(shown, expected);
        let moved: Vec<&Value> = events
            .iter()
            .filter(|event| event["occurrence"] == "20261109T090000Z")
            .collect();
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0]["summary"], "Review, moved");
        assert_eq!(moved[0]["end"], "2026-11-10T16:00:00Z");
    }

    #[test]
    fn a_cancelled_override_removes_an_occurrence() {
        let cancelled = "BEGIN:VEVENT\nUID:review\nRECURRENCE-ID:20261109T090000Z\nSTATUS:CANCELLED\nDTSTART:20261109T090000Z\nEND:VEVENT\n";
        let events = expanded(&format!("{WEEKLY}{cancelled}"), NOVEMBER.0, NOVEMBER.1);
        assert_eq!(
            starts(&events),
            ["2026-11-02T09:00:00Z", "2026-11-16T09:00:00Z"]
        );
    }

    #[test]
    fn tzid_events_keep_local_time_across_daylight_saving() {
        let events = expanded(
            "BEGIN:VEVENT\nUID:berlin\nDTSTART;TZID=Europe/Berlin:20261018T090000\nDTEND;TZID=Europe/Berlin:20261018T100000\nRRULE:FREQ=WEEKLY\nEND:VEVENT\n",
            "2026-10-15T00:00:00Z",
            "2026-10-29T00:00:00Z",
        );
        assert_eq!(
            starts(&events),
            ["2026-10-18T07:00:00Z", "2026-10-25T08:00:00Z"]
        );
        assert_eq!(events[1]["end"], "2026-10-25T09:00:00Z");
        assert_eq!(events[1]["occurrence"], "20261025T090000");
        assert_eq!(events[1]["time_zone"], "Europe/Berlin");
    }

    #[test]
    fn all_day_events_stay_dates() {
        let events = expanded(
            "BEGIN:VEVENT\nUID:rent\nDTSTART;VALUE=DATE:20260131\nDTEND;VALUE=DATE:20260201\nRRULE:FREQ=MONTHLY;BYMONTHDAY=-1;UNTIL=20260331\nEND:VEVENT\n",
            "2026-01-01T00:00:00Z",
            "2026-06-01T00:00:00Z",
        );
        assert_eq!(starts(&events), ["2026-01-31", "2026-02-28", "2026-03-31"]);
        assert_eq!(events[1]["end"], "2026-03-01");
        assert_eq!(events[1]["occurrence"], "20260228");
        assert_eq!(events[1]["all_day"], true);
    }

    #[test]
    fn occurrences_overlapping_the_window_start_are_included() {
        let events = expanded(
            "BEGIN:VEVENT\nUID:workshop\nDTSTART:20261102T090000Z\nDTEND:20261102T110000Z\nRRULE:FREQ=DAILY;COUNT=2\nEND:VEVENT\n",
            "2026-11-02T10:00:00Z",
            "2026-11-02T10:30:00Z",
        );
        assert_eq!(starts(&events), ["2026-11-02T09:00:00Z"]);
    }

    #[test]
    fn an_unparseable_rule_yields_the_master_once() {
        let events = expanded(
            "BEGIN:VEVENT\nUID:odd\nSUMMARY:Odd\nDTSTART:20261102T090000Z\nRRULE:FREQ=WEEKLY;BYDAY=XX\nEND:VEVENT\n",
            NOVEMBER.0,
            NOVEMBER.1,
        );
        assert_eq!(starts(&events), ["2026-11-02T09:00:00Z"]);
        assert_eq!(events[0]["summary"], "Odd");
        assert_eq!(events[0]["occurrence"], Value::Null);
    }
}

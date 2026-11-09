//! Repeating events expanded from their master VEVENT (RFC 5545 §3.8.5), for CalDAV servers
//! that answer a calendar query without expanding them.
mod windows_zones;

use crate::dav::{Line, field, lines, stamp, vevents};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Offset, TimeDelta, TimeZone};
use rrule::{RRule, RRuleError, RRuleSet, Tz, Unvalidated};
use serde_json::{Value, json};
use std::str::FromStr;
use windows_zones::WINDOWS_ZONES;

/// Each VTIMEZONE's TZID with the zone it was resolved to.
type Zones = Vec<(String, chrono_tz::Tz)>;

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
    /// Floating times, and zones neither known nor described by a VTIMEZONE, are taken as UTC.
    fn parse(params: &str, value: &str, zones: &Zones) -> Option<Self> {
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
        let named = zone.as_deref().and_then(|zone| resolve(zone, zones));
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

    fn each<'a>(line: &'a Line, zones: &'a Zones) -> impl Iterator<Item = Self> + 'a {
        line.value
            .split(',')
            .filter_map(|value| Self::parse(&line.params, value, zones))
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
/// At most `limit` occurrences of each event.
pub(crate) fn occurrences(ics: &str, (start, end): (i64, i64), limit: usize) -> Vec<Value> {
    let zones = zones(ics);
    let components = vevents(ics);
    let moved: Vec<(String, DateTime<Tz>)> = components
        .iter()
        .filter_map(|lines| {
            Some((
                text(lines, "UID")?.to_owned(),
                moment(lines, "RECURRENCE-ID", &zones)?.at,
            ))
        })
        .collect();
    components
        .iter()
        .flat_map(|lines| expand(lines, &zones, &moved, (start, end), limit))
        .collect()
}

fn expand(
    lines: &[Line],
    zones: &Zones,
    moved: &[(String, DateTime<Tz>)],
    (start, end): (i64, i64),
    limit: usize,
) -> Vec<Value> {
    let Some(first) = moment(lines, "DTSTART", zones) else {
        return Vec::new();
    };
    let length = span(lines, &first, zones);
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
    if let Some(id) = moment(lines, "RECURRENCE-ID", zones) {
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
    let Ok(set) = rule_set(lines, &first, zones).map(|set| overridden.fold(set, RRuleSet::exdate))
    else {
        return vec![event(first.at, None)];
    };
    let zone = first.at.timezone();
    let since = DateTime::from_timestamp(start, 0).unwrap_or_default() - length;
    let before = DateTime::from_timestamp(end, 0).unwrap_or_default();
    let dates = set
        .after(since.with_timezone(&zone))
        .before(before.with_timezone(&zone))
        .all(u16::try_from(limit).unwrap_or(u16::MAX))
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

/// How long an event lasts: to its DTEND, for its DURATION, or a day for a date without either.
fn span(lines: &[Line], first: &Moment, zones: &Zones) -> TimeDelta {
    moment(lines, "DTEND", zones)
        .map(|last| last.at - first.at)
        .or_else(|| text(lines, "DURATION").and_then(duration))
        .unwrap_or(TimeDelta::days(i64::from(first.date)))
}

/// How long the occurrences of the series in `ics` last, as a DURATION value such as `PT1800S`
/// or `P1D`.
pub(crate) fn length(ics: &str) -> Option<String> {
    let zones = zones(ics);
    let events = vevents(ics);
    let master = events
        .iter()
        .find(|lines| text(lines, "RECURRENCE-ID").is_none())?;
    let first = moment(master, "DTSTART", &zones)?;
    let length = span(master, &first, &zones);
    Some(match first.date {
        true => format!("P{}D", length.num_days()),
        false => format!("PT{}S", length.num_seconds()),
    })
}

/// `seconds` as a local time, such as `20261110T090000`, in the zone a TZID of `ics` names.
pub(crate) fn local(ics: &str, tzid: &str, seconds: i64) -> Option<String> {
    let zone = resolve(tzid, &zones(ics))?;
    let at = DateTime::from_timestamp(seconds, 0)?.with_timezone(&zone);
    Some(at.format("%Y%m%dT%H%M%S").to_string())
}

fn rule_set(lines: &[Line], first: &Moment, zones: &Zones) -> Result<RRuleSet, RRuleError> {
    let mut set = RRuleSet::new(first.at);
    // DTSTART is always an instance (RFC 5545 §3.8.5.2); `rrule` only counts it through a rule.
    if text(lines, "RRULE").is_none() {
        set = set.rdate(first.at);
    }
    for line in lines {
        set = match line.name.as_str() {
            "RRULE" => set.rrule(
                RRule::<Unvalidated>::from_str(&utc_until(&line.value))?.validate(first.at)?,
            ),
            "RDATE" => Moment::each(line, zones)
                .map(|moment| moment.at)
                .fold(set, RRuleSet::rdate),
            "EXDATE" => Moment::each(line, zones)
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

fn moment(lines: &[Line], name: &str, zones: &Zones) -> Option<Moment> {
    let line = lines.iter().find(|line| line.name == name)?;
    Moment::parse(&line.params, &line.value, zones)
}

/// A DURATION value (RFC 5545 §3.3.6), such as `PT1H30M`, `P1D` or `P1W`.
fn duration(value: &str) -> Option<TimeDelta> {
    let (sign, value) = match value.strip_prefix('-') {
        Some(value) => (-1, value),
        None => (1, value.trim_start_matches('+')),
    };
    let (mut seconds, mut digits) = (0, String::new());
    for c in value.strip_prefix('P')?.chars() {
        let unit = match c {
            '0'..='9' => {
                digits.push(c);
                continue;
            }
            'T' => continue,
            'W' => 604_800,
            'D' => 86_400,
            'H' => 3600,
            'M' => 60,
            'S' => 1,
            _ => return None,
        };
        seconds += digits.parse::<i64>().ok()? * unit;
        digits.clear();
    }
    Some(TimeDelta::seconds(sign * seconds))
}

/// Each VTIMEZONE's TZID resolved by name, then its `X-LIC-LOCATION`, then as a Windows name,
/// then by its standard offset.
fn zones(ics: &str) -> Zones {
    let mut zones = Vec::new();
    let (mut tzid, mut location, mut offset) = (None, None, None);
    let mut standard = false;
    for line in lines(ics) {
        match (line.name.as_str(), line.value.as_str()) {
            ("BEGIN", "STANDARD") => standard = true,
            ("END", "STANDARD") => standard = false,
            ("TZID", _) => tzid = Some(line.value),
            ("X-LIC-LOCATION", _) => location = Some(line.value),
            ("TZOFFSETTO", _) if standard && offset.is_none() => offset = Some(line.value),
            ("END", "VTIMEZONE") => {
                let (id, location, offset) = (tzid.take(), location.take(), offset.take());
                let Some(id) = id else { continue };
                let zone = named(&id)
                    .or_else(|| location.as_deref().and_then(named))
                    .or_else(|| windows(&id))
                    .or_else(|| offset.as_deref().and_then(fixed));
                zones.extend(zone.map(|zone| (id, zone)));
            }
            _ => (),
        }
    }
    zones
}

/// The zone a TZID names, through its VTIMEZONE when the calendar has one.
fn resolve(tzid: &str, zones: &Zones) -> Option<chrono_tz::Tz> {
    let described = zones.iter().find(|(id, _)| id == tzid);
    described
        .map(|(_, zone)| *zone)
        .or_else(|| named(tzid))
        .or_else(|| windows(tzid))
}

/// A zone by IANA or Windows name, such as `Europe/Berlin` or `W. Europe Standard Time`.
pub(crate) fn zone(name: &str) -> Option<chrono_tz::Tz> {
    named(name).or_else(|| windows(name))
}

/// An IANA name, or one with a prefix such as `/mozilla.org/20070129_1/Europe/Berlin`.
fn named(name: &str) -> Option<chrono_tz::Tz> {
    let tails = name.match_indices('/').map(|(at, _)| &name[at + 1..]);
    std::iter::once(name)
        .chain(tails)
        .find_map(|name| chrono_tz::Tz::from_str(name).ok())
}

fn windows(name: &str) -> Option<chrono_tz::Tz> {
    let (_, iana) = WINDOWS_ZONES.iter().find(|(windows, _)| *windows == name)?;
    chrono_tz::Tz::from_str(iana).ok()
}

/// A zone keeping a VTIMEZONE's offset, such as `+0530`, all year. shortcut: the VTIMEZONE's
/// daylight-saving rules are ignored; interpret them if unnamed zones with DST show up.
fn fixed(offset: &str) -> Option<chrono_tz::Tz> {
    let sign = if offset.starts_with('-') { -1 } else { 1 };
    let digits = offset.trim_start_matches(['+', '-']);
    let part = |range| {
        digits
            .get(range)
            .and_then(|part: &str| part.parse::<i32>().ok())
    };
    let seconds = sign * (part(0..2)? * 3600 + part(2..4)? * 60);
    let month = |month| NaiveDate::from_ymd_opt(2026, month, 1)?.and_hms_opt(0, 0, 0);
    let (january, july) = (month(1)?, month(7)?);
    let keeps = |zone: &&chrono_tz::Tz| {
        [january, july]
            .iter()
            .all(|at| zone.offset_from_utc_datetime(at).fix().local_minus_utc() == seconds)
    };
    let zones = chrono_tz::TZ_VARIANTS.iter().filter(keeps);
    zones
        .min_by_key(|zone| !zone.name().starts_with("Etc/"))
        .copied()
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
        occurrences(&ics, (at(start), at(end)), crate::dav::EVENTS)
    }

    fn starts(events: &[Value]) -> Vec<&str> {
        events
            .iter()
            .map(|event| event["start"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn every_windows_zone_names_a_known_iana_zone() {
        for (windows, iana) in WINDOWS_ZONES {
            assert!(chrono_tz::Tz::from_str(iana).is_ok(), "{windows} → {iana}");
        }
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
    fn a_windows_tzid_resolves_to_its_iana_zone() {
        let events = expanded(
            "BEGIN:VEVENT\nUID:outlook\nDTSTART;TZID=W. Europe Standard Time:20261018T090000\nDTEND;TZID=W. Europe Standard Time:20261018T100000\nRRULE:FREQ=WEEKLY\nEND:VEVENT\n",
            "2026-10-15T00:00:00Z",
            "2026-10-29T00:00:00Z",
        );
        assert_eq!(
            starts(&events),
            ["2026-10-18T07:00:00Z", "2026-10-25T08:00:00Z"]
        );
        assert_eq!(events[1]["end"], "2026-10-25T09:00:00Z");
        assert_eq!(events[1]["occurrence"], "20261025T090000");
        assert_eq!(events[1]["time_zone"], "W. Europe Standard Time");
    }

    #[test]
    fn a_mozilla_prefixed_tzid_resolves() {
        let events = expanded(
            "BEGIN:VEVENT\nUID:thunderbird\nDTSTART;TZID=/mozilla.org/20070129_1/Europe/Berlin:20261018T090000\nRRULE:FREQ=WEEKLY;COUNT=2\nEND:VEVENT\n",
            "2026-10-15T00:00:00Z",
            "2026-10-29T00:00:00Z",
        );
        assert_eq!(
            starts(&events),
            ["2026-10-18T07:00:00Z", "2026-10-25T08:00:00Z"]
        );
    }

    #[test]
    fn a_vtimezone_location_resolves_its_tzid() {
        let events = expanded(
            "BEGIN:VTIMEZONE\nTZID:Berlin time\nX-LIC-LOCATION:Europe/Berlin\nBEGIN:STANDARD\nTZOFFSETTO:+0100\nEND:STANDARD\nEND:VTIMEZONE\nBEGIN:VEVENT\nUID:lic\nDTSTART;TZID=Berlin time:20261018T090000\nRRULE:FREQ=WEEKLY;COUNT=2\nEND:VEVENT\n",
            "2026-10-15T00:00:00Z",
            "2026-10-29T00:00:00Z",
        );
        assert_eq!(
            starts(&events),
            ["2026-10-18T07:00:00Z", "2026-10-25T08:00:00Z"]
        );
    }

    #[test]
    fn an_unknown_tzid_uses_its_vtimezone_offset() {
        let events = expanded(
            "BEGIN:VTIMEZONE\nTZID:Office time\nBEGIN:DAYLIGHT\nTZOFFSETTO:+0630\nEND:DAYLIGHT\nBEGIN:STANDARD\nTZOFFSETTO:+0530\nEND:STANDARD\nEND:VTIMEZONE\nBEGIN:VEVENT\nUID:custom\nDTSTART;TZID=Office time:20261102T090000\nDTEND;TZID=Office time:20261102T100000\nRRULE:FREQ=DAILY;COUNT=2\nEND:VEVENT\n",
            NOVEMBER.0,
            NOVEMBER.1,
        );
        assert_eq!(
            starts(&events),
            ["2026-11-02T03:30:00Z", "2026-11-03T03:30:00Z"]
        );
        assert_eq!(events[1]["occurrence"], "20261103T090000");
        assert_eq!(events[1]["time_zone"], "Office time");
    }

    #[test]
    fn rdates_without_a_rule_keep_the_first_instance() {
        let events = expanded(
            "BEGIN:VEVENT\nUID:dates\nDTSTART:20261102T090000Z\nRDATE:20261105T090000Z,20261109T090000Z\nEXDATE:20261105T090000Z\nEND:VEVENT\n",
            NOVEMBER.0,
            NOVEMBER.1,
        );
        assert_eq!(
            starts(&events),
            ["2026-11-02T09:00:00Z", "2026-11-09T09:00:00Z"]
        );
    }

    #[test]
    fn duration_sets_the_end_when_dtend_is_absent() {
        let events = expanded(
            "BEGIN:VEVENT\nUID:timed\nDTSTART:20261102T090000Z\nDURATION:PT1H30M\nRRULE:FREQ=DAILY;COUNT=2\nEND:VEVENT\nBEGIN:VEVENT\nUID:week\nDTSTART;VALUE=DATE:20261103\nDURATION:P1W\nEND:VEVENT\nBEGIN:VEVENT\nUID:day\nDTSTART;VALUE=DATE:20261104\nEND:VEVENT\n",
            NOVEMBER.0,
            NOVEMBER.1,
        );
        let ends: Vec<&str> = events
            .iter()
            .map(|event| event["end"].as_str().unwrap())
            .collect();
        assert_eq!(
            ends,
            [
                "2026-11-02T10:30:00Z",
                "2026-11-03T10:30:00Z",
                "2026-11-10",
                "2026-11-05"
            ]
        );
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

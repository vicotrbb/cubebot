//! Five-field cron in UTC (docs/adr/agents.md#routines): minute, hour, day of month, month and day of week, each
//! `*`, a value or a range, optionally with a `/step`, in comma lists. Day of week 0 and 7 are
//! Sunday. When both day fields are restricted, a day matching either runs.
pub(crate) struct Cron {
    minutes: u64,
    hours: u64,
    days: u64,
    months: u64,
    weekdays: u64,
    any_day: bool,
    any_weekday: bool,
}

/// Leap days recur within eight years, so a schedule with no time in that span never runs.
const SCAN_DAYS: i64 = 366 * 8;

impl Cron {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let fields: Vec<&str> = text.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields[..] else {
            return None;
        };
        let weekdays = field(weekday, 0, 7)?;
        let cron = Self {
            minutes: field(minute, 0, 59)?,
            hours: field(hour, 0, 23)?,
            days: field(day, 1, 31)?,
            months: field(month, 1, 12)?,
            weekdays: (weekdays | weekdays >> 7) & 0x7f,
            any_day: day.starts_with('*'),
            any_weekday: weekday.starts_with('*'),
        };
        // Refuses dates that never come, such as 31 February.
        cron.after(0).map(|_| cron)
    }

    /// The first matching minute strictly after `seconds` since the Unix epoch, in seconds.
    pub(crate) fn after(&self, seconds: i64) -> Option<i64> {
        let start = seconds.div_euclid(60) + 1;
        let first = start.div_euclid(1440);
        (first..first + SCAN_DAYS)
            .filter(|&day| self.on(day))
            .find_map(|day| {
                let from = if day == first {
                    start - first * 1440
                } else {
                    0
                };
                (from..1440)
                    .find(|minute| bit(self.hours, minute / 60) && bit(self.minutes, minute % 60))
                    .map(|minute| (day * 1440 + minute) * 60)
            })
    }

    fn on(&self, day: i64) -> bool {
        let (month, date) = civil(day);
        // 1970-01-01 was a Thursday.
        let weekday = (day + 4).rem_euclid(7);
        let (by_date, by_weekday) = (bit(self.days, date), bit(self.weekdays, weekday));
        let by_day = match (self.any_day, self.any_weekday) {
            (false, false) => by_date || by_weekday,
            _ => by_date && by_weekday,
        };
        bit(self.months, month) && by_day
    }
}

fn bit(set: u64, value: i64) -> bool {
    set >> value & 1 == 1
}

/// One bit per allowed value.
fn field(text: &str, low: u32, high: u32) -> Option<u64> {
    let mut bits = 0;
    for part in text.split(',') {
        let (from, to, step) = range(part, low, high)?;
        for value in (from..=to).step_by(step) {
            bits |= 1 << value;
        }
    }
    Some(bits)
}

/// `*`, `a` or `a-b`, with an optional `/step`; `a/step` runs from `a` to the field's end.
fn range(part: &str, low: u32, high: u32) -> Option<(u32, u32, usize)> {
    let (span, step) = match part.split_once('/') {
        Some((span, step)) => (span, step.parse().ok().filter(|&step| step > 0)?),
        None => (part, 1),
    };
    let (from, to) = bounds(span, part.contains('/'), (low, high))?;
    (low <= from && from <= to && to <= high).then_some((from, to, step))
}

fn bounds(span: &str, stepped: bool, (low, high): (u32, u32)) -> Option<(u32, u32)> {
    Some(match span.split_once('-') {
        _ if span == "*" => (low, high),
        Some((from, to)) => (from.parse().ok()?, to.parse().ok()?),
        None if stepped => (span.parse().ok()?, high),
        None => (span.parse().ok()?, span.parse().ok()?),
    })
}

/// Month and day of a day count since 1970-01-01 (Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, i64) {
    let z = days + 719_468;
    let era_day = z.rem_euclid(146_097);
    let year_of_era = (era_day - era_day / 1460 + era_day / 36_524 - era_day / 146_096) / 365;
    let year_day = era_day - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * year_day + 2) / 153;
    let date = year_day - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    (month, date)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn next(schedule: &str, after: i64) -> Option<i64> {
        Cron::parse(schedule).unwrap().after(after)
    }

    #[test]
    fn next_times_follow_each_field() {
        // 2026-11-01 10:07:30 UTC, a Sunday, to 10:15.
        assert_eq!(next("*/15 * * * *", 1_793_527_650), Some(1_793_528_100));
        // Friday 2026-10-30 09:00 is not after itself: Monday 2026-11-02 09:00.
        assert_eq!(next("0 9 * * 1-5", 1_793_350_800), Some(1_793_610_000));
        // From 2026-03-01 the next 29 February is in 2028.
        assert_eq!(next("0 0 29 2 *", 1_772_323_200), Some(1_835_395_200));
        // Either restricted day field matches: Friday the 6th comes before the 13th.
        assert_eq!(next("0 0 13 * 5", 1_793_527_650), Some(1_793_923_200));
        assert_eq!(next("0 0 13 * *", 1_793_923_200), Some(1_794_528_000));
        // Across the end of a year, and 7 as Sunday.
        assert_eq!(next("0 0 1 1 *", 1_798_761_570), Some(1_798_761_600));
        assert_eq!(
            next("0 0 * * 7", 1_793_350_800),
            next("0 0 * * 0", 1_793_350_800)
        );
        assert_eq!(next("5,10-12 * * * *", 600), Some(660));
        assert_eq!(next("30/15 * * * *", 0), Some(1800));
    }

    #[test]
    fn malformed_or_impossible_schedules_are_refused() {
        for schedule in [
            "* * * *",
            "* * * * * *",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * * 13 *",
            "* * * * 8",
            "5-1 * * * *",
            "*/0 * * * *",
            "a * * * *",
            "0 0 31 2 *",
            "0 0 30 2 *",
        ] {
            assert!(Cron::parse(schedule).is_none(), "{schedule}");
        }
    }
}

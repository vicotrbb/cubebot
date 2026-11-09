//! Agents' routines on the calendar: upcoming runs read from each schedule in its own zone, one
//! digest for a viewer's day with more than [`HOURLY`] runs, and the scheduled runs that happened.
use super::{Item, RoutineDetail, Source, utc};
use crate::{
    identity::IdentityError,
    work::routines::cron::{self, Cron},
};
use chrono::{DateTime, NaiveDate, TimeDelta};
use chrono_tz::Tz;
use sqlx::PgPool;
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

/// A day with more runs than this shows as one digest instead.
const HOURLY: usize = 24;

/// A routine of one of the viewer's agents.
pub(super) struct Routine {
    pub agent: Uuid,
    pub id: Uuid,
    pub name: String,
    pub schedule: Option<String>,
    pub time_zone: String,
    pub enabled: bool,
}

/// A scheduled run that happened.
struct Ran {
    turn: Uuid,
    routine: Uuid,
    started: i64,
    ended: Option<i64>,
    outcome: String,
}

/// The viewer's agents as sources, and their routines' items in `from..to` counted in `viewer`'s
/// days: the upcoming runs of enabled schedules, and the scheduled runs that happened, which stay
/// when the routine is paused or loses its schedule.
pub(super) async fn items(
    pool: &PgPool,
    viewer: Uuid,
    bounds: (i64, i64),
    zone: Tz,
) -> Result<(Vec<Source>, Vec<Item>), IdentityError> {
    let agents = sqlx::query!(
        "SELECT id, name FROM agents WHERE created_by = $1 AND desired_state <> 'retired' ORDER BY name, id",
        viewer,
    )
    .fetch_all(pool)
    .await?;
    let routines = sqlx::query_as!(
        Routine,
        r#"
        SELECT r.agent_id AS agent, r.id, r.name, r.schedule, r.time_zone, r.enabled
        FROM routines r JOIN agents a ON a.id = r.agent_id
        WHERE a.created_by = $1 AND a.desired_state <> 'retired'
        ORDER BY r.id
        "#,
        viewer,
    )
    .fetch_all(pool)
    .await?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64);
    // A routine that runs every minute is ~89k steps over the longest range: off the async workers.
    let (routines, mut items) = tokio::task::spawn_blocking(move || {
        let upcoming: Vec<Item> = routines
            .iter()
            .filter(|routine| routine.enabled)
            .flat_map(|routine| upcoming(routine, bounds, now, zone))
            .collect();
        (routines, upcoming)
    })
    .await
    .unwrap_or_else(|error| std::panic::resume_unwind(error.into_panic()));
    let digested: HashSet<(Uuid, String)> = items
        .iter()
        .filter(|item| item.kind == "digest")
        .filter_map(|item| Some((item.routine.as_ref()?.routine, item.start.clone())))
        .collect();
    let ran = ran(pool, &routines, bounds).await?;
    items.extend(past(&routines, ran, &digested, zone));
    let sources = agents.into_iter().map(|agent| Source {
        id: format!("agent:{}", agent.id),
        kind: "agent",
        name: agent.name,
        account: None,
        connector: None,
        connection: None,
        color: None,
        writable: true,
        problem: None,
    });
    Ok((sources.collect(), items))
}

/// The routine's runs in `from..to` after `now`; a day of `viewer` with more than [`HOURLY`] runs,
/// past ones included, is one digest with that day's count instead.
pub(super) fn upcoming(
    routine: &Routine,
    (from, to): (i64, i64),
    now: i64,
    viewer: Tz,
) -> Vec<Item> {
    let schedule = routine.schedule.as_deref().and_then(Cron::parse);
    let Some((schedule, zone)) = schedule.zip(cron::zone(&routine.time_zone)) else {
        return Vec::new();
    };
    // Per day: how many runs, and the first ones; a day past HOURLY is only counted.
    let mut days: BTreeMap<NaiveDate, (u32, Vec<i64>)> = BTreeMap::new();
    let mut at = from - 1;
    while let Some(next) = schedule.after_in(at, zone).filter(|&next| next < to) {
        let (count, runs) = days.entry(day(next, viewer)).or_default();
        *count += 1;
        if runs.len() <= HOURLY {
            runs.push(next);
        }
        at = next;
    }
    days.into_iter()
        .flat_map(|(date, (count, runs))| match runs.len() > HOURLY {
            true => vec![digest(routine, date, count)],
            false => runs
                .into_iter()
                .filter(|&at| at > now)
                .map(|at| run(routine, at))
                .collect(),
        })
        .collect()
}

fn run(routine: &Routine, at: i64) -> Item {
    let start = utc(at);
    Item {
        kind: "routine",
        id: format!("routine:{}:{start}", routine.id),
        source: format!("agent:{}", routine.agent),
        title: routine.name.clone(),
        start,
        end: None,
        all_day: false,
        editable: true,
        event: None,
        routine: Some(detail(routine)),
    }
}

fn digest(routine: &Routine, date: NaiveDate, runs: u32) -> Item {
    let start = date.to_string();
    Item {
        kind: "digest",
        id: format!("digest:{}:{start}", routine.id),
        source: format!("agent:{}", routine.agent),
        title: routine.name.clone(),
        end: Some((date + TimeDelta::days(1)).to_string()),
        start,
        all_day: true,
        editable: true,
        event: None,
        routine: Some(RoutineDetail {
            runs: Some(runs),
            ..detail(routine)
        }),
    }
}

fn detail(routine: &Routine) -> RoutineDetail {
    RoutineDetail {
        agent: routine.agent,
        routine: routine.id,
        schedule: routine.schedule.clone().unwrap_or_default(),
        time_zone: routine.time_zone.clone(),
        runs: None,
        outcome: None,
        turn: None,
    }
}

/// The scheduled runs of `routines` that started in the range, oldest first. Runs by hand, by
/// webhook or by a connection's event are not on the calendar.
async fn ran(
    pool: &PgPool,
    routines: &[Routine],
    (from, to): (i64, i64),
) -> Result<Vec<Ran>, IdentityError> {
    let ids: Vec<Uuid> = routines.iter().map(|routine| routine.id).collect();
    Ok(sqlx::query_as!(
        Ran,
        r#"
        SELECT id AS turn, routine_id AS "routine!", extract(epoch FROM created_at)::bigint AS "started!",
            extract(epoch FROM completed_at)::bigint AS ended,
            CASE WHEN error IS NULL THEN status ELSE 'failed' END AS "outcome!"
        FROM chat_turns
        WHERE routine_id = ANY($1) AND origin = 'schedule'
            AND created_at >= to_timestamp($2) AND created_at < to_timestamp($3)
        ORDER BY created_at
        "#,
        &ids,
        from as f64,
        to as f64,
    )
    .fetch_all(pool)
    .await?)
}

/// Past runs as items: each run, or one digest for a viewer's day with more than [`HOURLY`] of a
/// routine's runs, and nothing for a day its schedule already shows as a digest.
fn past(
    routines: &[Routine],
    ran: Vec<Ran>,
    digested: &HashSet<(Uuid, String)>,
    zone: Tz,
) -> Vec<Item> {
    let mut days: BTreeMap<(Uuid, NaiveDate), Vec<Ran>> = BTreeMap::new();
    for run in ran {
        let date = day(run.started, zone);
        if !digested.contains(&(run.routine, date.to_string())) {
            days.entry((run.routine, date)).or_default().push(run);
        }
    }
    days.into_iter()
        .filter_map(|((id, date), runs)| {
            let routine = routines.iter().find(|routine| routine.id == id)?;
            Some(match runs.len() > HOURLY {
                true => vec![digest(routine, date, runs.len() as u32)],
                false => runs.into_iter().map(|run| happened(routine, run)).collect(),
            })
        })
        .flatten()
        .collect()
}

fn happened(routine: &Routine, run: Ran) -> Item {
    Item {
        kind: "run",
        id: format!("run:{}", run.turn),
        source: format!("agent:{}", routine.agent),
        title: routine.name.clone(),
        start: utc(run.started),
        end: run.ended.map(utc),
        all_day: false,
        editable: false,
        event: None,
        routine: Some(RoutineDetail {
            outcome: Some(run.outcome),
            turn: Some(run.turn),
            ..detail(routine)
        }),
    }
}

fn day(at: i64, zone: Tz) -> NaiveDate {
    DateTime::from_timestamp(at, 0)
        .unwrap_or_default()
        .with_timezone(&zone)
        .date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routine(schedule: &str, time_zone: &str) -> Routine {
        Routine {
            agent: Uuid::nil(),
            id: Uuid::nil(),
            name: "Standup".into(),
            schedule: Some(schedule.into()),
            time_zone: time_zone.into(),
            enabled: true,
        }
    }

    /// Seconds of an RFC 3339 time.
    fn at(text: &str) -> i64 {
        DateTime::parse_from_rfc3339(text).unwrap().timestamp()
    }

    fn range(from: &str, to: &str) -> (i64, i64) {
        (at(from), at(to))
    }

    #[test]
    fn upcoming_runs_are_listed_in_range() {
        let weekdays = routine("0 9 * * 1-5", "America/New_York");
        // Monday 2026-11-09 to Monday 2026-11-16, now on Tuesday morning before nine there.
        let window = range("2026-11-09T00:00:00Z", "2026-11-16T00:00:00Z");
        let items = upcoming(&weekdays, window, at("2026-11-10T13:00:00Z"), Tz::UTC);
        let starts: Vec<&str> = items.iter().map(|item| item.start.as_str()).collect();
        assert_eq!(
            starts,
            [
                "2026-11-10T14:00:00Z",
                "2026-11-11T14:00:00Z",
                "2026-11-12T14:00:00Z",
                "2026-11-13T14:00:00Z"
            ]
        );
        let first = &items[0];
        assert_eq!(
            (first.kind, first.end.as_deref(), first.all_day),
            ("routine", None, false)
        );
        assert_eq!(
            first.id,
            format!("routine:{}:2026-11-10T14:00:00Z", Uuid::nil())
        );
        let detail = first.routine.as_ref().unwrap();
        assert_eq!(
            (detail.schedule.as_str(), detail.time_zone.as_str()),
            ("0 9 * * 1-5", "America/New_York")
        );
    }

    #[test]
    fn more_than_24_runs_in_a_local_day_become_one_digest() {
        let often = routine("*/5 * * * *", "UTC");
        let window = range("2026-11-09T00:00:00Z", "2026-11-11T00:00:00Z");
        let items = upcoming(&often, window, 0, Tz::UTC);
        let digests: Vec<String> = items
            .iter()
            .map(|item| {
                let runs = item.routine.as_ref().unwrap().runs.unwrap();
                let end = item.end.as_deref().unwrap();
                format!(
                    "{} {}..{end} {runs} {}",
                    item.kind, item.start, item.all_day
                )
            })
            .collect();
        assert_eq!(
            digests,
            [
                "digest 2026-11-09..2026-11-10 288 true",
                "digest 2026-11-10..2026-11-11 288 true"
            ]
        );
    }

    #[test]
    fn past_runs_collapse_on_a_busy_day_and_hide_under_the_schedules_digest() {
        let paused = Routine {
            enabled: false,
            ..routine("*/5 * * * *", "UTC")
        };
        let monday = at("2026-11-09T00:00:00Z");
        // 30 runs on Monday, one on Tuesday, two on Wednesday.
        let starts = (0..30).map(|run| monday + run * 300).chain([
            monday + 86_400,
            monday + 2 * 86_400,
            monday + 2 * 86_400 + 60,
        ]);
        let ran = || {
            starts
                .clone()
                .map(|started| Ran {
                    turn: Uuid::new_v4(),
                    routine: Uuid::nil(),
                    started,
                    ended: None,
                    outcome: "complete".into(),
                })
                .collect()
        };
        let shown = |items: Vec<Item>| -> Vec<String> {
            let item = |item: &Item| format!("{} {}", item.kind, item.start);
            items.iter().map(item).collect()
        };
        let routines = [paused];
        let none = HashSet::new();
        assert_eq!(
            shown(past(&routines, ran(), &none, Tz::UTC)),
            [
                "digest 2026-11-09",
                "run 2026-11-10T00:00:00Z",
                "run 2026-11-11T00:00:00Z",
                "run 2026-11-11T00:01:00Z"
            ]
        );
        let digested = HashSet::from([(Uuid::nil(), "2026-11-11".to_owned())]);
        let kept = shown(past(&routines, ran(), &digested, Tz::UTC));
        assert_eq!(kept, ["digest 2026-11-09", "run 2026-11-10T00:00:00Z"]);
    }

    #[test]
    fn a_day_with_24_runs_lists_each() {
        let hourly = routine("0 * * * *", "UTC");
        let window = range("2026-11-09T00:00:00Z", "2026-11-10T00:00:00Z");
        let items = upcoming(&hourly, window, 0, Tz::UTC);
        assert_eq!(items.len(), 24);
        assert!(items.iter().all(|item| item.kind == "routine"));
        assert_eq!(items[23].start, "2026-11-09T23:00:00Z");
    }

    #[test]
    fn digest_days_follow_the_viewer_zone() {
        let often = routine("*/5 * * * *", "UTC");
        // One São Paulo day (UTC-3): 03:00 UTC to 03:00 UTC.
        let window = range("2026-11-09T03:00:00Z", "2026-11-10T03:00:00Z");
        let sao_paulo: Tz = "America/Sao_Paulo".parse().unwrap();
        let items = upcoming(&often, window, 0, sao_paulo);
        assert_eq!(items.len(), 1);
        assert_eq!(
            (
                items[0].start.as_str(),
                items[0].routine.as_ref().unwrap().runs
            ),
            ("2026-11-09", Some(288))
        );
        // Read in UTC the same window spans two partial days.
        let starts: Vec<String> = upcoming(&often, window, 0, Tz::UTC)
            .into_iter()
            .map(|item| item.start)
            .collect();
        assert_eq!(starts, ["2026-11-09", "2026-11-10"]);
    }
}

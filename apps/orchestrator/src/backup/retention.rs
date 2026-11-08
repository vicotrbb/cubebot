//! Keeps the newest backup of each of the newest days and weeks, and deletes the rest from the
//! bucket. A blob goes only once no kept backup's manifest names it.
use super::{Failure, Manifest, Run, blob_key, manifest_key};
use sqlx::PgPool;
use std::collections::{BTreeMap, HashSet};

pub(super) async fn prune(
    run: &Run,
    pool: &PgPool,
    daily: u32,
    weekly: u32,
) -> Result<(), Failure> {
    let backups = listed(run).await?;
    let verified: Vec<&str> = backups
        .iter()
        .rev()
        .filter(|(id, keys)| keys.contains(&manifest_key(id)))
        .map(|(id, _)| id.as_str())
        .collect();
    let mut keep = kept(&verified, daily, weekly);
    keep.insert(run.id.clone());
    let named = named(run, &keep).await?;
    let mut dropped = Vec::new();
    for (id, keys) in backups.iter().filter(|(id, _)| !keep.contains(*id)) {
        delete(run, id, keys).await?;
        dropped.push(id.clone());
    }
    sqlx::query!(
        "UPDATE backups SET status='deleted' WHERE id = ANY($1) AND status <> 'failed'",
        &dropped
    )
    .execute(pool)
    .await?;
    for (key, _) in run.bucket.list("blobs/").await? {
        if !named.contains(&key) {
            run.bucket.delete(&key).await?;
        }
    }
    Ok(())
}

/// Every backup in the bucket by id, with its keys.
async fn listed(run: &Run) -> Result<BTreeMap<String, Vec<String>>, Failure> {
    let mut backups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (key, _) in run.bucket.list("backups/").await? {
        if let Some((id, _)) = key
            .strip_prefix("backups/")
            .and_then(|rest| rest.split_once('/'))
        {
            backups.entry(id.to_owned()).or_default().push(key.clone());
        }
    }
    Ok(backups)
}

/// The blob keys the kept backups' manifests name.
async fn named(run: &Run, keep: &HashSet<String>) -> Result<HashSet<String>, Failure> {
    let mut named = HashSet::new();
    for id in keep {
        let bytes = run.bucket.bytes(&manifest_key(id)).await?;
        let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|error| {
            Failure::caused(format!("the manifest of {id} is unreadable"), error)
        })?;
        named.extend(manifest.blobs.iter().map(|blob| blob_key(blob.id)));
    }
    Ok(named)
}

/// The manifest first, so a backup half deleted is never taken for a whole one.
async fn delete(run: &Run, id: &str, keys: &[String]) -> Result<(), Failure> {
    let manifest = manifest_key(id);
    if keys.contains(&manifest) {
        run.bucket.delete(&manifest).await?;
    }
    for key in keys.iter().filter(|key| **key != manifest) {
        run.bucket.delete(key).await?;
    }
    Ok(())
}

/// Of verified backup ids, newest first, the newest of each of the newest `daily` days and
/// `weekly` weeks. Ids begin with their UTC date, YYYYMMDD.
fn kept(verified: &[&str], daily: u32, weekly: u32) -> HashSet<String> {
    let mut keep = HashSet::new();
    let (mut days, mut weeks) = (Vec::new(), Vec::new());
    for id in verified {
        let Some(day) = day_number(id) else { continue };
        if days.len() < daily as usize && !days.contains(&day) {
            days.push(day);
            keep.insert((*id).to_owned());
        }
        // 1970-01-01 was a Thursday, so weeks counted from three days later start on Mondays.
        let week = (day + 3).div_euclid(7);
        if weeks.len() < weekly as usize && !weeks.contains(&week) {
            weeks.push(week);
            keep.insert((*id).to_owned());
        }
    }
    keep
}

/// Days since 1970-01-01 of an id's leading YYYYMMDD (Howard Hinnant's days_from_civil).
fn day_number(id: &str) -> Option<i64> {
    let number = |range: std::ops::Range<usize>| id.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(4..6)?, number(6..8)?);
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    Some(era * 146_097 + of_cycle - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_count_from_the_epoch() {
        assert_eq!(day_number("19700101T000000Z"), Some(0));
        assert_eq!(day_number("20000301T000000Z"), Some(11_017));
        assert_eq!(day_number("20261107T030000Z"), Some(20_764));
        assert_eq!(day_number("garbage"), None);
    }

    #[test]
    fn the_newest_of_each_day_and_week_is_kept() {
        // Two a day for 40 days, newest first, ending Saturday 2026-11-07.
        let ids: Vec<String> = (0..40)
            .flat_map(|back| {
                let day = 20_764 - back;
                [
                    format!("{}T120000Z", civil(day)),
                    format!("{}T030000Z", civil(day)),
                ]
            })
            .collect();
        let verified: Vec<&str> = ids.iter().map(String::as_str).collect();
        let keep = kept(&verified, 7, 4);
        let mut kept: Vec<&str> = keep.iter().map(String::as_str).collect();
        kept.sort_unstable();
        // Seven days of noon backups, plus each of the three earlier weeks' Sunday noon one.
        assert_eq!(
            kept,
            [
                "20261018T120000Z",
                "20261025T120000Z",
                "20261101T120000Z",
                "20261102T120000Z",
                "20261103T120000Z",
                "20261104T120000Z",
                "20261105T120000Z",
                "20261106T120000Z",
                "20261107T120000Z",
            ]
        );
    }

    /// YYYYMMDD of a day number, for the test's fixtures.
    fn civil(day: i64) -> String {
        let z = day + 719_468;
        let era = z.div_euclid(146_097);
        let of_era = z - era * 146_097;
        let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
        let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_part = (5 * of_year + 2) / 153;
        let day = of_year - (153 * month_part + 2) / 5 + 1;
        let month = if month_part < 10 {
            month_part + 3
        } else {
            month_part - 9
        };
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        format!("{year:04}{month:02}{day:02}")
    }
}

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const LATEST_URL: &str = "https://api.github.com/repos/vicotrbb/cubebot/releases/latest";
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

static CHECKED: Mutex<Option<(Instant, Option<Release>)>> = Mutex::const_new(None);

#[derive(Clone, Serialize, utoipa::ToSchema)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub published_at: String,
}

#[derive(Deserialize)]
struct Published {
    tag_name: String,
    html_url: String,
    published_at: String,
}

pub async fn newer_release() -> Option<Release> {
    let mut checked = CHECKED.lock().await;
    if let Some((at, release)) = checked.as_ref()
        && at.elapsed() < CHECK_EVERY
    {
        return release.clone();
    }
    let release = fetch()
        .await
        .filter(|release| is_newer(&release.version, env!("CARGO_PKG_VERSION")));
    *checked = Some((Instant::now(), release.clone()));
    release
}

async fn fetch() -> Option<Release> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .user_agent("cubebot")
        .build()
        .ok()?;
    let response = client
        .get(LATEST_URL)
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let published: Published = response.json().await.ok()?;
    Some(Release {
        version: published.tag_name.trim_start_matches('v').to_owned(),
        url: published.html_url,
        published_at: published.published_at,
    })
}

fn is_newer(candidate: &str, current: &str) -> bool {
    let (base, pre) = current
        .split_once('-')
        .map_or((current, false), |(base, _)| (base, true));
    match (parts(candidate), parts(base)) {
        (Some(candidate), Some(base)) => candidate > base || (pre && candidate == base),
        _ => false,
    }
}

fn parts(version: &str) -> Option<Vec<u64>> {
    version.split('.').map(|part| part.parse().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn compares_numeric_release_parts() {
        assert!(is_newer("0.4.3", "0.4.2"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(!is_newer("0.4.2", "0.4.2"));
        assert!(!is_newer("0.4.1", "0.4.2"));
        assert!(!is_newer("0.5.0-rc", "0.4.2"));
        assert!(is_newer("0.5.0", "0.5.0-rc"));
        assert!(is_newer("0.5.1", "0.5.0-rc"));
        assert!(!is_newer("0.4.2", "0.5.0-rc"));
    }
}

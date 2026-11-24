mod web_push;

use crate::{
    identity::{Actor, IdentityError},
    work::{ActivityItem, Work},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;
use uuid::Uuid;
pub use web_push::Vapid;

const PUSH_SERVICES: [&str; 4] = [
    "fcm.googleapis.com",
    "push.services.mozilla.com",
    "push.apple.com",
    "notify.windows.com",
];
const PUSHED: [&str; 8] = [
    "watchdog",
    "attention",
    "approval_pending",
    "login_pending",
    "turn_waiting",
    "turn_failed",
    "recording_ready",
    "recording_failed",
];
const DEVICES: i64 = 20;
const PER_TICK: usize = 3;
const EVERY: Duration = Duration::from_secs(10);
const FALLBACK_SUBJECT: &str = "https://github.com/vicotrbb/cubebot";

#[derive(Deserialize, utoipa::ToSchema)]
pub struct PushSubscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct PushEndpoint {
    pub endpoint: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct PushSettings {
    pub public_key: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct PushTest {
    pub delivered: usize,
}

#[derive(Serialize)]
struct Notification<'a> {
    title: &'a str,
    body: &'a str,
    tag: String,
    url: &'a str,
}

struct Device {
    endpoint: String,
    p256dh: Vec<u8>,
    auth: Vec<u8>,
}

#[derive(Clone)]
pub struct Push {
    pool: PgPool,
    client: reqwest::Client,
    subject: String,
}

fn service_endpoint(endpoint: &str) -> Option<String> {
    let url = reqwest::Url::parse(endpoint).ok()?;
    let host = url.host_str()?;
    let known = PUSH_SERVICES
        .iter()
        .any(|service| host == *service || host.ends_with(&format!(".{service}")));
    (url.scheme() == "https" && url.port().is_none() && known && endpoint.len() <= 1024)
        .then(|| format!("https://{host}"))
}

fn decoded(value: &str, length: usize) -> Result<Vec<u8>, IdentityError> {
    URL_SAFE_NO_PAD
        .decode(value.trim_end_matches('='))
        .ok()
        .filter(|bytes| bytes.len() == length)
        .ok_or(IdentityError::InvalidInput)
}

fn seconds_from_now(seconds: u64) -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from((now + Duration::from_secs(seconds)).as_secs()).unwrap_or(i64::MAX)
}

impl Push {
    pub fn new(pool: PgPool) -> Self {
        let subject = std::env::var("CUBEBOT_PUBLIC_ORIGIN")
            .ok()
            .filter(|origin| origin.starts_with("https://"))
            .unwrap_or_else(|| FALLBACK_SUBJECT.into());
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            pool,
            client,
            subject,
        }
    }

    async fn vapid(&self) -> Result<Vapid, IdentityError> {
        let generated =
            Vapid::generate().map_err(|_| IdentityError::CredentialUnavailable(None))?;
        let pkcs8 = sqlx::query_scalar!(
            r#"
            WITH made AS (
                INSERT INTO push_key (pkcs8) VALUES ($1) ON CONFLICT (id) DO NOTHING RETURNING pkcs8
            )
            SELECT pkcs8 AS "pkcs8!" FROM made UNION ALL SELECT pkcs8 FROM push_key
            LIMIT 1
            "#,
            generated,
        )
        .fetch_one(&self.pool)
        .await?;
        Vapid::from_pkcs8(&pkcs8).map_err(|_| IdentityError::CredentialUnavailable(None))
    }

    pub async fn settings(&self) -> Result<PushSettings, IdentityError> {
        Ok(PushSettings {
            public_key: self.vapid().await?.public_key(),
        })
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn subscribe(
        &self,
        actor: &Actor,
        device: PushSubscription,
    ) -> Result<(), IdentityError> {
        service_endpoint(&device.endpoint).ok_or(IdentityError::InvalidInput)?;
        let p256dh = decoded(&device.p256dh, 65)?;
        let auth = decoded(&device.auth, 16)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query!(
            "
            INSERT INTO push_subscriptions (endpoint, account_id, p256dh, auth) VALUES ($1, $2, $3, $4)
            ON CONFLICT (endpoint) DO UPDATE
            SET account_id=EXCLUDED.account_id, p256dh=EXCLUDED.p256dh, auth=EXCLUDED.auth
            ",
            device.endpoint,
            actor.id,
            p256dh,
            auth,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "
            DELETE FROM push_subscriptions WHERE endpoint IN (
                SELECT endpoint FROM push_subscriptions WHERE account_id=$1
                ORDER BY created_at DESC OFFSET $2
            )
            ",
            actor.id,
            DEVICES,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "UPDATE human_accounts SET pushed_through=COALESCE(pushed_through, clock_timestamp()) WHERE id=$1",
            actor.id,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn unsubscribe(&self, actor: &Actor, endpoint: &str) -> Result<(), IdentityError> {
        sqlx::query!(
            "DELETE FROM push_subscriptions WHERE endpoint=$1 AND account_id=$2",
            endpoint,
            actor.id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn test(&self, actor: &Actor) -> Result<PushTest, IdentityError> {
        let note = Notification {
            title: "Notifications are on",
            body: "This is how Cubebot tells you an agent needs you.",
            tag: "test".into(),
            url: "/you",
        };
        Ok(PushTest {
            delivered: self.send_all(actor.id, &note).await?,
        })
    }

    async fn devices(&self, account: Uuid) -> Result<Vec<Device>, sqlx::Error> {
        sqlx::query_as!(
            Device,
            "SELECT endpoint, p256dh, auth FROM push_subscriptions WHERE account_id=$1",
            account,
        )
        .fetch_all(&self.pool)
        .await
    }

    async fn send_all(
        &self,
        account: Uuid,
        note: &Notification<'_>,
    ) -> Result<usize, IdentityError> {
        let vapid = self.vapid().await?;
        let payload = serde_json::to_vec(note).map_err(|_| IdentityError::InvalidInput)?;
        let mut delivered = 0;
        for device in self.devices(account).await? {
            if self.send(&vapid, &device, &payload).await? {
                delivered += 1;
            }
        }
        Ok(delivered)
    }

    async fn send(
        &self,
        vapid: &Vapid,
        device: &Device,
        payload: &[u8],
    ) -> Result<bool, sqlx::Error> {
        let Some(status) = self.post(vapid, device, payload).await else {
            return Ok(false);
        };
        if matches!(status.as_u16(), 404 | 410) {
            sqlx::query!(
                "DELETE FROM push_subscriptions WHERE endpoint=$1",
                device.endpoint
            )
            .execute(&self.pool)
            .await?;
        } else if !status.is_success() {
            tracing::warn!(%status, "push service refused a message");
        }
        Ok(status.is_success())
    }

    async fn post(
        &self,
        vapid: &Vapid,
        device: &Device,
        payload: &[u8],
    ) -> Option<reqwest::StatusCode> {
        let audience = service_endpoint(&device.endpoint)?;
        let sealed = web_push::encrypt(&device.p256dh, &device.auth, payload);
        let authorization =
            vapid.authorization(&audience, &self.subject, seconds_from_now(12 * 3600));
        let (Ok(body), Ok(authorization)) = (sealed, authorization) else {
            tracing::warn!("push message could not be sealed");
            return None;
        };
        self.client
            .post(&device.endpoint)
            .header("ttl", "86400")
            .header("urgency", "high")
            .header("content-encoding", "aes128gcm")
            .header("content-type", "application/octet-stream")
            .header("authorization", authorization)
            .body(body)
            .send()
            .await
            .inspect_err(|error| tracing::warn!(%error, "push service unreachable"))
            .ok()
            .map(|response| response.status())
    }

    async fn tick(&self) -> Result<(), IdentityError> {
        let accounts = sqlx::query!(
            r#"
            SELECT a.id, a.role='administrator' AS "administrator!", (to_jsonb(a.pushed_through) #>> '{}') AS through
            FROM human_accounts a
            WHERE a.active AND EXISTS(SELECT 1 FROM push_subscriptions s WHERE s.account_id=a.id)
            "#
        )
        .fetch_all(&self.pool)
        .await?;
        for account in accounts {
            let actor = Actor {
                id: account.id,
                administrator: account.administrator,
            };
            if let Err(error) = self.account(&actor, account.through.as_deref()).await {
                tracing::warn!(?error, account = %actor.id, "push notifications failed");
            }
        }
        Ok(())
    }

    async fn account(&self, actor: &Actor, through: Option<&str>) -> Result<(), IdentityError> {
        let fresh = fresh(Work::new(self.pool.clone()).activity(actor).await?, through);
        let Some(newest) = fresh.first() else {
            return Ok(());
        };
        sqlx::query!(
            "UPDATE human_accounts SET pushed_through=GREATEST(pushed_through, $2::text::timestamptz) WHERE id=$1",
            actor.id,
            newest.at,
        )
        .execute(&self.pool)
        .await?;
        if through.is_none() {
            return Ok(());
        }
        for item in fresh.iter().take(PER_TICK) {
            let title = self.title(item.agent).await?;
            let url = match (item.kind.as_str(), item.meeting) {
                ("watchdog", _) => "/watchdog".to_owned(),
                (_, Some(meeting)) => format!("/meetings/{meeting}"),
                _ => "/activity".to_owned(),
            };
            let note = Notification {
                title: &title,
                body: &item.text,
                tag: format!(
                    "{}:{}",
                    item.kind,
                    item.turn
                        .or(item.agent)
                        .or(item.meeting)
                        .unwrap_or_default()
                ),
                url: &url,
            };
            self.send_all(actor.id, &note).await?;
        }
        Ok(())
    }

    async fn title(&self, agent: Option<Uuid>) -> Result<String, sqlx::Error> {
        let Some(agent) = agent else {
            return Ok("Cubebot".into());
        };
        let name = sqlx::query_scalar!("SELECT name FROM agents WHERE id=$1", agent)
            .fetch_optional(&self.pool)
            .await?;
        Ok(name.unwrap_or_else(|| "Cubebot".into()))
    }
}

fn instant(at: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(at).ok()
}

fn fresh(items: Vec<ActivityItem>, through: Option<&str>) -> Vec<ActivityItem> {
    let through = through.and_then(instant);
    items
        .into_iter()
        .filter(|item| PUSHED.contains(&item.kind.as_str()))
        .filter(|item| match (instant(&item.at), through) {
            (Some(at), Some(through)) => at > through,
            (Some(_), None) => true,
            (None, _) => false,
        })
        .collect()
}

pub async fn notify(pool: PgPool, mut stop: watch::Receiver<bool>) {
    let push = Push::new(pool);
    loop {
        if let Err(error) = push.tick().await {
            tracing::warn!(?error, "push tick failed");
        }
        tokio::select! {
            _ = stop.changed() => return,
            () = tokio::time::sleep(EVERY) => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str, at: &str) -> ActivityItem {
        ActivityItem {
            kind: kind.into(),
            agent: None,
            room: None,
            task: None,
            turn: None,
            meeting: None,
            at: at.into(),
            text: String::new(),
            unread: true,
        }
    }

    #[test]
    fn only_new_items_that_need_the_person_are_pushed() {
        let items = vec![
            item("attention", "2026-11-19T10:00:05+00:00"),
            item("agent_published_skill", "2026-11-19T10:00:04+00:00"),
            item("approval_pending", "2026-11-19T10:00:00.5+00:00"),
            item("turn_failed", "2026-11-19T09:59:00+00:00"),
        ];
        let kinds: Vec<_> = fresh(items, Some("2026-11-19T10:00:00+00:00"))
            .into_iter()
            .map(|item| item.kind)
            .collect();
        assert_eq!(kinds, ["attention", "approval_pending"]);
    }

    #[test]
    fn only_known_push_services_are_contacted() {
        assert_eq!(
            service_endpoint("https://fcm.googleapis.com/fcm/send/abc").as_deref(),
            Some("https://fcm.googleapis.com")
        );
        assert_eq!(
            service_endpoint("https://web.push.apple.com/QGx").as_deref(),
            Some("https://web.push.apple.com")
        );
        for refused in [
            "http://fcm.googleapis.com/x",
            "https://fcm.googleapis.com:8443/x",
            "https://evil-push.apple.com.example.org/x",
            "https://10.0.0.1/x",
            "https://notpush.apple.com/x",
        ] {
            assert_eq!(service_endpoint(refused), None, "{refused}");
        }
    }
}

use super::Meetings;
use crate::identity::{Actor, IdentityError};
use axum::http::HeaderMap;
use serde::Serialize;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};
use uuid::Uuid;

const WINDOW: Duration = Duration::from_secs(60);
const LOOKUPS: u32 = 30;
const JOINS: u32 = 10;
const UNKNOWN: u32 = 200;
const NAME_CHARS: usize = 40;
const TOKEN_CHARS: usize = 43;

#[derive(Serialize, utoipa::ToSchema)]
pub struct PublicMeeting {
    pub title: String,
    pub host: String,
    pub state: String,
    pub locked: bool,
    pub lobby: bool,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct JoinedMeeting {
    pub id: Uuid,
}

#[derive(Default)]
pub struct Limiter(Mutex<Window>);

#[derive(Default)]
struct Window {
    started: Option<Instant>,
    lookups: HashMap<IpAddr, u32>,
    joins: HashMap<IpAddr, u32>,
    unknown: u32,
    refusing_until: Option<Instant>,
}

fn counted(counts: &mut HashMap<IpAddr, u32>, client: IpAddr, limit: u32) -> bool {
    let count = counts.entry(client).or_default();
    if *count >= limit {
        return false;
    }
    *count += 1;
    true
}

impl Limiter {
    fn window<T>(&self, now: Instant, use_window: impl FnOnce(&mut Window) -> T) -> T {
        let mut window = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if window
            .started
            .is_none_or(|started| now.duration_since(started) >= WINDOW)
        {
            *window = Window {
                started: Some(now),
                refusing_until: window.refusing_until,
                ..Window::default()
            };
        }
        use_window(&mut window)
    }

    pub fn lookup(&self, client: IpAddr, now: Instant) -> bool {
        self.window(now, |window| counted(&mut window.lookups, client, LOOKUPS))
    }

    pub fn join(&self, client: IpAddr, now: Instant) -> bool {
        self.window(now, |window| counted(&mut window.joins, client, JOINS))
    }

    pub fn unknown(&self, now: Instant) -> bool {
        self.window(now, |window| {
            if window.refusing_until.is_some_and(|until| now < until) {
                return false;
            }
            window.unknown += 1;
            if window.unknown > UNKNOWN {
                window.refusing_until = Some(now + WINDOW);
                return false;
            }
            true
        })
    }
}

pub fn client(peer: Option<SocketAddr>, headers: &HeaderMap, hops: usize) -> IpAddr {
    let forwarded = (hops > 0).then(|| forwarded(headers, hops)).flatten();
    forwarded
        .or(peer.map(|peer| peer.ip()))
        .unwrap_or(IpAddr::V6(Ipv6Addr::UNSPECIFIED))
}

fn forwarded(headers: &HeaderMap, hops: usize) -> Option<IpAddr> {
    let chain: Vec<&str> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .collect();
    chain.iter().rev().nth(hops - 1)?.parse().ok()
}

pub(crate) fn guest_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    let plain = name
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '\'' | '-'));
    (plain && (1..=NAME_CHARS).contains(&name.chars().count())).then(|| name.to_owned())
}

fn plausible(token: &str) -> bool {
    token.len() == TOKEN_CHARS
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

pub(super) fn trusted_hops() -> Result<usize, IdentityError> {
    match std::env::var("CUBEBOT_TRUSTED_PROXY_HOPS") {
        Ok(hops) => hops
            .parse()
            .map_err(|error: std::num::ParseIntError| IdentityError::Invalid(error.into())),
        Err(std::env::VarError::NotPresent) => Ok(0),
        Err(error) => Err(IdentityError::Invalid(error.into())),
    }
}

impl Meetings {
    pub fn limiter(&self) -> &Limiter {
        &self.limiter
    }

    pub fn client(&self, peer: Option<SocketAddr>, headers: &HeaderMap) -> IpAddr {
        client(peer, headers, self.hops)
    }

    pub fn origin(&self) -> Option<&str> {
        self.origin.as_deref()
    }

    #[tracing::instrument(skip_all)]
    pub async fn public_meeting(
        &self,
        token: &str,
    ) -> Result<Option<PublicMeeting>, IdentityError> {
        if !plausible(token) {
            return Ok(None);
        }
        Ok(sqlx::query_as!(
            PublicMeeting,
            r#"
            SELECT m.title, COALESCE(NULLIF(o.profile->>'name', ''), o.login) AS "host!",
                CASE WHEN m.ended_at IS NOT NULL THEN 'ended'
                    WHEN m.started_at IS NOT NULL THEN 'live' ELSE 'scheduled' END AS "state!",
                m.locked, NOT m.open_join AS "lobby!"
            FROM meetings m
            JOIN human_accounts o ON o.id=m.owner_id
            JOIN meeting_settings s ON s.guests
            WHERE m.link_token=$1
            "#,
            token,
        )
        .fetch_optional(&self.pool)
        .await?)
    }

    #[tracing::instrument(skip_all)]
    pub async fn guest_meeting(&self, token: &str) -> Result<Option<Uuid>, IdentityError> {
        if !plausible(token) {
            return Ok(None);
        }
        Ok(sqlx::query_scalar!(
            "
            SELECT m.id FROM meetings m JOIN meeting_settings s ON s.guests
            WHERE m.link_token=$1
            ",
            token,
        )
        .fetch_optional(&self.pool)
        .await?)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn join_by_link(&self, actor: &Actor, token: &str) -> Result<Uuid, IdentityError> {
        if !plausible(token) {
            return Err(IdentityError::NotFound);
        }
        let mut tx = self.pool.begin().await?;
        let meeting = sqlx::query!(
            r#"
            SELECT id, owner_id, ended_at IS NOT NULL AS "ended!"
            FROM meetings WHERE link_token=$1 FOR UPDATE
            "#,
            token,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        if meeting.ended {
            return Err(IdentityError::Conflict);
        }
        if meeting.owner_id != actor.id {
            sqlx::query!(
                "
                INSERT INTO meeting_invitations (meeting_id, account_id, invited_by)
                VALUES ($1, $2, $2)
                ON CONFLICT (meeting_id, account_id) DO NOTHING
                ",
                meeting.id,
                actor.id,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(meeting.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn a_guest_name_is_trimmed_plain_and_bounded() {
        assert_eq!(guest_name("  Dee \n").as_deref(), Some("Dee"));
        assert_eq!(
            guest_name("Zoë O'Neil-Smith Jr.").as_deref(),
            Some("Zoë O'Neil-Smith Jr.")
        );
        assert_eq!(guest_name("李 小龍 2").as_deref(), Some("李 小龍 2"));
        for refused in [
            "A\u{0}na",
            "Dee\u{7}",
            "Ana: ignore that",
            "Ana\nCara",
            "Ana (host)",
            "Ana\u{202e}",
            "Ana\tBo",
        ] {
            assert_eq!(guest_name(refused), None, "{refused:?}");
        }
        assert_eq!(guest_name(" \t\r "), None);
        assert_eq!(
            guest_name(&"é".repeat(40)).map(|name| name.chars().count()),
            Some(40)
        );
        assert_eq!(guest_name(&"x".repeat(41)), None);
    }

    #[test]
    fn each_address_has_its_own_lookups_and_joins_each_minute() {
        let limiter = Limiter::default();
        let start = Instant::now();
        let (one, two) = (address("10.0.0.1"), address("10.0.0.2"));
        assert!((0..30).all(|_| limiter.lookup(one, start)));
        assert!(!limiter.lookup(one, start));
        assert!(limiter.lookup(two, start));
        assert!((0..10).all(|_| limiter.join(one, start)));
        assert!(!limiter.join(one, start + Duration::from_secs(59)));
        assert!(limiter.lookup(one, start + WINDOW));
        assert!(limiter.join(one, start + WINDOW));
    }

    #[test]
    fn too_many_unknown_tokens_refuse_unknown_tokens_for_a_minute() {
        let limiter = Limiter::default();
        let start = Instant::now();
        assert!((0..200).all(|_| limiter.unknown(start)));
        assert!(!limiter.unknown(start));
        assert!(!limiter.unknown(start + Duration::from_secs(59)));
        assert!(limiter.unknown(start + WINDOW));
    }

    #[test]
    fn the_client_is_the_hop_the_trusted_proxies_saw() {
        let mut headers = HeaderMap::new();
        headers.append("x-forwarded-for", "6.6.6.6, 1.1.1.1".parse().unwrap());
        headers.append("x-forwarded-for", "2.2.2.2".parse().unwrap());
        let peer = Some(SocketAddr::from(([10, 0, 0, 9], 443)));
        assert_eq!(client(peer, &headers, 0), address("10.0.0.9"));
        assert_eq!(client(peer, &headers, 1), address("2.2.2.2"));
        assert_eq!(client(peer, &headers, 2), address("1.1.1.1"));
        assert_eq!(client(peer, &headers, 9), address("10.0.0.9"));
        assert_eq!(client(None, &HeaderMap::new(), 0), address("::"));
    }

    #[test]
    fn only_link_shaped_tokens_reach_the_database() {
        assert!(plausible(&"a".repeat(43)));
        assert!(!plausible(&"a".repeat(42)));
        assert!(!plausible(&format!("{}=", "a".repeat(42))));
    }
}

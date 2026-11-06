//! Keeping connections usable: renewing access tokens before they lapse, checking health, and
//! polling for the events that start routines. Only the orchestrator holds refresh tokens and
//! app keys (docs/adr/identity.md#orchestrator-owned-provider-authentication-refresh).
use super::{ConnectError, SPARE, Sealed, Stored, now, refusal};
use crate::{
    connections::{
        Connections,
        subscriptions::{self, Failure},
    },
    identity::IdentityError,
};
use cubebot_connectors::{Auth, Credential, Event, github};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Background renewal starts this long before expiry.
const LEAD: i64 = 10 * 60;
/// A poll's events start one routine run, bounded so its message stays within a turn's.
const EVENT_BYTES: usize = 12 * 1024;

/// Why a renewal did not happen.
pub(super) enum Renewal {
    /// The vendor refused the saved sign-in: a person must sign in again.
    Refused(String),
    /// Nothing was decided; the tokens are kept and renewal retries.
    Failing(String),
}

impl Connections {
    /// One pass of upkeep; the subscriptions' refresher runs it every half minute.
    pub async fn tend(&self) {
        for (what, result) in [
            ("renewal", self.renew_due().await),
            ("poll", self.poll_due().await),
            ("check", self.recheck_stale().await),
        ] {
            if let Err(error) = result {
                tracing::warn!(?error, what, "connection upkeep failed");
            }
        }
    }

    async fn renew_due(&self) -> Result<(), IdentityError> {
        let due = sqlx::query_scalar!(
            "
            SELECT id FROM connections
            WHERE refresh_after <= clock_timestamp() AND status <> 'sign_in_again'
            ORDER BY refresh_after LIMIT 32
            "
        )
        .fetch_all(&self.pool)
        .await?;
        for id in due {
            self.renew_within(id, LEAD).await?;
        }
        Ok(())
    }

    /// Renews an access token about to expire.
    pub(super) async fn renew(&self, id: Uuid) -> Result<(), IdentityError> {
        self.renew_within(id, SPARE).await
    }

    /// Renews the token if it expires within `ahead` seconds. The row lock serializes callers: one
    /// that waited finds the token renewed and leaves it alone.
    #[tracing::instrument(skip_all, fields(connection = %id))]
    async fn renew_within(&self, id: Uuid, ahead: i64) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        let mut stored = self.stored(&mut tx, id, true).await?;
        let fresh = stored.expires_at.is_some_and(|at| at - now() > ahead);
        if fresh && stored.sealed.access_token.is_some() {
            return Ok(());
        }
        let renewed = match stored.auth.as_str() {
            "oauth2" => self.refreshed(&stored).await,
            "github_app" => self
                .minted(&mut stored.config, &stored.sealed)
                .await
                .map(|(token, expires_at)| (token, None, expires_at - now())),
            _ => return Ok(()),
        };
        let (status, problem) = match renewed {
            Ok(renewed) => {
                self.save_renewed(&mut tx, &mut stored, renewed).await?;
                (None, None)
            }
            Err(Renewal::Refused(problem)) => (Some("sign_in_again"), Some(problem)),
            Err(Renewal::Failing(problem)) => (None, Some(problem)),
        };
        sqlx::query!(
            "
            UPDATE connections
            SET status = COALESCE($2, status), problem = $3, updated_at = clock_timestamp()
            WHERE id = $1
            ",
            id,
            status,
            problem,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn save_renewed(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        stored: &mut Stored,
        (access, refresh, expires_in): (String, Option<String>, i64),
    ) -> Result<(), IdentityError> {
        stored.sealed.access_token = Some(access);
        if let Some(refresh) = refresh {
            stored.sealed.refresh_token = Some(refresh);
        }
        let credential = self
            .seal_connection(&mut **tx, stored.id, &stored.sealed)
            .await?;
        sqlx::query!(
            "
            UPDATE connections
            SET credential = $2, config = $3,
                expires_at = clock_timestamp() + make_interval(secs => $4),
                refresh_after = clock_timestamp() + make_interval(secs => $4 - LEAST(600, $4 / 2))
            WHERE id = $1
            ",
            stored.id,
            credential,
            stored.config,
            expires_in as f64,
        )
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// The OAuth refresh-token grant; a vendor that does not rotate the token keeps the old one.
    async fn refreshed(&self, stored: &Stored) -> Result<(String, Option<String>, i64), Renewal> {
        let refused = |problem: &str| Renewal::Refused(problem.into());
        let oauth = stored
            .connector
            .manifest()
            .auth
            .iter()
            .find_map(|auth| match auth {
                Auth::OAuth2(oauth) => Some(oauth),
                _ => None,
            })
            .ok_or_else(|| refused("This connector no longer signs in with OAuth."))?;
        let refresh = stored
            .sealed
            .refresh_token
            .as_deref()
            .ok_or_else(|| refused("No refresh token was saved; sign in again."))?;
        let url = self
            .endpoint(oauth.token, &stored.config)
            .map_err(|_| refused("A setting the sign-in needs is missing."))?;
        let client_id = stored.config["client_id"].as_str().unwrap_or_default();
        let mut form = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", client_id),
        ];
        if let Some(secret) = stored.sealed.fields.get("client_secret") {
            form.push(("client_secret", secret));
        }
        let grant = subscriptions::tokens(url, &form, Some(refresh))
            .await
            .map_err(renewal)?;
        Ok((
            grant.tokens.access_token,
            Some(grant.tokens.refresh_token),
            grant.expires_in,
        ))
    }

    /// An hour-long GitHub App installation token, recording which installation it is for.
    pub(super) async fn minted(
        &self,
        config: &mut Value,
        sealed: &Sealed,
    ) -> Result<(String, i64), Renewal> {
        let app = config["app_id"].as_u64().ok_or_else(|| {
            Renewal::Refused("The GitHub App is not saved; sign in again.".into())
        })?;
        let pem = sealed.fields.get("pem").ok_or_else(|| {
            Renewal::Refused("The GitHub App's key is not saved; sign in again.".into())
        })?;
        let jwt = github::jwt(app, pem, now())
            .map_err(|_| Renewal::Refused("The GitHub App's key is not usable.".into()))?;
        let cx = self.context(config.clone(), Credential::Token { token: jwt });
        let installation = config["installation_id"].as_u64();
        match github::installation_token(&cx, installation).await {
            Ok((token, expires_at, installation)) => {
                config["installation_id"] = Value::from(installation);
                Ok((token, expires_at))
            }
            Err(cubebot_connectors::Error::Unauthorized) => Err(Renewal::Refused(
                "GitHub refused the app's key; sign in again.".into(),
            )),
            Err(error) => Err(Renewal::Failing(refusal(&error))),
        }
    }

    /// Asks the vendor to revoke an OAuth sign-in; the connection is gone here either way.
    pub(super) async fn revoke(&self, stored: &Stored) {
        let revoke = stored
            .connector
            .manifest()
            .auth
            .iter()
            .find_map(|auth| match auth {
                Auth::OAuth2(oauth) if stored.auth == "oauth2" => oauth.revoke,
                _ => None,
            });
        let (Some(revoke), Some(token)) = (revoke, stored.sealed.refresh_token.as_deref()) else {
            return;
        };
        let Ok(url) = self.endpoint(revoke, &stored.config) else {
            return;
        };
        if let Err(failure) = subscriptions::post(url, &[("token", token)]).await {
            tracing::info!(?failure, "connection revocation not confirmed");
        }
    }

    async fn recheck_stale(&self) -> Result<(), IdentityError> {
        let stale = sqlx::query_scalar!(
            "
            SELECT id FROM connections
            WHERE status <> 'sign_in_again'
                AND (checked_at IS NULL OR checked_at < clock_timestamp() - interval '6 hours')
            ORDER BY checked_at NULLS FIRST LIMIT 8
            "
        )
        .fetch_all(&self.pool)
        .await?;
        for id in stale {
            self.check_connection(id).await?;
        }
        Ok(())
    }

    /// Polls the connections some enabled routine waits on, at most every two minutes each.
    async fn poll_due(&self) -> Result<(), IdentityError> {
        sqlx::query!(
            "DELETE FROM connection_events WHERE received_at < clock_timestamp() - interval '30 days'"
        )
        .execute(&self.pool)
        .await?;
        let due = sqlx::query!(
            "
            SELECT c.id, c.connector FROM connections c
            WHERE c.status <> 'sign_in_again'
                AND (c.polled_at IS NULL OR c.polled_at < clock_timestamp() - interval '2 minutes')
                AND EXISTS(SELECT 1 FROM routines r WHERE r.trigger_connection = c.id AND r.enabled)
            ORDER BY c.polled_at NULLS FIRST LIMIT 16
            "
        )
        .fetch_all(&self.pool)
        .await?;
        for row in due {
            let reports = cubebot_connectors::find(&row.connector)
                .is_some_and(|connector| !connector.manifest().events.is_empty());
            if reports {
                self.poll(row.id).await?;
            }
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(connection = %id))]
    async fn poll(&self, id: Uuid) -> Result<(), IdentityError> {
        let polled = match self.fresh_credential(id).await {
            Ok((credential, _)) => self.polled(id, credential).await,
            Err(ConnectError::Identity(error)) => return Err(error),
            Err(refused) => Err((None, format!("{refused:?}"))),
        };
        let (cursor, events) = match polled {
            Ok(polled) => polled,
            Err((status, problem)) => {
                tracing::info!(%problem, "connection poll failed");
                sqlx::query!(
                    "UPDATE connections SET polled_at = clock_timestamp(), status = COALESCE($2, status), problem = COALESCE($3, problem) WHERE id = $1",
                    id,
                    status,
                    status.map(|_| problem),
                )
                .execute(&self.pool)
                .await?;
                return Ok(());
            }
        };
        self.record(id, cursor, events).await
    }

    /// Saves the poll's cursor and new events, and starts the routines waiting for them.
    async fn record(
        &self,
        id: Uuid,
        cursor: Option<String>,
        events: Vec<Event>,
    ) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query!(
            "UPDATE connections SET cursor = $2, polled_at = clock_timestamp() WHERE id = $1",
            id,
            cursor,
        )
        .execute(&mut *tx)
        .await?;
        let mut fresh: BTreeMap<&str, Vec<Event>> = BTreeMap::new();
        for event in events.into_iter().take(50) {
            if remembered(&mut tx, id, &event).await? {
                fresh.entry(event.kind).or_default().push(event);
            }
        }
        for (kind, events) in fresh {
            let made =
                crate::work::routines::triggered(&mut tx, id, kind, &described(&events)).await?;
            let ids: Vec<&str> = events.iter().map(|event| event.id.as_str()).collect();
            sqlx::query!(
                "UPDATE connection_events SET routines = $4 WHERE connection_id = $1 AND kind = $2 AND event_id = ANY($3)",
                id,
                kind,
                &ids as &[&str],
                made,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// The connector's poll from the saved cursor: the next cursor and what happened. A refusal
    /// carries the status it leaves the connection in, when it changes it.
    async fn polled(&self, id: Uuid, credential: Credential) -> Result<Polled, Unpolled> {
        let unpolled = |error: IdentityError| (None, error.to_string());
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| unpolled(error.into()))?;
        let stored = self.stored(&mut tx, id, false).await.map_err(unpolled)?;
        let cursor = sqlx::query_scalar!("SELECT cursor FROM connections WHERE id = $1", id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|error| unpolled(error.into()))?;
        tx.commit().await.map_err(|error| unpolled(error.into()))?;
        let cx = self.context(stored.config, credential);
        let poll = stored
            .connector
            .poll(&cx, cursor.as_deref())
            .await
            .map_err(|error| {
                let status =
                    (error == cubebot_connectors::Error::Unauthorized).then_some("sign_in_again");
                (status, refusal(&error))
            })?;
        let known = stored.connector.manifest().events;
        let events = poll
            .events
            .into_iter()
            .filter(|event| known.iter().any(|spec| spec.kind == event.kind))
            .collect();
        Ok((
            poll.cursor.filter(|cursor| cursor.len() <= 16 * 1024),
            events,
        ))
    }
}

/// A refused refresh needs a new sign-in; anything else keeps the tokens and retries.
fn renewal(failure: Failure) -> Renewal {
    tracing::warn!(?failure, "connection refresh failed");
    match failure {
        Failure::Oauth(_) => {
            Renewal::Refused("The account refused the saved sign-in; sign in again.".into())
        }
        Failure::Unavailable(_) => {
            Renewal::Failing("Renewing the sign-in failed; it is retried.".into())
        }
    }
}

type Polled = (Option<String>, Vec<Event>);
type Unpolled = (Option<&'static str>, String);

/// Records an event once; false when it was already seen.
async fn remembered(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    event: &Event,
) -> Result<bool, IdentityError> {
    let mut summary: String = event
        .summary
        .chars()
        .filter(|c| !c.is_control())
        .take(300)
        .collect();
    if summary.is_empty() {
        summary = event.kind.into();
    }
    let event_id: String = event.id.chars().take(500).collect();
    let inserted = sqlx::query!(
        "
        INSERT INTO connection_events (connection_id, kind, event_id, summary)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT DO NOTHING
        ",
        id,
        event.kind,
        event_id,
        summary,
    )
    .execute(&mut **tx)
    .await?;
    Ok(inserted.rows_affected() == 1)
}

/// The events as JSON for a routine's message, dropping the oldest until it fits.
fn described(events: &[Event]) -> String {
    let mut shown: Vec<Value> = events
        .iter()
        .map(|event| json!({"kind": event.kind, "id": event.id, "summary": event.summary, "data": event.data}))
        .collect();
    loop {
        let text = Value::from(shown.clone()).to_string();
        if text.len() <= EVENT_BYTES || shown.len() <= 1 {
            return text.chars().take(EVENT_BYTES).collect();
        }
        shown.remove(0);
    }
}

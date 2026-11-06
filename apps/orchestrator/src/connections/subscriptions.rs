//! Personal subscription connections: a person's ChatGPT or SuperGrok plan serving only their own
//! work. The orchestrator alone holds the refresh token and refreshes; agents get the access token
//! (docs/adr/identity.md#personal-subscription-connections).
use super::{Connections, administrator};
use crate::identity::{Actor, IdentityError, Source};
use cubebot_protocol::runtime::ProviderKind;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::watch;
use uuid::Uuid;

/// Who the work is for; a subscription serves only its owner.
#[derive(Clone, Copy)]
pub(crate) enum For {
    Agent(crate::agents::AgentId),
    Person(Uuid),
    /// Installation bookkeeping, which a personal subscription never serves.
    Installation,
}

/// Encrypted as one record; never logged or returned.
#[derive(Serialize, Deserialize)]
pub(super) struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
}

/// A validated token response.
pub(super) struct Grant {
    pub tokens: Tokens,
    pub expires_in: i64,
    /// Unix seconds before which the vendor refuses a refresh.
    pub earliest_refresh: Option<i64>,
    pub scope: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    expires_in: i64,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    earliest_refresh_at: Option<i64>,
}

/// An OAuth endpoint's answer other than success.
#[derive(Debug)]
pub(super) enum Failure {
    /// The vendor's OAuth error code, such as `invalid_grant` or `authorization_pending`.
    Oauth(String),
    /// Unreachable, a server fault or a malformed answer: nothing was decided.
    Unavailable(Option<Source>),
}

/// A sign-in that did not finish; the code is shown to the person.
pub enum SignInError {
    Identity(IdentityError),
    /// `sign_in_expired`, `sign_in_denied`, `sign_in_refused`, `sign_in_mismatch` or `plan_not_included`.
    Refused(&'static str),
    Unreachable(Option<Source>),
}
impl From<IdentityError> for SignInError {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error)
    }
}
impl From<sqlx::Error> for SignInError {
    fn from(error: sqlx::Error) -> Self {
        Self::Identity(error.into())
    }
}
impl From<Failure> for SignInError {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Oauth(code) => {
                tracing::warn!(%code, "subscription sign-in refused");
                Self::Refused("sign_in_refused")
            }
            Failure::Unavailable(source) => Self::Unreachable(source),
        }
    }
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct SubscriptionSummary {
    pub provider: ProviderKind,
    /// The plan's account, as its vendor names it: usually an email address.
    pub account: String,
    /// Login of the person whose plan it is.
    pub owner: String,
    pub mine: bool,
    /// Unix seconds when the access token expires; the orchestrator renews it before then.
    pub expires_at: i64,
    /// `sign_in_again` when the vendor refused the refresh, `refresh_failing` while a renewal retries.
    pub problem: Option<String>,
    /// Turns waiting because the plan reached its usage limit, while it is the provider in use.
    pub waiting: i64,
}

/// Sign-ins started but not finished, held in the single active orchestrator.
/// shortcut: an orchestrator restart forgets them and the person starts again; persist them if restarts mid-sign-in matter.
#[derive(Clone, Default)]
pub(super) struct Pending(Arc<Mutex<HashMap<Uuid, SignIn>>>);

pub(super) struct SignIn {
    pub actor: Uuid,
    pub until: Instant,
    pub flow: Flow,
}

pub(super) enum Flow {
    Chatgpt {
        verifier: String,
        state: String,
        nonce: String,
        client_id: Option<String>,
    },
    Supergrok {
        device_code: String,
        interval: Duration,
        next: Instant,
    },
    Connection(Box<super::native::SignIn>),
}

impl Pending {
    pub fn start(&self, sign_in: SignIn) -> Result<Uuid, IdentityError> {
        let mut pending = self
            .0
            .lock()
            .map_err(|_| IdentityError::CredentialUnavailable(None))?;
        let now = Instant::now();
        pending.retain(|_, sign_in| sign_in.until > now);
        if pending.len() >= 32 {
            return Err(IdentityError::Busy);
        }
        let id = Uuid::new_v4();
        pending.insert(id, sign_in);
        Ok(id)
    }

    /// Runs `step` on the actor's live sign-in; it returns whether the sign-in is over.
    pub fn with<T>(
        &self,
        id: Uuid,
        actor: Uuid,
        step: impl FnOnce(&mut SignIn) -> (bool, T),
    ) -> Result<T, SignInError> {
        let mut pending = self
            .0
            .lock()
            .map_err(|_| IdentityError::CredentialUnavailable(None))?;
        let live = pending
            .get_mut(&id)
            .filter(|sign_in| sign_in.actor == actor && sign_in.until > Instant::now())
            .ok_or(SignInError::Refused("sign_in_expired"))?;
        let (over, value) = step(live);
        if over {
            pending.remove(&id);
        }
        Ok(value)
    }

    /// The live sign-in whose flow `matches`, such as the one a callback's state names.
    pub fn find(&self, matches: impl Fn(&Flow) -> bool) -> Option<Uuid> {
        let pending = self.0.lock().ok()?;
        let now = Instant::now();
        pending
            .iter()
            .find(|(_, sign_in)| sign_in.until > now && matches(&sign_in.flow))
            .map(|(id, _)| *id)
    }

    pub fn finish(&self, id: Uuid) {
        if let Ok(mut pending) = self.0.lock() {
            pending.remove(&id);
        }
    }
}

/// The vendor's endpoint, or a controlled loopback origin in tests.
pub(super) fn endpoint(vendor: Option<&Url>, production: &str, path: &str) -> Result<Url, Failure> {
    let base = match (vendor, std::env::var("CUBEBOT_TEST_PROVIDER_ORIGIN")) {
        (Some(origin), _) => origin.clone(),
        (None, Ok(origin)) => {
            let url =
                Url::parse(&origin).map_err(|error| Failure::Unavailable(Some(error.into())))?;
            if url.host_str() != Some("127.0.0.1") {
                return Err(Failure::Unavailable(None));
            }
            url
        }
        (None, Err(_)) => {
            Url::parse(production).map_err(|error| Failure::Unavailable(Some(error.into())))?
        }
    };
    base.join(path)
        .map_err(|error| Failure::Unavailable(Some(error.into())))
}

pub(super) fn http() -> Result<reqwest::Client, Failure> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| Failure::Unavailable(Some(error.without_url().into())))
}

/// POSTs a form to an OAuth endpoint. Bodies and URLs carry secrets, so causes never keep them.
pub(super) async fn post(url: Url, form: &[(&str, &str)]) -> Result<Vec<u8>, Failure> {
    posted(http()?.post(url), form).await
}

async fn posted(
    request: reqwest::RequestBuilder,
    form: &[(&str, &str)],
) -> Result<Vec<u8>, Failure> {
    let response = request
        .header("accept", "application/json")
        .form(form)
        .send()
        .await
        .map_err(|error| Failure::Unavailable(Some(error.without_url().into())))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|error| Failure::Unavailable(Some(error.without_url().into())))?;
    if status.is_success() {
        return Ok(body.to_vec());
    }
    if status.is_client_error() && status.as_u16() != 429 {
        #[derive(Deserialize)]
        struct Refusal {
            error: String,
        }
        if let Ok(refusal) = serde_json::from_slice::<Refusal>(&body) {
            return Err(Failure::Oauth(bounded(refusal.error)));
        }
    }
    tracing::warn!(
        status = status.as_u16(),
        "subscription OAuth request failed"
    );
    Err(Failure::Unavailable(None))
}

/// Vendor error codes reach logs, so a hostile answer cannot flood them.
fn bounded(mut code: String) -> String {
    code.retain(|character| character.is_ascii_alphanumeric() || "_-.".contains(character));
    code.truncate(64);
    code
}

/// A token-endpoint request, validated; `previous` keeps a refresh token the vendor did not rotate.
pub(super) async fn tokens(
    url: Url,
    form: &[(&str, &str)],
    previous: Option<&str>,
) -> Result<Grant, Failure> {
    let body = post(url, form).await?;
    let response: TokenResponse =
        serde_json::from_slice(&body).map_err(|error| Failure::Unavailable(Some(error.into())))?;
    let refresh_token = response
        .refresh_token
        .or_else(|| previous.map(str::to_owned))
        .filter(|token| token_valid(token))
        .ok_or(Failure::Unavailable(None))?;
    if !token_valid(&response.access_token)
        || !(60..=31 * 24 * 3600).contains(&response.expires_in)
        || !response.id_token.as_deref().is_none_or(token_valid)
    {
        return Err(Failure::Unavailable(None));
    }
    Ok(Grant {
        tokens: Tokens {
            access_token: response.access_token,
            refresh_token,
            id_token: response.id_token,
        },
        expires_in: response.expires_in,
        earliest_refresh: response.earliest_refresh_at,
        scope: response.scope,
    })
}

/// The client-credentials grant (RFC 6749 §4.4): the app's id and secret in Basic authentication,
/// answering an access token and its lifetime in seconds. There is no refresh token; renewal asks again.
pub(super) async fn client_token(
    url: Url,
    form: &[(&str, &str)],
    (id, secret): (&str, &str),
) -> Result<(String, i64), Failure> {
    let body = posted(http()?.post(url).basic_auth(id, Some(secret)), form).await?;
    let response: TokenResponse =
        serde_json::from_slice(&body).map_err(|error| Failure::Unavailable(Some(error.into())))?;
    if !token_valid(&response.access_token) || !(60..=31 * 24 * 3600).contains(&response.expires_in)
    {
        return Err(Failure::Unavailable(None));
    }
    Ok((response.access_token, response.expires_in))
}

/// Tokens become HTTP headers and form fields: bounded, printable, no spaces.
fn token_valid(token: &str) -> bool {
    (1..=16 * 1024).contains(&token.len()) && token.bytes().all(|byte| byte.is_ascii_graphic())
}

/// An account name to show: printable and bounded, never a token.
pub(super) fn account(name: &str) -> Option<String> {
    let name = name.trim();
    ((1..=320).contains(&name.len()) && !name.chars().any(char::is_control))
        .then(|| name.to_owned())
}

/// The query of a pasted (or called-back) address, once it is shown to be `redirect`'s, for the
/// sign-in that sent `state`. `Identity` errors are the person's paste; `Refused` ones the vendor's.
pub(super) fn landed(
    landed: &str,
    redirect: &Url,
    state: &str,
) -> Result<HashMap<String, String>, SignInError> {
    let url = Url::parse(landed.trim()).map_err(|_| IdentityError::InvalidInput)?;
    if landed.len() > 8192
        || url.scheme() != redirect.scheme()
        || url.host_str() != redirect.host_str()
        || url.port_or_known_default() != redirect.port_or_known_default()
        || url.path() != redirect.path()
    {
        return Err(IdentityError::InvalidInput.into());
    }
    let fields: HashMap<String, String> = url.query_pairs().into_owned().collect();
    if fields.get("state").map(String::as_str) != Some(state) {
        return Err(IdentityError::InvalidInput.into());
    }
    if fields.contains_key("error") {
        return Err(SignInError::Refused("sign_in_denied"));
    }
    Ok(fields)
}

/// An authorization code from a landed address, bounded and printable.
pub(super) fn code(fields: &HashMap<String, String>) -> Result<String, SignInError> {
    fields
        .get("code")
        .filter(|code| {
            (1..=2048).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_graphic())
        })
        .cloned()
        .ok_or(SignInError::Refused("sign_in_refused"))
}

/// A device-code grant's start (RFC 8628).
#[derive(Deserialize)]
pub(super) struct Device {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    #[serde(default)]
    pub interval: Option<u64>,
}

pub(super) async fn device(url: Url, form: &[(&str, &str)]) -> Result<Device, Failure> {
    let body = post(url, form).await?;
    serde_json::from_slice(&body).map_err(|error| Failure::Unavailable(Some(error.into())))
}

impl Device {
    /// The page to show, which must be the vendor's own (`ours`) over HTTPS unless a test controls
    /// the vendor, and the polling interval.
    pub fn validated(
        &self,
        ours: impl Fn(&str) -> bool,
        controlled: bool,
    ) -> Option<(String, Duration)> {
        let page = self
            .verification_uri_complete
            .as_deref()
            .unwrap_or(&self.verification_uri);
        let url = Url::parse(page).ok()?;
        let printable = |text: &str, most: usize| {
            (1..=most).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_graphic())
        };
        ((controlled || (ours(url.host_str()?) && url.scheme() == "https"))
            && printable(&self.device_code, 1024)
            && printable(&self.user_code, 32)
            && (60..=3600).contains(&self.expires_in))
        .then(|| {
            (
                page.to_owned(),
                Duration::from_secs(self.interval.unwrap_or(5).clamp(1, 60)),
            )
        })
    }
}

/// A device-grant poll's answer short of tokens.
pub(super) enum Waiting {
    /// Not approved yet; `slower` when the vendor asked to poll less often.
    Pending {
        slower: bool,
    },
    Refused(&'static str),
}

/// Polls the token endpoint for a device grant.
pub(super) async fn device_tokens(
    url: Url,
    form: &[(&str, &str)],
) -> Result<Result<Grant, Waiting>, Failure> {
    match tokens(url, form, None).await {
        Ok(grant) => Ok(Ok(grant)),
        Err(Failure::Oauth(code)) => Ok(Err(match code.as_str() {
            "authorization_pending" => Waiting::Pending { slower: false },
            "slow_down" => Waiting::Pending { slower: true },
            "access_denied" | "authorization_denied" | "authorization_declined" => {
                Waiting::Refused("sign_in_denied")
            }
            "expired_token" | "code_expired" => Waiting::Refused("sign_in_expired"),
            _ => Waiting::Refused("sign_in_refused"),
        })),
        Err(failure) => Err(failure),
    }
}

/// Seconds before expiry when a renewal starts, unless the vendor says later.
const LEAD: i64 = 10 * 60;
/// Agents ask for a fresh token this long before expiry, so a token handed out has this long left.
const SPARE: i64 = 2 * 60;

impl Connections {
    /// The access token and its expiry, when the subscription serves `who` and has time left.
    pub(super) async fn subscription_token(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        provider: ProviderKind,
        who: For,
    ) -> Result<(String, i64), IdentityError> {
        let (person, agent) = match who {
            For::Agent(agent) => (None, Some(agent.0)),
            For::Person(person) => (Some(person), None),
            For::Installation => return Err(IdentityError::CredentialUnavailable(None)),
        };
        let row = sqlx::query!(
            r#"
            SELECT s.credential, s.revision, extract(epoch FROM s.expires_at)::bigint AS "expires_at!"
            FROM provider_subscriptions s
            JOIN human_accounts o ON o.id = s.owner AND o.active
            WHERE s.provider = $1
                AND s.problem IS DISTINCT FROM 'sign_in_again'
                AND s.expires_at > clock_timestamp() + make_interval(secs => $4)
                AND (s.owner = $2 OR EXISTS(SELECT 1 FROM agents a WHERE a.id = $3 AND a.created_by = s.owner))
            "#,
            provider.as_str(),
            person,
            agent,
            SPARE as f64,
        )
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(IdentityError::CredentialUnavailable(None))?;
        let tokens = self
            .opened(tx, provider, row.revision, &row.credential)
            .await?;
        Ok((tokens.access_token, row.expires_at))
    }

    pub(super) async fn opened(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        provider: ProviderKind,
        revision: i64,
        credential: &[u8],
    ) -> Result<Tokens, IdentityError> {
        let context = context(tx, provider, revision).await?;
        let plain = self
            .credentials
            .decrypt(credential, &context)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
        // serde_json's error would not carry the plaintext, but a parse failure never shows it either.
        serde_json::from_slice(&plain).map_err(|_| IdentityError::CredentialUnavailable(None))
    }

    async fn sealed(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        provider: ProviderKind,
        revision: i64,
        tokens: &Tokens,
    ) -> Result<Vec<u8>, IdentityError> {
        let context = context(tx, provider, revision).await?;
        let plain =
            serde_json::to_vec(tokens).map_err(|error| IdentityError::Invalid(error.into()))?;
        self.credentials
            .encrypt(&plain, &context)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))
    }

    /// Saves a finished sign-in. Someone else's plan for the same provider must be signed out first.
    pub(super) async fn signed_in(
        &self,
        actor: &Actor,
        provider: ProviderKind,
        account: &str,
        grant: Grant,
    ) -> Result<SubscriptionSummary, IdentityError> {
        let mut tx = self.pool.begin().await?;
        administrator(&mut tx, actor).await?;
        let current = sqlx::query!(
            "SELECT owner, revision FROM provider_subscriptions WHERE provider = $1 FOR UPDATE",
            provider.as_str(),
        )
        .fetch_optional(&mut *tx)
        .await?;
        if current
            .as_ref()
            .is_some_and(|current| current.owner != actor.id)
        {
            return Err(IdentityError::Conflict);
        }
        let revision = current.map_or(1, |current| current.revision + 1);
        let credential = self
            .sealed(&mut tx, provider, revision, &grant.tokens)
            .await?;
        sqlx::query!(
            "
            INSERT INTO provider_subscriptions
                (provider, owner, account, credential, revision, expires_at, refresh_after)
            VALUES ($1, $2, $3, $4, $5,
                clock_timestamp() + make_interval(secs => $6),
                GREATEST(clock_timestamp() + make_interval(secs => $6 - $7), to_timestamp($8)))
            ON CONFLICT (provider) DO UPDATE SET
                account = EXCLUDED.account,
                credential = EXCLUDED.credential,
                revision = EXCLUDED.revision,
                expires_at = EXCLUDED.expires_at,
                refresh_after = EXCLUDED.refresh_after,
                problem = NULL,
                updated_at = clock_timestamp()
            ",
            provider.as_str(),
            actor.id,
            account,
            credential,
            revision,
            grant.expires_in as f64,
            LEAD.min(grant.expires_in / 2) as f64,
            grant.earliest_refresh.unwrap_or(0) as f64,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.subscriptions(actor)
            .await?
            .into_iter()
            .find(|summary| summary.provider == provider)
            .ok_or(IdentityError::NotFound)
    }

    /// Renews the subscription if it is due. Its row lock serializes callers: one that waited
    /// re-reads the renewed row, finds it no longer due, and leaves the new tokens alone.
    #[tracing::instrument(skip_all, fields(provider = provider.as_str()))]
    pub async fn refresh_due(&self, provider: ProviderKind) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        let Some(row) = sqlx::query!(
            "
            SELECT credential, revision
            FROM provider_subscriptions
            WHERE provider = $1 AND refresh_after <= clock_timestamp()
                AND problem IS DISTINCT FROM 'sign_in_again'
            FOR UPDATE
            ",
            provider.as_str(),
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(());
        };
        let tokens = self
            .opened(&mut tx, provider, row.revision, &row.credential)
            .await?;
        let renewed = match provider {
            ProviderKind::Chatgpt => super::chatgpt::refresh(self.vendor(), &mut tx, &tokens).await,
            ProviderKind::Supergrok => super::supergrok::refresh(self.vendor(), &tokens).await,
            _ => return Err(IdentityError::InvalidInput),
        };
        match renewed {
            Ok(grant) => {
                self.renewed(&mut tx, provider, row.revision + 1, grant)
                    .await?
            }
            Err(failure) => trouble(&mut tx, provider, failure).await?,
        }
        tx.commit().await?;
        Ok(())
    }

    /// The rotated refresh token is saved with its access token, in the transaction holding the lock.
    async fn renewed(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        provider: ProviderKind,
        revision: i64,
        grant: Grant,
    ) -> Result<(), IdentityError> {
        let credential = self.sealed(tx, provider, revision, &grant.tokens).await?;
        sqlx::query!(
            "
            UPDATE provider_subscriptions SET
                credential = $2,
                revision = $3,
                expires_at = clock_timestamp() + make_interval(secs => $4),
                refresh_after = GREATEST(clock_timestamp() + make_interval(secs => $4 - $5), to_timestamp($6)),
                problem = NULL,
                updated_at = clock_timestamp()
            WHERE provider = $1
            ",
            provider.as_str(),
            credential,
            revision,
            grant.expires_in as f64,
            LEAD.min(grant.expires_in / 2) as f64,
            grant.earliest_refresh.unwrap_or(0) as f64,
        )
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn subscriptions(
        &self,
        actor: &Actor,
    ) -> Result<Vec<SubscriptionSummary>, IdentityError> {
        let mut tx = self.pool.begin().await?;
        administrator(&mut tx, actor).await?;
        let rows = sqlx::query!(
            r#"
            SELECT s.provider, s.account, a.login, s.owner = $1 AS "mine!",
                extract(epoch FROM s.expires_at)::bigint AS "expires_at!", s.problem,
                CASE WHEN EXISTS(SELECT 1 FROM provider_connection c WHERE c.provider = s.provider)
                    THEN (SELECT count(*) FROM chat_turns t WHERE t.status = 'waiting' AND t.error = 'provider_usage_limit')
                    ELSE 0 END AS "waiting!"
            FROM provider_subscriptions s
            JOIN human_accounts a ON a.id = s.owner
            ORDER BY s.provider
            "#,
            actor.id,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(SubscriptionSummary {
                    provider: ProviderKind::parse(&row.provider)
                        .ok_or(IdentityError::InvalidInput)?,
                    account: row.account,
                    owner: row.login,
                    mine: row.mine,
                    expires_at: row.expires_at,
                    problem: row.problem,
                    waiting: row.waiting,
                })
            })
            .collect()
    }

    /// Forgets the plan and stops agents using it, then asks the vendor to revoke it.
    /// Returns whether the vendor confirmed; the tokens are gone here either way.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, provider = provider.as_str()))]
    pub async fn sign_out(
        &self,
        actor: &Actor,
        provider: ProviderKind,
    ) -> Result<bool, IdentityError> {
        if !provider.subscription() {
            return Err(IdentityError::NotFound);
        }
        let mut tx = self.pool.begin().await?;
        administrator(&mut tx, actor).await?;
        let row = sqlx::query!(
            "DELETE FROM provider_subscriptions WHERE provider = $1 RETURNING credential, revision",
            provider.as_str(),
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        let tokens = self
            .opened(&mut tx, provider, row.revision, &row.credential)
            .await;
        sqlx::query!(
            "DELETE FROM provider_connection WHERE provider = $1",
            provider.as_str()
        )
        .execute(&mut *tx)
        .await?;
        let client = sqlx::query_scalar!("SELECT chatgpt_client_id FROM installation")
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        let Ok(tokens) = tokens else {
            return Ok(false);
        };
        let revoked = match provider {
            ProviderKind::Chatgpt => {
                super::chatgpt::revoke(self.vendor(), client.as_deref(), &tokens).await
            }
            _ => super::supergrok::revoke(self.vendor(), &tokens).await,
        };
        Ok(revoked.is_ok())
    }
}

/// Only the person whose plan it is may make it the provider in use, and only while it works.
pub(super) async fn owned(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    provider: ProviderKind,
) -> Result<(), IdentityError> {
    let owner = sqlx::query_scalar!(
        "SELECT owner FROM provider_subscriptions WHERE provider = $1 AND problem IS DISTINCT FROM 'sign_in_again' FOR SHARE",
        provider.as_str(),
    )
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(IdentityError::NotFound)?;
    if owner != actor.id {
        return Err(IdentityError::Forbidden);
    }
    Ok(())
}

/// A refused refresh needs a new sign-in; anything else keeps the tokens and retries.
async fn trouble(
    tx: &mut Transaction<'_, Postgres>,
    provider: ProviderKind,
    failure: Failure,
) -> Result<(), IdentityError> {
    let (problem, cause) = match failure {
        Failure::Oauth(code) => ("sign_in_again", code),
        Failure::Unavailable(source) => (
            "refresh_failing",
            source.map(|source| source.to_string()).unwrap_or_default(),
        ),
    };
    tracing::warn!(problem, %cause, "subscription refresh failed");
    sqlx::query!(
        "UPDATE provider_subscriptions SET problem = $2, updated_at = clock_timestamp() WHERE provider = $1",
        provider.as_str(),
        problem,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn context(
    tx: &mut Transaction<'_, Postgres>,
    provider: ProviderKind,
    revision: i64,
) -> Result<Vec<u8>, IdentityError> {
    let installation = sqlx::query_scalar!(r#"SELECT id AS "id!" FROM installation"#)
        .fetch_one(&mut **tx)
        .await?;
    Ok(format!(
        "cubebot:{installation}:subscription:{}:revision:{revision}",
        provider.as_str()
    )
    .into_bytes())
}

async fn refresh_subscriptions(connections: &Connections) {
    for provider in [ProviderKind::Chatgpt, ProviderKind::Supergrok] {
        if let Err(error) = connections.refresh_due(provider).await {
            tracing::warn!(
                ?error,
                provider = provider.as_str(),
                "subscription refresh failed"
            );
        }
    }
}

/// Renews due subscriptions and tends native connections every half minute, ahead of agents
/// needing them.
pub async fn keep_fresh(connections: Connections, mut stop: watch::Receiver<bool>) {
    loop {
        refresh_subscriptions(&connections).await;
        connections.tend().await;
        tokio::select! {
            _ = stop.changed() => return,
            () = tokio::time::sleep(Duration::from_secs(30)) => (),
        }
    }
}

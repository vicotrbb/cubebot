//! SuperGrok and X Premium through device-code sign-in at `auth.x.ai`, with the Grok CLI's own
//! client id as xAI's endorsed partners use it. Opt-in, and removable with this file
//! (docs/adr/identity.md#personal-subscription-connections).
use super::{
    Connections, administrator,
    subscriptions::{Failure, Flow, Grant, SignIn, SignInError, SubscriptionSummary, Tokens},
};
use crate::identity::Actor;
use cubebot_protocol::runtime::ProviderKind;
use openidconnect::{
    Nonce,
    core::{CoreIdToken, CoreIdTokenVerifier},
};
use reqwest::Url;
use serde::Deserialize;
use std::time::{Duration, Instant};
use uuid::Uuid;

const ISSUER: &str = "https://auth.x.ai/";
const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct SupergrokSignIn {
    pub sign_in: Uuid,
    pub user_code: String,
    /// Opens xAI's approval page with the code filled in.
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Deserialize)]
struct Device {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    expires_in: u64,
    #[serde(default)]
    interval: Option<u64>,
}

impl Connections {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn start_supergrok(&self, actor: &Actor) -> Result<SupergrokSignIn, SignInError> {
        let mut tx = self.pool.begin().await?;
        administrator(&mut tx, actor).await?;
        tx.commit().await?;
        let url = super::subscriptions::endpoint(self.vendor(), ISSUER, "oauth2/device/code")?;
        let body =
            super::subscriptions::post(url, &[("client_id", CLIENT_ID), ("scope", SCOPE)]).await?;
        let device: Device = serde_json::from_slice(&body)
            .map_err(|error| Failure::Unavailable(Some(error.into())))?;
        let controlled =
            self.vendor().is_some() || std::env::var_os("CUBEBOT_TEST_PROVIDER_ORIGIN").is_some();
        let (page, interval) = validated(&device, controlled).ok_or(Failure::Unavailable(None))?;
        let sign_in = self.sign_ins.start(SignIn {
            actor: actor.id,
            until: Instant::now() + Duration::from_secs(device.expires_in),
            flow: Flow::Supergrok {
                device_code: device.device_code,
                interval,
                next: Instant::now() + interval,
            },
        })?;
        Ok(SupergrokSignIn {
            sign_in,
            user_code: device.user_code,
            verification_uri: page,
            expires_in: device.expires_in,
            interval: interval.as_secs(),
        })
    }

    /// `None` while the person has not approved yet. Asking sooner than xAI allows asks nothing.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, sign_in = %sign_in))]
    pub async fn poll_supergrok(
        &self,
        actor: &Actor,
        sign_in: Uuid,
    ) -> Result<Option<SubscriptionSummary>, SignInError> {
        let due = self
            .sign_ins
            .with(sign_in, actor.id, |pending| match &mut pending.flow {
                Flow::Supergrok {
                    device_code,
                    interval,
                    next,
                } if *next <= Instant::now() => {
                    *next = Instant::now() + *interval;
                    (false, Some(device_code.clone()))
                }
                _ => (false, None),
            })?;
        let Some(device_code) = due else {
            return Ok(None);
        };
        let url = super::subscriptions::endpoint(self.vendor(), ISSUER, "oauth2/token")?;
        let form = [
            ("grant_type", DEVICE_GRANT),
            ("client_id", CLIENT_ID),
            ("device_code", &device_code),
        ];
        let grant = match super::subscriptions::tokens(url, &form, None).await {
            Ok(grant) => grant,
            Err(Failure::Oauth(code)) => return self.waiting(actor, sign_in, &code),
            Err(failure) => return Err(failure.into()),
        };
        self.sign_ins.finish(sign_in);
        let account = account(&grant).ok_or(SignInError::Refused("sign_in_refused"))?;
        Ok(Some(
            self.signed_in(actor, ProviderKind::Supergrok, &account, grant)
                .await?,
        ))
    }

    fn waiting(
        &self,
        actor: &Actor,
        sign_in: Uuid,
        code: &str,
    ) -> Result<Option<SubscriptionSummary>, SignInError> {
        let refused = match code {
            "authorization_pending" => return Ok(None),
            "slow_down" => {
                self.sign_ins.with(sign_in, actor.id, |pending| {
                    if let Flow::Supergrok { interval, .. } = &mut pending.flow {
                        *interval += Duration::from_secs(5);
                    }
                    (false, ())
                })?;
                return Ok(None);
            }
            "access_denied" | "authorization_denied" => "sign_in_denied",
            "expired_token" => "sign_in_expired",
            _ => "sign_in_refused",
        };
        self.sign_ins.finish(sign_in);
        Err(SignInError::Refused(refused))
    }
}

/// The page shown to the person must be xAI's own, over HTTPS, unless a test controls the vendor.
fn validated(device: &Device, controlled: bool) -> Option<(String, Duration)> {
    let page = device
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&device.verification_uri);
    let url = Url::parse(page).ok()?;
    let host = url.host_str()?;
    let ours = host == "x.ai" || host.ends_with(".x.ai") || controlled;
    let printable = |text: &str, most: usize| {
        (1..=most).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_graphic())
    };
    (ours
        && (url.scheme() == "https" || controlled)
        && printable(&device.device_code, 1024)
        && printable(&device.user_code, 32)
        && (60..=3600).contains(&device.expires_in))
    .then(|| {
        (
            page.to_owned(),
            Duration::from_secs(device.interval.unwrap_or(5).clamp(1, 60)),
        )
    })
}

/// The ID token came straight from xAI's token endpoint over TLS, which OpenID Connect accepts in
/// place of its signature; it only names the account, so issuer and audience are what matter.
fn account(grant: &Grant) -> Option<String> {
    let token: CoreIdToken = grant.tokens.id_token.as_deref()?.parse().ok()?;
    let unchecked = CoreIdTokenVerifier::new_insecure_without_verification();
    let claims = token.claims(&unchecked, |_: Option<&Nonce>| Ok(())).ok()?;
    let ours = claims.issuer().as_str().trim_end_matches('/') == ISSUER.trim_end_matches('/')
        && claims
            .audiences()
            .iter()
            .any(|audience| audience.as_str() == CLIENT_ID);
    let name = claims
        .email()
        .map(|email| email.as_str())
        .unwrap_or(claims.subject().as_str());
    ours.then(|| super::subscriptions::account(name))?
}

pub(super) async fn refresh(vendor: Option<&Url>, tokens: &Tokens) -> Result<Grant, Failure> {
    let url = super::subscriptions::endpoint(vendor, ISSUER, "oauth2/token")?;
    super::subscriptions::tokens(
        url,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", &tokens.refresh_token),
        ],
        Some(&tokens.refresh_token),
    )
    .await
}

pub(super) async fn revoke(vendor: Option<&Url>, tokens: &Tokens) -> Result<(), Failure> {
    let url = super::subscriptions::endpoint(vendor, ISSUER, "oauth2/revoke")?;
    super::subscriptions::post(
        url,
        &[
            ("token", &tokens.refresh_token),
            ("token_type_hint", "refresh_token"),
            ("client_id", CLIENT_ID),
        ],
    )
    .await
    .map(drop)
}

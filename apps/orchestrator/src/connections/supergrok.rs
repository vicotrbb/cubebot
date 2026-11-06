//! SuperGrok and X Premium through device-code sign-in at `auth.x.ai`, with the Grok CLI's own
//! client id as xAI's endorsed partners use it. Opt-in, and removable with this file
//! (docs/adr/identity.md#personal-subscription-connections).
use super::{
    Connections, administrator,
    subscriptions::{
        Failure, Flow, Grant, SignIn, SignInError, SubscriptionSummary, Tokens, Waiting,
    },
};
use crate::identity::Actor;
use cubebot_protocol::runtime::ProviderKind;
use openidconnect::{
    Nonce,
    core::{CoreIdToken, CoreIdTokenVerifier},
};
use reqwest::Url;
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

impl Connections {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn start_supergrok(&self, actor: &Actor) -> Result<SupergrokSignIn, SignInError> {
        let mut tx = self.pool.begin().await?;
        administrator(&mut tx, actor).await?;
        tx.commit().await?;
        let url = super::subscriptions::endpoint(self.vendor(), ISSUER, "oauth2/device/code")?;
        let device =
            super::subscriptions::device(url, &[("client_id", CLIENT_ID), ("scope", SCOPE)])
                .await?;
        let ours = |host: &str| host == "x.ai" || host.ends_with(".x.ai");
        let (page, interval) = device
            .validated(ours, self.controlled())
            .ok_or(Failure::Unavailable(None))?;
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
        let grant = match super::subscriptions::device_tokens(url, &form).await? {
            Ok(grant) => grant,
            Err(waiting) => return self.waiting(actor, sign_in, waiting),
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
        waiting: Waiting,
    ) -> Result<Option<SubscriptionSummary>, SignInError> {
        match waiting {
            Waiting::Pending { slower } => {
                self.sign_ins.with(sign_in, actor.id, |pending| {
                    if let (true, Flow::Supergrok { interval, .. }) = (slower, &mut pending.flow) {
                        *interval += Duration::from_secs(5);
                    }
                    (false, ())
                })?;
                Ok(None)
            }
            Waiting::Refused(code) => {
                self.sign_ins.finish(sign_in);
                Err(SignInError::Refused(code))
            }
        }
    }
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

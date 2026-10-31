//! Optional installation OIDC. Provider credentials never become agent credentials.

mod bindings;
mod config;
mod http;
mod provider;
mod storage;
pub use config::Config;
pub use config::environment;

use super::{Identity, IdentityError, Session, passwords};
use openidconnect::{
    CsrfToken, Nonce, PkceCodeChallenge, PkceCodeVerifier, Scope, core::CoreAuthenticationFlow,
};

static NETWORK_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

#[derive(Clone)]
pub struct Oidc {
    pub(crate) identity: Identity,
    config: Config,
    secret: Option<String>,
    http: http::Http,
    revision: i64,
}

pub struct Started {
    pub authorization_url: String,
    pub state: String,
    pub browser_secret: String,
}

impl Oidc {
    pub async fn configure(
        mut identity: Identity,
        config: Config,
        secret: Option<String>,
    ) -> Result<Self, IdentityError> {
        let config = Config::parse(
            &serde_json::to_string(&config)
                .map_err(|error| IdentityError::Invalid(error.into()))?,
        )?;
        identity.browser_origin = Some(config.origin()?);
        let revision = storage::configure(&identity.pool, &config).await?;
        let http = http::Http::new(config.issuer.starts_with("http://"))?;
        Ok(Self {
            identity,
            config,
            secret,
            http,
            revision,
        })
    }

    pub async fn start(&self, link_session: Option<&str>) -> Result<Started, IdentityError> {
        let _permit = NETWORK_SLOTS
            .try_acquire()
            .map_err(|_| IdentityError::Busy)?;
        if let Some(session) = link_session {
            self.identity.authenticate(session).await?;
        }
        let client = provider::client(&self.config, self.secret.as_deref(), &self.http).await?;
        let browser_secret = passwords::token();
        let challenge = PkceCodeChallenge::from_code_verifier_sha256(&PkceCodeVerifier::new(
            browser_secret.clone(),
        ));
        let (url, state, nonce) = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .add_scope(Scope::new("email".into()))
            .add_scope(Scope::new("profile".into()))
            .set_pkce_challenge(challenge)
            .url();
        storage::store_flow(
            &self.identity.pool,
            self.revision,
            state.secret(),
            &browser_secret,
            nonce.secret(),
            link_session,
        )
        .await?;
        Ok(Started {
            authorization_url: url.to_string(),
            state: state.secret().to_owned(),
            browser_secret,
        })
    }

    pub async fn finish(
        &self,
        state: &str,
        browser: String,
        code: String,
    ) -> Result<Session, IdentityError> {
        let _permit = NETWORK_SLOTS
            .try_acquire()
            .map_err(|_| IdentityError::Busy)?;
        if state.len() > 256 || browser.len() != 64 || code.len() > 8192 {
            return Err(IdentityError::Unauthorized);
        }
        let flow = storage::claim_flow(&self.identity.pool, self.revision, state, &browser).await?;
        let client = provider::client(&self.config, self.secret.as_deref(), &self.http).await?;
        let verified = provider::verify(
            &client,
            &self.http,
            code,
            browser,
            flow.nonce.clone(),
            !self.config.admit_email_domains.is_empty(),
        )
        .await?;
        storage::admit(&self.identity.pool, &self.config, flow, verified).await
    }
}

#[tracing::instrument(skip_all)]
pub async fn disable(pool: &sqlx::PgPool) -> Result<(), IdentityError> {
    sqlx::query!(
        "
        UPDATE identity_oidc_policy
        SET enabled = false, revision = revision + 1
        WHERE singleton AND enabled
        ",
    )
    .execute(pool)
    .await?;
    Ok(())
}

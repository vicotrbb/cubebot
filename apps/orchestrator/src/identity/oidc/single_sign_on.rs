//! The installation's single sign-on: set by `CUBEBOT_OIDC_CONFIG`, which then owns it, or by an
//! administrator in the console, whose change applies at once.
use super::{Config, Oidc, http::Http, provider};
use crate::{
    connections::Credentials,
    identity::{Identity, IdentityError},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Authenticated-encryption context, so the secret cannot be swapped with another record.
const CONTEXT: &[u8] = b"cubebot:oidc-client-secret";
const CALLBACK: &str = "/api/identity/oidc/callback";

#[derive(Clone)]
pub struct SingleSignOn {
    current: Arc<RwLock<Option<Oidc>>>,
    identity: Identity,
    credentials: Option<Credentials>,
    deployment: Option<(Config, bool)>,
}

/// What an administrator sees; never the client secret itself.
#[derive(Serialize, utoipa::ToSchema)]
pub struct SingleSignOnSettings {
    /// Set by the deployment's `CUBEBOT_OIDC_CONFIG`; the console cannot change it.
    pub managed: bool,
    pub enabled: bool,
    pub issuer: Option<String>,
    pub client_id: Option<String>,
    pub has_client_secret: bool,
    pub admit_subjects: Vec<String>,
    pub admit_email_domains: Vec<String>,
    /// The callback to register with the identity provider; absent without a public origin.
    pub redirect_uri: Option<String>,
    /// Whether a client secret can be stored, which needs the installation's credential key.
    pub can_store_secret: bool,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveSingleSignOn {
    pub issuer: String,
    pub client_id: String,
    /// Absent keeps the stored secret; empty removes it, for a public client.
    pub client_secret: Option<String>,
    #[serde(default)]
    pub admit_subjects: Vec<String>,
    #[serde(default)]
    pub admit_email_domains: Vec<String>,
}

impl SingleSignOn {
    /// `deployment` comes from `CUBEBOT_OIDC_CONFIG`; without it, the console's stored provider
    /// applies, and without that, OIDC sessions end as before.
    pub async fn start(
        identity: Identity,
        deployment: Option<(Config, Option<String>)>,
        credentials: Option<Credentials>,
    ) -> Result<Self, IdentityError> {
        let mut sso = Self {
            current: Arc::new(RwLock::new(None)),
            identity: identity.clone(),
            credentials,
            deployment: None,
        };
        let configured = match deployment {
            Some((config, secret)) => {
                sso.deployment = Some((config.clone(), secret.is_some()));
                Some((config, secret))
            }
            None => sso.stored().await?,
        };
        match configured {
            Some((config, secret)) => {
                let oidc = Oidc::configure(identity, config, secret).await?;
                sso.identity = oidc.identity.clone();
                *sso.current.write().await = Some(oidc);
            }
            None => super::disable(&identity.pool).await?,
        }
        Ok(sso)
    }

    /// The identity every other route shares; its browser origin may come from the provider.
    pub fn identity(&self) -> Identity {
        self.identity.clone()
    }

    pub async fn current(&self) -> Option<Oidc> {
        self.current.read().await.clone()
    }

    async fn stored(&self) -> Result<Option<(Config, Option<String>)>, IdentityError> {
        let row = sqlx::query!("SELECT config, client_secret FROM identity_oidc_policy")
            .fetch_one(&self.identity.pool)
            .await?;
        let Some(config) = row.config else {
            return Ok(None);
        };
        let config: Config =
            serde_json::from_value(config).map_err(|error| IdentityError::Invalid(error.into()))?;
        let secret =
            match (row.client_secret, &self.credentials) {
                (None, _) => None,
                (Some(record), Some(credentials)) => Some(
                    String::from_utf8(credentials.decrypt(&record, CONTEXT).map_err(|error| {
                        IdentityError::CredentialUnavailable(Some(error.into()))
                    })?)
                    .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?,
                ),
                (Some(_), None) => return Err(IdentityError::CredentialUnavailable(None)),
            };
        Ok(Some((config, secret)))
    }

    pub async fn settings(&self) -> Result<SingleSignOnSettings, IdentityError> {
        let redirect_uri = self.redirect_uri();
        let can_store_secret = self.credentials.is_some();
        if let Some((config, secret)) = &self.deployment {
            return Ok(view(
                true,
                Some(config),
                *secret,
                redirect_uri,
                can_store_secret,
            ));
        }
        let row = sqlx::query!(
            r#"SELECT enabled, config, client_secret IS NOT NULL AS "secret!" FROM identity_oidc_policy"#
        )
        .fetch_one(&self.identity.pool)
        .await?;
        let config = row
            .config
            .and_then(|config| serde_json::from_value::<Config>(config).ok())
            .filter(|_| row.enabled);
        Ok(view(
            false,
            config.as_ref(),
            row.secret,
            redirect_uri,
            can_store_secret,
        ))
    }

    fn redirect_uri(&self) -> Option<String> {
        self.identity
            .browser_origin
            .as_ref()
            .map(|origin| format!("{origin}{CALLBACK}"))
    }

    /// Checks the issuer's discovery before saving, so a typo never locks people out.
    #[tracing::instrument(skip_all)]
    pub async fn save(
        &self,
        input: SaveSingleSignOn,
    ) -> Result<SingleSignOnSettings, IdentityError> {
        if self.deployment.is_some() {
            return Err(IdentityError::Conflict);
        }
        let redirect_uri = self.redirect_uri().ok_or(IdentityError::InvalidInput)?;
        let config = Config::parse(
            &serde_json::json!({
                "issuer": input.issuer.trim(),
                "client_id": input.client_id.trim(),
                "redirect_uri": redirect_uri,
                "admit_subjects": input.admit_subjects,
                "admit_email_domains": input.admit_email_domains,
            })
            .to_string(),
        )?;
        let secret = self.secret(input.client_secret).await?;
        let http = Http::new(config.issuer.starts_with("http://"))?;
        provider::client(&config, secret.as_deref(), &http).await?;
        let record = self.seal(secret.as_deref())?;
        let stored =
            serde_json::to_value(&config).map_err(|error| IdentityError::Invalid(error.into()))?;
        let mut current = self.current.write().await;
        sqlx::query!(
            "UPDATE identity_oidc_policy SET config=$1, client_secret=$2 WHERE singleton",
            stored,
            record,
        )
        .execute(&self.identity.pool)
        .await?;
        *current = Some(Oidc::configure(self.identity.clone(), config, secret).await?);
        drop(current);
        self.settings().await
    }

    /// An absent secret keeps the stored one; an empty one removes it.
    async fn secret(&self, input: Option<String>) -> Result<Option<String>, IdentityError> {
        let secret = match input {
            Some(secret) => Some(secret).filter(|secret| !secret.is_empty()),
            None => self.stored().await?.and_then(|(_, secret)| secret),
        };
        if secret
            .as_ref()
            .is_some_and(|secret| secret.len() > 1024 || secret.chars().any(char::is_control))
        {
            return Err(IdentityError::InvalidInput);
        }
        Ok(secret)
    }

    fn seal(&self, secret: Option<&str>) -> Result<Option<Vec<u8>>, IdentityError> {
        match (secret, &self.credentials) {
            (None, _) => Ok(None),
            (Some(secret), Some(credentials)) => credentials
                .encrypt(secret.as_bytes(), CONTEXT)
                .map(Some)
                .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into()))),
            (Some(_), None) => Err(IdentityError::CredentialUnavailable(None)),
        }
    }

    /// Ends single sign-on and every session it started; built-in sign-in keeps working.
    #[tracing::instrument(skip_all)]
    pub async fn disable(&self) -> Result<(), IdentityError> {
        if self.deployment.is_some() {
            return Err(IdentityError::Conflict);
        }
        let mut current = self.current.write().await;
        sqlx::query!(
            "UPDATE identity_oidc_policy SET config=NULL, client_secret=NULL WHERE singleton"
        )
        .execute(&self.identity.pool)
        .await?;
        super::disable(&self.identity.pool).await?;
        *current = None;
        Ok(())
    }
}

fn view(
    managed: bool,
    config: Option<&Config>,
    has_client_secret: bool,
    redirect_uri: Option<String>,
    can_store_secret: bool,
) -> SingleSignOnSettings {
    SingleSignOnSettings {
        managed,
        enabled: config.is_some(),
        issuer: config.map(|config| config.issuer.clone()),
        client_id: config.map(|config| config.client_id.clone()),
        has_client_secret: config.is_some() && has_client_secret,
        admit_subjects: config.map_or_else(Vec::new, |config| config.admit_subjects.clone()),
        admit_email_domains: config
            .map_or_else(Vec::new, |config| config.admit_email_domains.clone()),
        redirect_uri: config.map_or(redirect_uri, |config| Some(config.redirect_uri.clone())),
        can_store_secret,
    }
}

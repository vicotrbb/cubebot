use openidconnect::reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::identity::IdentityError;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub issuer: String,
    pub client_id: String,
    pub redirect_uri: String,
    #[serde(default)]
    pub admit_subjects: Vec<String>,
    #[serde(default)]
    pub admit_email_domains: Vec<String>,
}

impl Config {
    pub fn parse(input: &str) -> Result<Self, IdentityError> {
        let config: Self = serde_json::from_str(input).map_err(|_| IdentityError::InvalidInput)?;
        let issuer = endpoint(&config.issuer, true)?;
        let redirect = endpoint(&config.redirect_uri, true)?;
        if issuer.query().is_some()
            || redirect.query().is_some()
            || redirect.path() != "/api/identity/oidc/callback"
        {
            return Err(IdentityError::InvalidInput);
        }
        if config.client_id.is_empty()
            || config.client_id.len() > 255
            || config.admit_subjects.len() > 1000
        {
            return Err(IdentityError::InvalidInput);
        }
        for domain in &config.admit_email_domains {
            validate_domain(domain)?;
        }
        Ok(config)
    }

    pub fn public_origin(value: &str) -> Result<String, IdentityError> {
        let url = endpoint(value, true)?;
        if url.path() != "/" || url.query().is_some() {
            return Err(IdentityError::InvalidInput);
        }
        Ok(url.origin().ascii_serialization())
    }

    pub fn origin(&self) -> Result<String, IdentityError> {
        Ok(endpoint(&self.redirect_uri, true)?
            .origin()
            .ascii_serialization())
    }

    pub(super) fn permits(&self, subject: &str, email: Option<&str>, verified: bool) -> bool {
        if self.admit_subjects.iter().any(|allowed| allowed == subject) {
            return true;
        }
        if !verified {
            return false;
        }
        let Some((_, domain)) = email.and_then(|email| email.rsplit_once('@')) else {
            return false;
        };
        self.admit_email_domains
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(domain))
    }
}

fn validate_domain(domain: &str) -> Result<(), IdentityError> {
    let url = endpoint(&format!("https://{domain}"), false)?;
    if domain.is_empty()
        || url.host_str() != Some(domain.to_ascii_lowercase().as_str())
        || url.port().is_some()
        || url.path() != "/"
        || url.query().is_some()
    {
        return Err(IdentityError::InvalidInput);
    }
    Ok(())
}

pub(super) fn endpoint(value: &str, local_http: bool) -> Result<Url, IdentityError> {
    let url = Url::parse(value).map_err(|_| IdentityError::InvalidInput)?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
    let secure = url.scheme() == "https" || (local_http && local && url.scheme() == "http");
    if !secure
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(IdentityError::InvalidInput);
    }
    Ok(url)
}

pub fn environment() -> Result<Option<(Config, Option<String>)>, IdentityError> {
    use std::io::Read;
    let path = match std::env::var("CUBEBOT_OIDC_CONFIG") {
        Ok(path) => path,
        Err(std::env::VarError::NotPresent) => return Ok(None),
        Err(_) => return Err(IdentityError::InvalidInput),
    };
    let file = std::fs::File::open(path).map_err(|_| IdentityError::InvalidInput)?;
    let mut input = String::new();
    file.take(65537)
        .read_to_string(&mut input)
        .map_err(|_| IdentityError::InvalidInput)?;
    if input.len() > 65536 {
        return Err(IdentityError::InvalidInput);
    }
    let secret = match std::env::var("CUBEBOT_OIDC_CLIENT_SECRET") {
        Ok(secret) => Some(secret),
        Err(std::env::VarError::NotPresent) => None,
        Err(_) => return Err(IdentityError::InvalidInput),
    };
    Ok(Some((Config::parse(&input)?, secret)))
}

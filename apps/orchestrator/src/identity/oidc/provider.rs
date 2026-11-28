use super::{Config, http::Http};
use crate::identity::IdentityError;
use openidconnect::{
    AccessTokenHash, AuthorizationCode, ClientId, ClientSecret, EndpointMaybeSet, EndpointNotSet,
    EndpointSet, Nonce, OAuth2TokenResponse, PkceCodeVerifier, RedirectUrl, TokenResponse,
    core::{CoreClient, CoreProviderMetadata, CoreUserInfoClaims},
};

pub(super) type Client = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

#[tracing::instrument(skip_all, fields(issuer = %config.issuer))]
pub(super) async fn client(
    config: &Config,
    secret: Option<&str>,
    http: &Http,
) -> Result<Client, IdentityError> {
    let metadata = CoreProviderMetadata::discover_async(
        openidconnect::IssuerUrl::new(config.issuer.clone())
            .map_err(|error| IdentityError::Invalid(error.into()))?,
        http,
    )
    .await
    .map_err(|error| IdentityError::OidcUnavailable(error.into()))?;
    super::config::endpoint(
        metadata.authorization_endpoint().as_str(),
        config.issuer.starts_with("http://"),
    )?;
    Ok(CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(config.client_id.clone()),
        secret.map(|secret| ClientSecret::new(secret.to_owned())),
    )
    .set_redirect_uri(
        RedirectUrl::new(config.redirect_uri.clone())
            .map_err(|error| IdentityError::Invalid(error.into()))?,
    ))
}

pub(super) struct Verified {
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
    /// The groups claim's values, when the provider is set to read one.
    pub groups: Vec<String>,
}

/// Reads one claim of an ID token whose signature and claims were already verified.
fn groups_from(id: &openidconnect::core::CoreIdToken, claim: &str) -> Vec<String> {
    use base64::Engine;
    let token = id.to_string();
    let payload = token
        .split('.')
        .nth(1)
        .and_then(|part| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(part)
                .ok()
        })
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let values = payload.as_ref().and_then(|payload| payload.get(claim));
    let listed = match values {
        Some(serde_json::Value::Array(values)) => {
            values.iter().filter_map(|value| value.as_str()).collect()
        }
        Some(serde_json::Value::String(value)) => vec![value.as_str()],
        _ => Vec::new(),
    };
    listed
        .into_iter()
        .filter(|value| !value.is_empty() && value.len() <= 320)
        .take(1000)
        .map(str::to_owned)
        .collect()
}

#[tracing::instrument(skip_all)]
pub(super) async fn verify(
    client: &Client,
    http: &Http,
    code: String,
    browser: String,
    nonce: String,
    need_email: bool,
    groups_claim: Option<&str>,
) -> Result<Verified, IdentityError> {
    let token = client
        .exchange_code(AuthorizationCode::new(code))
        .map_err(|error| IdentityError::OidcUnavailable(error.into()))?
        .set_pkce_verifier(PkceCodeVerifier::new(browser))
        .request_async(http)
        .await
        .map_err(|error| match error {
            openidconnect::RequestTokenError::Request(error) => {
                IdentityError::OidcUnavailable(error.into())
            }
            _ => IdentityError::Unauthorized,
        })?;
    let id = token.id_token().ok_or(IdentityError::Unauthorized)?;
    let verifier = client.id_token_verifier();
    let claims = id
        .claims(&verifier, &Nonce::new(nonce))
        .map_err(|error| IdentityError::Rejected(error.into()))?;
    verify_access_hash(id, &token, &verifier, claims.access_token_hash())?;
    let mut identity = Verified {
        subject: claims.subject().as_str().to_owned(),
        email: claims.email().map(|email| email.as_str().to_owned()),
        email_verified: claims.email_verified() == Some(true),
        groups: groups_claim.map_or_else(Vec::new, |claim| groups_from(id, claim)),
    };
    if need_email && identity.email.is_none() {
        let user: CoreUserInfoClaims = client
            .user_info(token.access_token().clone(), Some(claims.subject().clone()))
            .map_err(|error| IdentityError::Rejected(error.into()))?
            .request_async(http)
            .await
            .map_err(|error| IdentityError::Rejected(error.into()))?;
        identity.email = user.email().map(|email| email.as_str().to_owned());
        identity.email_verified = user.email_verified() == Some(true);
    }
    if identity.subject.is_empty() || identity.subject.len() > 255 || !identity.subject.is_ascii() {
        return Err(IdentityError::Unauthorized);
    }
    Ok(identity)
}

fn verify_access_hash(
    id: &openidconnect::core::CoreIdToken,
    token: &openidconnect::core::CoreTokenResponse,
    verifier: &openidconnect::core::CoreIdTokenVerifier<'_>,
    expected: Option<&AccessTokenHash>,
) -> Result<(), IdentityError> {
    if let Some(expected) = expected {
        let actual = AccessTokenHash::from_token(
            token.access_token(),
            id.signing_alg()
                .map_err(|error| IdentityError::Rejected(error.into()))?,
            id.signing_key(verifier)
                .map_err(|error| IdentityError::Rejected(error.into()))?,
        )
        .map_err(|error| IdentityError::Rejected(error.into()))?;
        if actual != *expected {
            return Err(IdentityError::Unauthorized);
        }
    }
    Ok(())
}

//! Sign in with ChatGPT for open-source apps (https://developers.openai.com/siwc). Never Codex's
//! client id or `chatgpt.com/backend-api`.
use super::{
    Connections, administrator,
    subscriptions::{Failure, Flow, Grant, SignIn, SignInError, SubscriptionSummary, Tokens},
};
use crate::identity::{Actor, IdentityError};
use cubebot_protocol::runtime::ProviderKind;
use openidconnect::{
    ClientId, CsrfToken, IssuerUrl, JsonWebKeySetUrl, Nonce, PkceCodeChallenge,
    core::{CoreIdToken, CoreIdTokenVerifier, CoreJsonWebKeySet},
};
use reqwest::Url;
use sqlx::{Postgres, Transaction};
use std::time::{Duration, Instant};
use uuid::Uuid;

const ISSUER: &str = "https://auth.openai.com/";
const RESOURCE: &str = "https://api.openai.com/v1";
const SCOPE: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const PLAN: &str = "chatgpt.tokens.use.direct";
/// Nothing listens here: the browser fails to load it and the person pastes its address.
/// Not 1455, where a local Codex sign-in listens.
const REDIRECT: &str = "http://127.0.0.1:1456/auth/callback";

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct ChatgptSignIn {
    pub sign_in: Uuid,
    /// Opened in the person's browser; it ends on a `127.0.0.1` address they paste back.
    pub url: String,
}

#[derive(Debug, PartialEq)]
struct Callback {
    code: String,
    client_id: Option<String>,
}

impl Connections {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn start_chatgpt(&self, actor: &Actor) -> Result<ChatgptSignIn, IdentityError> {
        let mut tx = self.pool.begin().await?;
        administrator(&mut tx, actor).await?;
        let installation =
            sqlx::query!("SELECT agent_host_id, chatgpt_client_id FROM installation")
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (state, nonce) = (CsrfToken::new_random(), Nonce::new_random());
        let url = authorize_url(
            self.vendor(),
            installation.chatgpt_client_id.as_deref(),
            installation.agent_host_id,
            challenge.as_str(),
            state.secret(),
            nonce.secret(),
        )
        .map_err(|_| IdentityError::CredentialUnavailable(None))?;
        let sign_in = self.sign_ins.start(SignIn {
            actor: actor.id,
            until: Instant::now() + Duration::from_secs(15 * 60),
            flow: Flow::Chatgpt {
                verifier: verifier.secret().clone(),
                state: state.secret().clone(),
                nonce: nonce.secret().clone(),
                client_id: installation.chatgpt_client_id,
            },
        })?;
        Ok(ChatgptSignIn {
            sign_in,
            url: url.into(),
        })
    }

    /// Finishes with the `127.0.0.1` address the browser landed on.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, sign_in = %sign_in))]
    pub async fn finish_chatgpt(
        &self,
        actor: &Actor,
        sign_in: Uuid,
        landed: &str,
    ) -> Result<SubscriptionSummary, SignInError> {
        let (verifier, nonce, saved, back) =
            self.sign_ins.with(sign_in, actor.id, |pending| {
                let Flow::Chatgpt {
                    verifier,
                    state,
                    nonce,
                    client_id,
                } = &pending.flow
                else {
                    return (false, Err(SignInError::Refused("sign_in_expired")));
                };
                match callback(landed, state) {
                    // A mistyped paste may be retried; a vendor's answer settles the sign-in.
                    Err(SignInError::Identity(error)) => (false, Err(error.into())),
                    back => (
                        true,
                        back.map(|back| (verifier.clone(), nonce.clone(), client_id.clone(), back)),
                    ),
                }
            })??;
        let client_id = issued(saved.as_deref(), back.client_id)?;
        let grant = super::subscriptions::tokens(
            token_url(self.vendor())?,
            &[
                ("grant_type", "authorization_code"),
                ("client_id", &client_id),
                ("code", &back.code),
                ("code_verifier", &verifier),
                ("redirect_uri", REDIRECT),
                ("resource", RESOURCE),
            ],
            None,
        )
        .await?;
        if !grant
            .scope
            .as_deref()
            .is_some_and(|scope| scope.split(' ').any(|granted| granted == PLAN))
        {
            return Err(SignInError::Refused("plan_not_included"));
        }
        let account = account(self.vendor(), &grant, &client_id, &nonce).await?;
        self.keep_client(&client_id).await?;
        Ok(self
            .signed_in(actor, ProviderKind::Chatgpt, &account, grant)
            .await?)
    }

    /// The first sign-in registers the installation's client; later ones reuse it.
    async fn keep_client(&self, client_id: &str) -> Result<(), IdentityError> {
        sqlx::query!(
            "UPDATE installation SET chatgpt_client_id = $1 WHERE chatgpt_client_id IS NULL",
            client_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn authorize_url(
    vendor: Option<&Url>,
    client_id: Option<&str>,
    host: Uuid,
    challenge: &str,
    state: &str,
    nonce: &str,
) -> Result<Url, Failure> {
    let host = format!("urn:uuid:{host}");
    let mut params = vec![
        ("client_id", client_id.unwrap_or("dynamic_agent_client")),
        ("ext_agent_host_id", &host),
        ("response_type", "code"),
        ("redirect_uri", REDIRECT),
        ("scope", SCOPE),
        ("resource", RESOURCE),
        ("state", state),
        ("nonce", nonce),
        ("code_challenge_method", "S256"),
        ("code_challenge", challenge),
    ];
    if client_id.is_none() {
        params.push(("agent_name_hint", "Cubebot"));
    }
    let mut url = super::subscriptions::endpoint(vendor, ISSUER, "api/accounts/authorize")?;
    url.query_pairs_mut().extend_pairs(params);
    Ok(url)
}

/// Checks a pasted address is our callback for this sign-in. `Identity` errors are the person's
/// paste; `Refused` ones are the vendor's answer.
fn callback(landed: &str, state: &str) -> Result<Callback, SignInError> {
    let url = Url::parse(landed.trim()).map_err(|_| IdentityError::InvalidInput)?;
    if landed.len() > 8192
        || url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port() != Some(1456)
        || url.path() != "/auth/callback"
    {
        return Err(IdentityError::InvalidInput.into());
    }
    let field = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    if field("state").as_deref() != Some(state) {
        return Err(IdentityError::InvalidInput.into());
    }
    if field("error").is_some() {
        return Err(SignInError::Refused("sign_in_denied"));
    }
    let code = field("code")
        .filter(|code| {
            (1..=2048).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_graphic())
        })
        .ok_or(SignInError::Refused("sign_in_refused"))?;
    Ok(Callback {
        code,
        client_id: field("client_id"),
    })
}

/// The issued client: a new registration returns one; a returning one must not change it.
fn issued(saved: Option<&str>, returned: Option<String>) -> Result<String, SignInError> {
    let valid = |id: &str| {
        id.strip_prefix("oaiapp_").is_some_and(|rest| {
            (1..=200).contains(&rest.len())
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        })
    };
    match (saved, returned) {
        (Some(saved), Some(returned)) if saved != returned => {
            Err(SignInError::Refused("sign_in_mismatch"))
        }
        (Some(saved), _) => Ok(saved.to_owned()),
        (None, Some(returned)) if valid(&returned) => Ok(returned),
        (None, _) => Err(SignInError::Refused("sign_in_refused")),
    }
}

fn token_url(vendor: Option<&Url>) -> Result<Url, Failure> {
    super::subscriptions::endpoint(vendor, ISSUER, "api/accounts/oauth/token")
}

/// The verified ID token's account: signature, issuer, audience, expiry and nonce.
async fn account(
    vendor: Option<&Url>,
    grant: &Grant,
    client_id: &str,
    nonce: &str,
) -> Result<String, SignInError> {
    let id_token = grant
        .tokens
        .id_token
        .as_deref()
        .ok_or(SignInError::Refused("sign_in_refused"))?;
    let unreachable =
        |error: Box<dyn std::error::Error + Send + Sync>| SignInError::Unreachable(Some(error));
    let issuer = super::subscriptions::endpoint(vendor, ISSUER, "")?;
    let keys = JsonWebKeySetUrl::from_url(
        issuer
            .join(".well-known/jwks.json")
            .map_err(|e| unreachable(e.into()))?,
    );
    let http = super::subscriptions::http()?;
    let keys = CoreJsonWebKeySet::fetch_async(&keys, &http)
        .await
        .map_err(|error| unreachable(error.to_string().into()))?;
    let issuer = IssuerUrl::new(issuer.as_str().trim_end_matches('/').to_owned())
        .map_err(|error| unreachable(error.into()))?;
    let verifier =
        CoreIdTokenVerifier::new_public_client(ClientId::new(client_id.into()), issuer, keys);
    let token: CoreIdToken = id_token
        .parse()
        .map_err(|_| SignInError::Refused("sign_in_refused"))?;
    let claims = token
        .claims(&verifier, &Nonce::new(nonce.into()))
        .map_err(|error| {
            tracing::warn!(%error, "ChatGPT ID token rejected");
            SignInError::Refused("sign_in_refused")
        })?;
    let name = claims
        .email()
        .map(|email| email.as_str())
        .unwrap_or(claims.subject().as_str());
    super::subscriptions::account(name).ok_or(SignInError::Refused("sign_in_refused"))
}

pub(super) async fn refresh(
    vendor: Option<&Url>,
    tx: &mut Transaction<'_, Postgres>,
    tokens: &Tokens,
) -> Result<Grant, Failure> {
    let client_id = sqlx::query_scalar!("SELECT chatgpt_client_id FROM installation")
        .fetch_one(&mut **tx)
        .await
        .map_err(|error| Failure::Unavailable(Some(error.into())))?
        .ok_or(Failure::Oauth("invalid_client".into()))?;
    super::subscriptions::tokens(
        token_url(vendor)?,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", &client_id),
            ("refresh_token", &tokens.refresh_token),
            ("resource", RESOURCE),
        ],
        Some(&tokens.refresh_token),
    )
    .await
}

/// OpenAI answers an empty 200 even for a token already invalid.
pub(super) async fn revoke(
    vendor: Option<&Url>,
    client_id: Option<&str>,
    tokens: &Tokens,
) -> Result<(), Failure> {
    let client_id = client_id.ok_or(Failure::Oauth("invalid_client".into()))?;
    let url = super::subscriptions::endpoint(vendor, ISSUER, "api/accounts/oauth/revoke")?;
    super::subscriptions::post(
        url,
        &[
            ("token", &tokens.refresh_token),
            ("token_type_hint", "refresh_token"),
            ("client_id", client_id),
        ],
    )
    .await
    .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_sign_in_registers_and_a_later_one_reuses_the_issued_client() {
        let url = authorize_url(None, None, Uuid::nil(), "challenge", "state", "nonce").unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(
            url.as_str().split('?').next(),
            Some("https://auth.openai.com/api/accounts/authorize")
        );
        assert_eq!(query["client_id"], "dynamic_agent_client");
        assert_eq!(query["agent_name_hint"], "Cubebot");
        assert_eq!(
            query["ext_agent_host_id"],
            "urn:uuid:00000000-0000-0000-0000-000000000000"
        );
        assert_eq!(query["resource"], "https://api.openai.com/v1");
        assert_eq!(query["redirect_uri"], "http://127.0.0.1:1456/auth/callback");
        assert_eq!(query["code_challenge_method"], "S256");
        assert!(query["scope"].split(' ').any(|scope| scope == PLAN));
        let again = authorize_url(None, Some("oaiapp_abc"), Uuid::nil(), "c", "s", "n").unwrap();
        assert!(
            again
                .query_pairs()
                .any(|(key, value)| key == "client_id" && value == "oaiapp_abc")
        );
        assert!(!again.query_pairs().any(|(key, _)| key == "agent_name_hint"));
    }

    #[test]
    fn only_this_sign_ins_own_callback_is_accepted() {
        let good =
            "http://127.0.0.1:1456/auth/callback?code=abc&scope=x&state=s1&client_id=oaiapp_X1";
        assert_eq!(
            callback(good, "s1").ok(),
            Some(Callback {
                code: "abc".into(),
                client_id: Some("oaiapp_X1".into())
            })
        );
        for paste in [
            "http://127.0.0.1:1456/auth/callback?code=abc&state=other",
            "http://localhost:1456/auth/callback?code=abc&state=s1",
            "https://127.0.0.1:1456/auth/callback?code=abc&state=s1",
            "http://127.0.0.1:1455/auth/callback?code=abc&state=s1",
            "http://127.0.0.1:1456/elsewhere?code=abc&state=s1",
            "not a url",
        ] {
            assert!(
                matches!(callback(paste, "s1"), Err(SignInError::Identity(_))),
                "{paste}"
            );
        }
        let denied = "http://127.0.0.1:1456/auth/callback?error=access_denied&state=s1";
        assert!(matches!(
            callback(denied, "s1"),
            Err(SignInError::Refused("sign_in_denied"))
        ));
    }

    #[test]
    fn a_returned_client_must_be_issued_and_never_replace_the_saved_one() {
        assert_eq!(
            issued(None, Some("oaiapp_A1".into())).ok().as_deref(),
            Some("oaiapp_A1")
        );
        assert!(issued(None, Some("dynamic_agent_client".into())).is_err());
        assert!(issued(None, None).is_err());
        assert_eq!(
            issued(Some("oaiapp_A1"), None).ok().as_deref(),
            Some("oaiapp_A1")
        );
        assert!(matches!(
            issued(Some("oaiapp_A1"), Some("oaiapp_B2".into())),
            Err(SignInError::Refused("sign_in_mismatch"))
        ));
    }
}

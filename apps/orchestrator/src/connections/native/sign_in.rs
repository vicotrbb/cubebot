//! Signing in through the vendor with the person's own app (no Cubebot-registered apps): OAuth
//! with PKCE that lands on a pasted loopback address or this installation's callback, the
//! device-code grant, and GitHub's app manifest. The protocol steps are the subscriptions' own.
use super::{
    ConnectError, ConnectionSummary, Draft, LOOPBACK, Sealed, callback, connector, method,
    settings, slug,
};
use crate::connections::{
    Connections, ProviderAccess,
    subscriptions::{self, Failure, Flow, Grant, SignInError, Waiting},
};
use cubebot_connectors::{Auth, Credential, Level, OAuth2, github};
use openidconnect::{CsrfToken, PkceCodeChallenge};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StartSignIn {
    pub connector: String,
    pub slug: String,
    /// `oauth2` or `github_app`, as the connector offers.
    pub auth: String,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub config: Map<String, Value>,
    /// The client secret, when the person's OAuth client has one.
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
    #[serde(default)]
    pub access: Option<ProviderAccess>,
    #[serde(default)]
    #[schema(value_type = Option<cubebot_connectors::Level>)]
    pub level: Option<Level>,
    /// Lands on this installation's callback instead of a loopback address to paste back.
    #[serde(default)]
    pub callback: bool,
    /// Signs in this connection again, keeping its access, level and any secret not given.
    #[serde(default)]
    pub connection: Option<Uuid>,
}

#[derive(Serialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignInStarted {
    /// Open `url`; the browser ends on `redirect_uri`, which the person pastes back unless it is
    /// this installation's callback.
    Redirect {
        sign_in: Uuid,
        url: String,
        redirect_uri: String,
    },
    /// Enter `user_code` at `url`, then finish until it answers.
    Device {
        sign_in: Uuid,
        url: String,
        user_code: String,
        expires_in: u64,
        interval: u64,
    },
    /// Submit `manifest` as the form field `manifest` to `url` in a new tab; the vendor then sends
    /// the browser to `redirect_uri`.
    Form {
        sign_in: Uuid,
        url: String,
        manifest: String,
        redirect_uri: String,
    },
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FinishSignIn {
    /// The address the browser landed on; absent while a device sign-in waits.
    #[serde(default)]
    pub url: Option<String>,
}

/// What the start's answer will be, once the sign-in has its id.
type Started = Box<dyn FnOnce(Uuid) -> SignInStarted>;

/// A sign-in in progress, held with the subscriptions' (`subscriptions::Pending`).
pub(in crate::connections) struct SignIn {
    draft: Draft,
    secrets: BTreeMap<String, String>,
    replacing: Option<Uuid>,
    step: Step,
}

enum Step {
    Redirect {
        verifier: String,
        state: String,
        redirect: Url,
    },
    Device {
        device_code: String,
        interval: Duration,
        next: Instant,
    },
    GithubApp {
        state: String,
        redirect: Url,
    },
}

/// A vendor's answer short of success, as the person reads it.
fn said(error: SignInError) -> ConnectError {
    match error {
        SignInError::Identity(error) => error.into(),
        SignInError::Refused(code) => ConnectError::Refused(
            match code {
                "sign_in_expired" => "The sign-in expired; start again.",
                "sign_in_denied" => "The sign-in was declined.",
                _ => "The vendor refused the sign-in; check the app's settings and try again.",
            }
            .into(),
        ),
        SignInError::Unreachable(source) => {
            tracing::warn!(error = ?source.map(|source| source.to_string()), "connection sign-in unreachable");
            ConnectError::Refused("The vendor could not be reached; try again.".into())
        }
    }
}

impl From<Failure> for ConnectError {
    fn from(failure: Failure) -> Self {
        said(failure.into())
    }
}

impl Connections {
    /// An OAuth endpoint, with `{key}` filled from the connection's settings, or a test's loopback.
    pub(super) fn endpoint(&self, template: &str, config: &Value) -> Result<Url, ConnectError> {
        let url = filled(template, config)
            .and_then(|filled| Url::parse(&filled).ok())
            .ok_or_else(|| {
                ConnectError::Invalid("a setting the sign-in needs is missing".into())
            })?;
        let Some(vendor) = self.vendor() else {
            return Ok(url);
        };
        let mut rerooted = vendor.clone();
        rerooted.set_path(&format!(
            "/{}{}",
            url.host_str().unwrap_or_default(),
            url.path()
        ));
        Ok(rerooted)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor, connector = %input.connector))]
    pub async fn start_connection_sign_in(
        &self,
        actor: Uuid,
        input: StartSignIn,
        origin: Option<&str>,
    ) -> Result<SignInStarted, ConnectError> {
        let (draft, secrets, replacing) = self.draft(input.connection, &input).await?;
        let redirect = redirect_to(input.callback, origin)?;
        let auth = method(draft.connector.manifest(), draft.auth)?;
        let (step, started) = match auth {
            Auth::OAuth2(oauth) if oauth.device.is_some() => self.device(&draft, oauth).await?,
            Auth::OAuth2(oauth) => self.redirect(&draft, oauth, redirect)?,
            Auth::GithubApp { .. } => self.github_form(&draft, redirect, origin)?,
            _ => return Err(ConnectError::Invalid("this sign-in needs no vendor".into())),
        };
        let until = Instant::now() + Duration::from_secs(15 * 60);
        let sign_in = Box::new(SignIn {
            draft,
            secrets,
            replacing,
            step,
        });
        let id = self.sign_ins.start(subscriptions::SignIn {
            actor,
            until,
            flow: Flow::Connection(sign_in),
        })?;
        Ok(started(id))
    }

    /// What the sign-in will save: a new connection, or one signing in again.
    async fn draft(
        &self,
        existing: Option<Uuid>,
        input: &StartSignIn,
    ) -> Result<(Draft, BTreeMap<String, String>, Option<Uuid>), ConnectError> {
        let Some(id) = existing else {
            let connector = connector(&input.connector)?;
            let manifest = connector.manifest();
            let auth = method(manifest, &input.auth)?;
            slug(&input.slug)?;
            let config = settings(
                manifest,
                auth,
                input.config.clone(),
                &input.secrets,
                &Value::Null,
            )?;
            let draft = Draft {
                connector,
                slug: input.slug.clone(),
                auth: auth.kind(),
                config,
                access: input.access.clone().unwrap_or_default(),
                level: input.level.unwrap_or(manifest.level),
            };
            return Ok((draft, input.secrets.clone(), None));
        };
        let mut tx = self.pool.begin().await?;
        let stored = self.stored(&mut tx, id, false).await?;
        tx.commit().await?;
        let manifest = stored.connector.manifest();
        let auth = method(manifest, &stored.auth)?;
        let mut secrets = stored.sealed.fields;
        secrets.extend(input.secrets.clone());
        let config = match input.config.is_empty() {
            true => stored.config,
            false => settings(
                manifest,
                auth,
                input.config.clone(),
                &secrets,
                &stored.config,
            )?,
        };
        let draft = Draft {
            connector: stored.connector,
            slug: String::new(),
            auth: auth.kind(),
            config,
            access: ProviderAccess::default(),
            level: manifest.level,
        };
        Ok((draft, secrets, Some(id)))
    }

    fn redirect(
        &self,
        draft: &Draft,
        oauth: &OAuth2,
        redirect: Url,
    ) -> Result<(Step, Started), ConnectError> {
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let state = CsrfToken::new_random().secret().clone();
        let scopes = draft
            .connector
            .manifest()
            .scopes(oauth, &draft.config)
            .join(" ");
        let mut url = self.endpoint(oauth.authorize, &draft.config)?;
        url.query_pairs_mut()
            .append_pair(
                "client_id",
                draft.config["client_id"].as_str().unwrap_or_default(),
            )
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect.as_str())
            .append_pair("scope", &scopes)
            .append_pair("state", &state)
            .append_pair("code_challenge", challenge.as_str())
            .append_pair("code_challenge_method", "S256")
            .extend_pairs(oauth.params);
        let (url, redirect_uri) = (url.to_string(), redirect.to_string());
        let step = Step::Redirect {
            verifier: verifier.secret().clone(),
            state,
            redirect,
        };
        Ok((
            step,
            Box::new(move |sign_in| SignInStarted::Redirect {
                sign_in,
                url,
                redirect_uri,
            }),
        ))
    }

    async fn device(&self, draft: &Draft, oauth: &OAuth2) -> Result<(Step, Started), ConnectError> {
        let device_url = oauth.device.unwrap_or_default();
        let url = self.endpoint(device_url, &draft.config)?;
        let scopes = draft
            .connector
            .manifest()
            .scopes(oauth, &draft.config)
            .join(" ");
        let client_id = draft.config["client_id"].as_str().unwrap_or_default();
        let device =
            subscriptions::device(url, &[("client_id", client_id), ("scope", &scopes)]).await?;
        let (page, interval) = device
            .validated(|_| true, self.controlled())
            .ok_or(Failure::Unavailable(None))?;
        let step = Step::Device {
            device_code: device.device_code,
            interval,
            next: Instant::now() + interval,
        };
        let (user_code, expires_in) = (device.user_code, device.expires_in);
        Ok((
            step,
            Box::new(move |sign_in| SignInStarted::Device {
                sign_in,
                url: page,
                user_code,
                expires_in,
                interval: interval.as_secs(),
            }),
        ))
    }

    fn github_form(
        &self,
        draft: &Draft,
        redirect: Url,
        origin: Option<&str>,
    ) -> Result<(Step, Started), ConnectError> {
        let state = CsrfToken::new_random().secret().clone();
        // App names are unique across GitHub, so each installation's carries its own mark.
        let mark = &state[..6.min(state.len())].to_ascii_lowercase();
        let name = format!("Cubebot {} {mark}", draft.slug);
        let home = origin.unwrap_or("https://github.com/vicotrbb/cubebot");
        let manifest = github::app_manifest(&name, home, redirect.as_str()).to_string();
        let organization = draft.config["organization"].as_str();
        let url = github::app_creation_url(organization, &state);
        let redirect_uri = redirect.to_string();
        Ok((
            Step::GithubApp { state, redirect },
            Box::new(move |sign_in| SignInStarted::Form {
                sign_in,
                url,
                manifest,
                redirect_uri,
            }),
        ))
    }

    /// The sign-in a callback's `state` belongs to.
    pub fn connection_sign_in_for(&self, state: &str) -> Option<Uuid> {
        self.sign_ins.find(|flow| match flow {
            Flow::Connection(sign_in) => match &sign_in.step {
                Step::Redirect { state: own, .. } | Step::GithubApp { state: own, .. } => {
                    own == state
                }
                Step::Device { .. } => false,
            },
            _ => false,
        })
    }

    /// Finishes with the address the browser landed on, or polls a device sign-in: `None` while
    /// the person has not approved yet.
    #[tracing::instrument(skip_all, fields(actor = %actor, sign_in = %id))]
    pub async fn finish_connection_sign_in(
        &self,
        actor: Uuid,
        id: Uuid,
        input: FinishSignIn,
    ) -> Result<Option<ConnectionSummary>, ConnectError> {
        let landed = input.url.unwrap_or_default();
        let next = self
            .sign_ins
            .with(id, actor, |pending| {
                let Flow::Connection(sign_in) = &mut pending.flow else {
                    return (false, Err(SignInError::Refused("sign_in_expired")));
                };
                advance(sign_in, &landed)
            })
            .map_err(said)?
            .map_err(said)?;
        let Some((sign_in, code)) = next else {
            return Ok(None);
        };
        let saved = match &sign_in.step {
            Step::Redirect {
                verifier, redirect, ..
            } => self.exchanged(&sign_in, &code, verifier, redirect).await,
            Step::Device { .. } => self.device_polled(actor, id, &sign_in, &code).await,
            Step::GithubApp { .. } => self.github_converted(&sign_in, &code).await.map(Some),
        };
        if !matches!(saved, Ok(None)) {
            self.sign_ins.finish(id);
        }
        match saved? {
            Some(account) => self.save(actor, &sign_in, account).await.map(Some),
            None => Ok(None),
        }
    }

    async fn exchanged(
        &self,
        sign_in: &SignIn,
        code: &str,
        verifier: &str,
        redirect: &Url,
    ) -> Result<Option<Signed>, ConnectError> {
        let oauth = oauth(sign_in)?;
        let url = self.endpoint(oauth.token, &sign_in.draft.config)?;
        let client_id = sign_in.draft.config["client_id"]
            .as_str()
            .unwrap_or_default();
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect.as_str()),
            ("client_id", client_id),
            ("code_verifier", verifier),
        ];
        if let Some(secret) = sign_in.secrets.get("client_secret") {
            form.push(("client_secret", secret));
        }
        let grant = subscriptions::tokens(url, &form, None).await?;
        self.signed(sign_in, grant).await.map(Some)
    }

    /// One poll of a device grant, when it is due.
    async fn device_polled(
        &self,
        actor: Uuid,
        id: Uuid,
        sign_in: &SignIn,
        device_code: &str,
    ) -> Result<Option<Signed>, ConnectError> {
        let oauth = oauth(sign_in)?;
        let url = self.endpoint(oauth.token, &sign_in.draft.config)?;
        let client_id = sign_in.draft.config["client_id"]
            .as_str()
            .unwrap_or_default();
        let form = [
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", client_id),
            ("device_code", device_code),
        ];
        match subscriptions::device_tokens(url, &form).await? {
            Ok(grant) => self.signed(sign_in, grant).await.map(Some),
            Err(Waiting::Pending { slower }) => {
                let _ = self.sign_ins.with(id, actor, |pending| {
                    if let Flow::Connection(sign_in) = &mut pending.flow
                        && let Step::Device { interval, .. } = &mut sign_in.step
                        && slower
                    {
                        *interval += Duration::from_secs(5);
                    }
                    (false, ())
                });
                Ok(None)
            }
            Err(Waiting::Refused(code)) => Err(said(SignInError::Refused(code))),
        }
    }

    /// Names the account the new tokens belong to.
    async fn signed(&self, sign_in: &SignIn, grant: Grant) -> Result<Signed, ConnectError> {
        let credential = Credential::Token {
            token: grant.tokens.access_token.clone(),
        };
        let account = self
            .check(sign_in.draft.connector, &sign_in.draft.config, credential)
            .await?;
        Ok(Signed {
            account: Some(account),
            config: sign_in.draft.config.clone(),
            sealed: Sealed {
                fields: sign_in.secrets.clone(),
                access_token: Some(grant.tokens.access_token),
                refresh_token: Some(grant.tokens.refresh_token),
            },
            expires_in: Some(grant.expires_in),
            problem: None,
        })
    }

    /// GitHub made the app; until the person installs it, the connection waits with that problem.
    async fn github_converted(&self, sign_in: &SignIn, code: &str) -> Result<Signed, ConnectError> {
        let anonymous = Credential::Fields {
            fields: BTreeMap::new(),
        };
        let app = github::convert(&self.context(Value::Null, anonymous), code)
            .await
            .map_err(|error| ConnectError::Refused(super::refusal(&error)))?;
        let mut config = sign_in.draft.config.clone();
        config["app_id"] = Value::from(app.id);
        config["app_slug"] = Value::from(app.slug.clone());
        let mut signed = Signed {
            account: None,
            config,
            sealed: Sealed {
                fields: BTreeMap::from([("pem".to_owned(), app.pem)]),
                ..Sealed::default()
            },
            expires_in: None,
            problem: None,
        };
        match self.minted(&mut signed.config, &signed.sealed).await {
            Ok((token, expires_at)) => {
                let credential = Credential::Token {
                    token: token.clone(),
                };
                signed.account = Some(
                    self.check(sign_in.draft.connector, &signed.config, credential)
                        .await?,
                );
                signed.sealed.access_token = Some(token);
                signed.expires_in = Some(expires_at - super::now());
            }
            Err(_) => {
                signed.problem = Some(format!(
                    "Install the app on the repositories it may use ({}), then test the connection.",
                    github::install_url(&app.slug)
                ));
            }
        }
        Ok(signed)
    }

    /// Saves a finished sign-in as a new connection, or into the one it signs in again.
    async fn save(
        &self,
        actor: Uuid,
        sign_in: &SignIn,
        signed: Signed,
    ) -> Result<ConnectionSummary, ConnectError> {
        let Some(id) = sign_in.replacing else {
            let draft = Draft {
                config: signed.config,
                ..sign_in.draft.copy()
            };
            let account = (signed.account, signed.expires_in, signed.problem);
            return self.insert(actor, draft, &signed.sealed, account).await;
        };
        let mut tx = self.pool.begin().await?;
        let credential = self.seal_connection(&mut *tx, id, &signed.sealed).await?;
        sqlx::query!(
            r#"
            UPDATE connections
            SET credential = $2, config = $3, account = COALESCE($4, account),
                expires_at = clock_timestamp() + make_interval(secs => $5),
                refresh_after = clock_timestamp() + make_interval(secs => $5 - LEAST(600, $5 / 2)),
                status = CASE WHEN $6::text IS NULL THEN 'connected' ELSE 'error' END, problem = $6,
                checked_at = clock_timestamp(), updated_at = clock_timestamp()
            WHERE id = $1
            "#,
            id,
            credential,
            signed.config,
            signed.account,
            signed.expires_in.map(|seconds| seconds as f64),
            signed.problem,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(self.connection_summary(id).await?)
    }
}

/// Where the vendor sends the browser: this installation's callback, or the loopback address.
/// `template` with each `{key}` replaced by the connection's setting of that name, when every one
/// is set and plain enough to sit in a URL or form.
pub(super) fn filled(template: &str, config: &Value) -> Option<String> {
    let mut filled = template.to_owned();
    for (key, value) in config.as_object().into_iter().flatten() {
        let Some(value) = value.as_str() else {
            continue;
        };
        let safe = (1..=64).contains(&value.len())
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-.".contains(&b));
        if safe {
            filled = filled.replace(&format!("{{{key}}}"), value);
        }
    }
    (!filled.contains('{')).then_some(filled)
}

fn redirect_to(callback_wanted: bool, origin: Option<&str>) -> Result<Url, ConnectError> {
    let redirect = match (callback_wanted, origin) {
        (true, Some(origin)) => callback(origin),
        (true, None) => {
            return Err(ConnectError::Invalid(
                "this installation has no public address".into(),
            ));
        }
        (false, _) => LOOPBACK.into(),
    };
    Url::parse(&redirect).map_err(|_| ConnectError::Invalid("a bad address".into()))
}

/// A sign-in the vendor finished, ready to save.
pub(super) struct Signed {
    account: Option<String>,
    config: Value,
    sealed: Sealed,
    expires_in: Option<i64>,
    problem: Option<String>,
}

fn oauth(sign_in: &SignIn) -> Result<&'static OAuth2, ConnectError> {
    match method(sign_in.draft.connector.manifest(), sign_in.draft.auth)? {
        Auth::OAuth2(oauth) => Ok(oauth),
        _ => Err(ConnectError::Invalid("not an OAuth sign-in".into())),
    }
}

/// Takes the sign-in's next step under its lock: the code from a landed address, or the device
/// code when a poll is due. A mistyped paste may be retried; a vendor's answer settles it.
#[allow(clippy::type_complexity)]
fn advance(
    sign_in: &mut SignIn,
    landed: &str,
) -> (bool, Result<Option<(SignIn, String)>, SignInError>) {
    let (state, redirect) = match &mut sign_in.step {
        Step::Device {
            device_code,
            interval,
            next,
        } => {
            if *next > Instant::now() {
                return (false, Ok(None));
            }
            *next = Instant::now() + *interval;
            let device_code = device_code.clone();
            return (false, Ok(Some((sign_in.copy(), device_code))));
        }
        Step::Redirect {
            state, redirect, ..
        }
        | Step::GithubApp { state, redirect } => (state.clone(), redirect.clone()),
    };
    match subscriptions::landed(landed, &redirect, &state)
        .and_then(|fields| subscriptions::code(&fields))
    {
        Err(SignInError::Identity(error)) => (false, Err(error.into())),
        Err(refused) => (true, Err(refused)),
        Ok(code) => (true, Ok(Some((sign_in.copy(), code)))),
    }
}

impl SignIn {
    fn copy(&self) -> Self {
        Self {
            draft: self.draft.copy(),
            secrets: self.secrets.clone(),
            replacing: self.replacing,
            step: match &self.step {
                Step::Redirect {
                    verifier,
                    state,
                    redirect,
                } => Step::Redirect {
                    verifier: verifier.clone(),
                    state: state.clone(),
                    redirect: redirect.clone(),
                },
                Step::Device {
                    device_code,
                    interval,
                    next,
                } => Step::Device {
                    device_code: device_code.clone(),
                    interval: *interval,
                    next: *next,
                },
                Step::GithubApp { state, redirect } => Step::GithubApp {
                    state: state.clone(),
                    redirect: redirect.clone(),
                },
            },
        }
    }
}

impl Draft {
    fn copy(&self) -> Self {
        Self {
            connector: self.connector,
            slug: self.slug.clone(),
            auth: self.auth,
            config: self.config.clone(),
            access: self.access.clone(),
            level: self.level,
        }
    }
}

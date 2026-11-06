//! Native connections (docs/adr/identity.md#native-connectors): accounts on compiled-in
//! connectors. Each credential is sealed to its connection; people see settings and status, and
//! an agent with access gets a short-lived credential that it holds in its own process. Callers
//! authorize people.
mod sign_in;
mod tend;

pub(super) use sign_in::SignIn;
pub use sign_in::{FinishSignIn, SignInStarted, StartSignIn};

use super::{Access, Connections, ProviderAccess, validate_access, vault::sealing_context};
use crate::{agents::AgentId, identity::IdentityError};
use cubebot_connectors::{Auth, Connector, Context, Credential, Level, Manifest};
use cubebot_protocol::runtime::{Connection, ConnectionCredential};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::{Postgres, Transaction};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Where an OAuth sign-in lands when the installation has no address of its own. Nothing listens
/// there: the browser fails to load it and the person pastes the address back.
pub(super) const LOOPBACK: &str = "http://127.0.0.1:1457/callback";
/// A credential handed to an agent has at least this long left; it asks again before then.
const SPARE: i64 = 2 * 60;
/// Settings the orchestrator keeps for itself, which a person never sets.
const MANAGED: [&str; 4] = ["linked", "app_id", "app_slug", "installation_id"];

/// Sealed as one record per connection; never logged or returned.
#[derive(Default, Serialize, Deserialize)]
struct Sealed {
    /// Pasted secrets by field key, an OAuth client's secret, or a GitHub App's private key.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    fields: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    access_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
}

/// Why a connection could not be saved or used, worded for the person.
#[derive(Debug)]
pub enum ConnectError {
    Identity(IdentityError),
    /// What they entered is not valid.
    Invalid(String),
    /// The vendor refused the sign-in or the request.
    Refused(String),
    /// The vendor refused the credential itself: a person must sign in again.
    SignInAgain,
}
impl From<IdentityError> for ConnectError {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error)
    }
}
impl From<sqlx::Error> for ConnectError {
    fn from(error: sqlx::Error) -> Self {
        Self::Identity(error.into())
    }
}

/// Every connector, and the addresses an OAuth client must allow.
#[derive(Serialize, utoipa::ToSchema)]
pub struct Catalog {
    #[schema(value_type = Vec<cubebot_connectors::Manifest>)]
    pub connectors: Vec<&'static Manifest>,
    /// The loopback address a sign-in lands on and the person pastes back.
    pub loopback_redirect: String,
    /// This installation's own callback, when it has a public address.
    pub callback_redirect: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ConnectionSummary {
    pub id: Uuid,
    pub connector: String,
    /// Prefixes the connection's tools as `<slug>__<tool>`.
    pub slug: String,
    /// How it signs in: `api_key`, `password`, `oauth2`, `github_app`, `linked`,
    /// `client_credentials` or `public`.
    pub auth: String,
    /// The account as its vendor names it.
    pub account: Option<String>,
    /// Settings that are not secret.
    #[schema(value_type = Object)]
    pub config: sqlx::types::Json<Value>,
    #[schema(value_type = ProviderAccess)]
    pub access: Access,
    #[schema(value_type = cubebot_connectors::Level)]
    pub level: String,
    #[schema(value_type = Vec<String>)]
    pub disabled_tools: sqlx::types::Json<Vec<String>>,
    /// `connected`, `sign_in_again` or `error`.
    pub status: String,
    pub problem: Option<String>,
    pub checked_at: Option<String>,
    /// When the current access token expires; the orchestrator renews it before then.
    pub expires_at: Option<String>,
    pub polled_at: Option<String>,
}

/// A connection signed in with pasted secrets, an app's own client credentials, another
/// connection or nothing; OAuth and GitHub Apps sign in through `POST /api/connections/sign-ins`.
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveConnection {
    pub connector: String,
    /// Lowercase letters, digits and hyphens, at most 32.
    pub slug: String,
    /// `api_key`, `password`, `linked`, `client_credentials` or `public`, as the connector offers.
    pub auth: String,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub config: Map<String, Value>,
    /// The sign-in method's secret fields; never returned.
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
    /// A linked sign-in's connection, such as the Google account Email signs in with.
    #[serde(default)]
    pub linked: Option<Uuid>,
    /// All agents when absent.
    #[serde(default)]
    pub access: Option<ProviderAccess>,
    /// The connector's default when absent.
    #[serde(default)]
    #[schema(value_type = Option<cubebot_connectors::Level>)]
    pub level: Option<Level>,
}

/// Each field present replaces the current one; changed settings or secrets are checked again.
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateConnection {
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub config: Option<Map<String, Value>>,
    #[serde(default)]
    pub secrets: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub access: Option<ProviderAccess>,
    #[serde(default)]
    #[schema(value_type = Option<cubebot_connectors::Level>)]
    pub level: Option<Level>,
    #[serde(default)]
    pub disabled_tools: Option<Vec<String>>,
}

/// A connection as one agent's Connections tab shows it.
#[derive(Serialize, utoipa::ToSchema)]
pub struct AgentConnection {
    pub id: Uuid,
    pub connector: String,
    pub slug: String,
    pub account: Option<String>,
    #[schema(value_type = cubebot_connectors::Level)]
    pub level: String,
    pub status: String,
    /// Every agent may use it; access is changed on the connection, not per agent.
    pub all_agents: bool,
    /// This agent may use it.
    pub enabled: bool,
    /// The events it reports, for routine triggers.
    pub events: Vec<String>,
}

/// What a connection reported recently and which routines it starts.
#[derive(Serialize, utoipa::ToSchema)]
pub struct ConnectionActivity {
    pub events: Vec<ConnectionEvent>,
    pub routines: Vec<EventRoutine>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ConnectionEvent {
    pub kind: String,
    /// Untrusted vendor text.
    pub summary: String,
    pub received_at: String,
    /// Routines it started.
    pub routines: i32,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct EventRoutine {
    pub id: Uuid,
    pub name: String,
    pub agent: Uuid,
    pub agent_name: String,
    pub event: String,
    pub enabled: bool,
}

/// A connection as the orchestrator uses it.
struct Stored {
    id: Uuid,
    connector: &'static dyn Connector,
    auth: String,
    config: Value,
    sealed: Sealed,
    /// Unix seconds.
    expires_at: Option<i64>,
}

pub(super) fn connector(id: &str) -> Result<&'static dyn Connector, ConnectError> {
    cubebot_connectors::find(id).ok_or_else(|| ConnectError::Invalid(format!("no connector {id}")))
}

fn method<'a>(manifest: &'a Manifest, kind: &str) -> Result<&'a Auth, ConnectError> {
    manifest
        .auth
        .iter()
        .find(|auth| auth.kind() == kind)
        .ok_or_else(|| {
            ConnectError::Invalid(format!("{} does not sign in with {kind}", manifest.name))
        })
}

/// The settings a person entered, checked, with the orchestrator's own kept from `kept`.
fn settings(
    manifest: &Manifest,
    auth: &Auth,
    mut config: Map<String, Value>,
    secrets: &BTreeMap<String, String>,
    kept: &Value,
) -> Result<Value, ConnectError> {
    config.retain(|key, value| !MANAGED.contains(&key.as_str()) && !value.is_null());
    if let Some(problem) = cubebot_connectors::problem(manifest, auth, &config, secrets) {
        return Err(ConnectError::Invalid(problem));
    }
    for key in MANAGED {
        if let Some(value) = kept.get(key) {
            config.insert(key.into(), value.clone());
        }
    }
    Ok(Value::Object(config))
}

fn slug(slug: &str) -> Result<(), ConnectError> {
    cubebot_connectors::slug_valid(slug)
        .then_some(())
        .ok_or_else(|| {
            ConnectError::Invalid("a name is 1 to 32 lowercase letters, digits or hyphens".into())
        })
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

/// A vendor's refusal as the console words it.
fn refusal(error: &cubebot_connectors::Error) -> String {
    match error {
        cubebot_connectors::Error::Unauthorized => "The account refused the credential.".into(),
        error => {
            let mut text = error.to_string();
            text.truncate(1024);
            text
        }
    }
}

impl Connections {
    pub fn catalog(&self, origin: Option<&str>) -> Catalog {
        Catalog {
            connectors: cubebot_connectors::all()
                .iter()
                .map(|connector| connector.manifest())
                .collect(),
            loopback_redirect: LOOPBACK.into(),
            callback_redirect: origin.map(callback),
        }
    }

    async fn seal_connection<'e>(
        &self,
        db: impl sqlx::PgExecutor<'e>,
        id: Uuid,
        sealed: &Sealed,
    ) -> Result<Vec<u8>, IdentityError> {
        let context = sealing_context(db, "connection", id).await?;
        let plain =
            serde_json::to_vec(sealed).map_err(|error| IdentityError::Invalid(error.into()))?;
        self.credentials
            .encrypt(&plain, &context)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))
    }

    async fn open_connection<'e>(
        &self,
        db: impl sqlx::PgExecutor<'e>,
        id: Uuid,
        sealed: &[u8],
    ) -> Result<Sealed, IdentityError> {
        let context = sealing_context(db, "connection", id).await?;
        let plain = self
            .credentials
            .decrypt(sealed, &context)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
        // A parse failure never shows the plaintext.
        serde_json::from_slice(&plain).map_err(|_| IdentityError::CredentialUnavailable(None))
    }

    /// The vendor's own context, or the loopback one a test controls.
    fn context(&self, config: Value, credential: Credential) -> Context {
        let cx = Context::new(config, credential);
        match self.vendor() {
            Some(vendor) => cx.with_origin(vendor.as_str()),
            None => cx,
        }
    }

    #[tracing::instrument(skip_all)]
    pub async fn connections(&self) -> Result<Vec<ConnectionSummary>, IdentityError> {
        self.summaries(None).await
    }

    async fn summaries(&self, one: Option<Uuid>) -> Result<Vec<ConnectionSummary>, IdentityError> {
        Ok(sqlx::query_as!(
            ConnectionSummary,
            r#"
            SELECT id, connector, slug, auth, account, config AS "config!: _", access AS "access!: Access",
                level, disabled_tools AS "disabled_tools!: _", status, problem,
                (to_jsonb(checked_at) #>> '{}') AS checked_at, (to_jsonb(expires_at) #>> '{}') AS expires_at,
                (to_jsonb(polled_at) #>> '{}') AS polled_at
            FROM connections
            WHERE $1::uuid IS NULL OR id = $1
            ORDER BY connector, slug
            "#,
            one,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn connection_summary(&self, id: Uuid) -> Result<ConnectionSummary, IdentityError> {
        self.summaries(Some(id))
            .await?
            .pop()
            .ok_or(IdentityError::NotFound)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor, connector = %input.connector))]
    pub async fn create_connection(
        &self,
        actor: Uuid,
        input: SaveConnection,
    ) -> Result<ConnectionSummary, ConnectError> {
        let connector = connector(&input.connector)?;
        let manifest = connector.manifest();
        let auth = method(manifest, &input.auth)?;
        if matches!(auth, Auth::OAuth2(_) | Auth::GithubApp { .. }) {
            return Err(ConnectError::Invalid(
                "this sign-in goes through the vendor".into(),
            ));
        }
        slug(&input.slug)?;
        let mut config = settings(manifest, auth, input.config, &input.secrets, &Value::Null)?;
        if let Some(linked) = input.linked {
            self.linkable(auth, linked).await?;
            config["linked"] = Value::String(linked.to_string());
        } else if matches!(auth, Auth::Linked { .. }) {
            return Err(ConnectError::Invalid(
                "choose the connection to sign in with".into(),
            ));
        }
        let mut sealed = Sealed {
            fields: input.secrets,
            ..Sealed::default()
        };
        let (credential, expires_in) = self.first(connector, auth, &config, &mut sealed).await?;
        let account = self.check(connector, &config, credential).await?;
        let draft = Draft {
            connector,
            slug: input.slug,
            auth: auth.kind(),
            config,
            access: input.access.unwrap_or_default(),
            level: input.level.unwrap_or(manifest.level),
        };
        self.insert(actor, draft, &sealed, (Some(account), expires_in, None))
            .await
    }

    /// The credential a new connection is checked with, and how long it lasts; an app's own
    /// token is kept in `sealed`.
    async fn first(
        &self,
        connector: &dyn Connector,
        auth: &Auth,
        config: &Value,
        sealed: &mut Sealed,
    ) -> Result<(Credential, Option<i64>), ConnectError> {
        let fields = sealed.fields.clone();
        match auth {
            Auth::Linked { .. } => return Ok((self.linked(config).await?.0, None)),
            Auth::ClientCredentials(_) => (),
            _ => return Ok((Credential::Fields { fields }, None)),
        }
        let (token, expires_in) = self.granted(connector, config, sealed).await.map_err(
            |(tend::Renewal::Refused(problem) | tend::Renewal::Failing(problem))| {
                ConnectError::Refused(problem)
            },
        )?;
        sealed.access_token = Some(token.clone());
        Ok((Credential::Token { token }, Some(expires_in)))
    }

    /// Proves a credential and names its account; a refusal is the person's to read.
    async fn check(
        &self,
        connector: &dyn Connector,
        config: &Value,
        credential: Credential,
    ) -> Result<String, ConnectError> {
        let cx = self.context(config.clone(), credential);
        let account = connector.check(&cx).await.map_err(|error| {
            tracing::info!(connector = connector.manifest().id, %error, "connection check refused");
            match error {
                cubebot_connectors::Error::Unauthorized => ConnectError::SignInAgain,
                error => ConnectError::Refused(refusal(&error)),
            }
        })?;
        super::subscriptions::account(&account.name)
            .ok_or_else(|| ConnectError::Refused("the vendor named no account".into()))
    }

    /// Saves a new connection whose credential was just proven.
    async fn insert(
        &self,
        actor: Uuid,
        draft: Draft,
        sealed: &Sealed,
        (account, expires_in, problem): (Option<String>, Option<i64>, Option<String>),
    ) -> Result<ConnectionSummary, ConnectError> {
        let mut tx = self.pool.begin().await?;
        validate_access(&mut tx, &draft.access).await?;
        let id = Uuid::new_v4();
        let credential = self.seal_connection(&mut *tx, id, sealed).await?;
        let row = (account, expires_in, problem);
        let inserted = insert_row(&mut tx, (id, actor), &draft, credential, row)
            .await?
            .ok_or(IdentityError::Conflict)?;
        tx.commit().await?;
        Ok(self.connection_summary(inserted).await?)
    }

    /// A linked sign-in borrows a connection of a connector it names, with the product it needs.
    async fn linkable(&self, auth: &Auth, linked: Uuid) -> Result<(), ConnectError> {
        let Auth::Linked {
            connectors,
            product,
        } = auth
        else {
            return Err(ConnectError::Invalid("this sign-in does not link".into()));
        };
        let row = sqlx::query!(
            "SELECT connector, auth, config FROM connections WHERE id = $1",
            linked
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| ConnectError::Invalid("that connection no longer exists".into()))?;
        let other = connector(&row.connector)?.manifest();
        let fits = connectors.contains(&row.connector.as_str())
            && row.auth != "linked"
            && other.enabled(&row.config).contains(product);
        fits.then_some(()).ok_or_else(|| {
            ConnectError::Invalid(format!("choose a connection with {product} switched on"))
        })
    }

    #[tracing::instrument(skip_all, fields(connection = %id))]
    pub async fn update_connection(
        &self,
        id: Uuid,
        input: UpdateConnection,
    ) -> Result<ConnectionSummary, ConnectError> {
        let mut tx = self.pool.begin().await?;
        let mut stored = self.stored(&mut tx, id, true).await?;
        let disabled = changed(&mut stored, &input)?;
        if let Some(access) = &input.access {
            validate_access(&mut tx, access).await?;
        }
        let credential = self.seal_connection(&mut *tx, id, &stored.sealed).await?;
        let changes = (&stored.config, credential, disabled);
        if !updated(&mut tx, id, &input, changes).await? {
            return Err(IdentityError::Conflict.into());
        }
        tx.commit().await?;
        if input.config.is_some() || input.secrets.is_some() {
            self.check_connection(id).await?;
        }
        Ok(self.connection_summary(id).await?)
    }

    /// Forgets the connection, then asks the vendor to revoke an OAuth sign-in. Returns its slug.
    #[tracing::instrument(skip_all, fields(connection = %id))]
    pub async fn delete_connection(&self, id: Uuid) -> Result<String, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let stored = self.stored(&mut tx, id, true).await;
        let slug = sqlx::query_scalar!("DELETE FROM connections WHERE id = $1 RETURNING slug", id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(IdentityError::NotFound)?;
        tx.commit().await?;
        if let Ok(stored) = stored {
            self.revoke(&stored).await;
        }
        Ok(slug)
    }

    /// Runs the connector's check with a current credential and records what it found.
    #[tracing::instrument(skip_all, fields(connection = %id))]
    pub async fn check_connection(&self, id: Uuid) -> Result<ConnectionSummary, IdentityError> {
        let checked = match self.fresh_credential(id).await {
            Ok((credential, _)) => {
                let mut tx = self.pool.begin().await?;
                let stored = self.stored(&mut tx, id, false).await?;
                tx.commit().await?;
                self.check(stored.connector, &stored.config, credential)
                    .await
            }
            Err(ConnectError::Identity(error)) => return Err(error),
            Err(refused) => Err(refused),
        };
        let (status, account, problem) = match &checked {
            Ok(account) => ("connected", Some(account.as_str()), None),
            Err(ConnectError::SignInAgain) => (
                "sign_in_again",
                None,
                Some("The account refused the credential; sign in again."),
            ),
            Err(ConnectError::Refused(problem) | ConnectError::Invalid(problem)) => {
                ("error", None, Some(problem.as_str()))
            }
            Err(ConnectError::Identity(_)) => {
                ("error", None, Some("The credential could not be opened."))
            }
        };
        sqlx::query!(
            r#"
            UPDATE connections
            SET status = $2, account = COALESCE($3, account), problem = $4, checked_at = clock_timestamp()
            WHERE id = $1
            "#,
            id,
            status,
            account,
            problem,
        )
        .execute(&self.pool)
        .await?;
        self.connection_summary(id).await
    }

    async fn stored(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: Uuid,
        lock: bool,
    ) -> Result<Stored, IdentityError> {
        let row = sqlx::query!(
            r#"
            SELECT connector, auth, config, credential, extract(epoch FROM expires_at)::bigint AS expires_at
            FROM connections WHERE id = $1
            "#,
            id,
        )
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        if lock {
            sqlx::query!("SELECT id FROM connections WHERE id = $1 FOR UPDATE", id)
                .fetch_one(&mut **tx)
                .await?;
        }
        Ok(Stored {
            id,
            connector: cubebot_connectors::find(&row.connector).ok_or(IdentityError::NotFound)?,
            auth: row.auth,
            config: row.config,
            sealed: self.open_connection(&mut **tx, id, &row.credential).await?,
            expires_at: row.expires_at,
        })
    }

    /// A current credential for the connection and when it expires, renewing it first when due.
    async fn fresh_credential(&self, id: Uuid) -> Result<(Credential, Option<i64>), ConnectError> {
        let mut tx = self.pool.begin().await?;
        let stored = self.stored(&mut tx, id, false).await?;
        tx.commit().await?;
        match stored.auth.as_str() {
            "api_key" | "password" | "public" => Ok((
                Credential::Fields {
                    fields: stored.sealed.fields,
                },
                None,
            )),
            "linked" => self.linked(&stored.config).await,
            _ => self.token(stored).await,
        }
    }

    /// The access token of an OAuth or GitHub App connection, renewed when it is close to expiry.
    async fn token(&self, stored: Stored) -> Result<(Credential, Option<i64>), ConnectError> {
        let fresh = stored.expires_at.is_some_and(|at| at - now() > SPARE);
        let (token, expires_at) = match (fresh, stored.sealed.access_token) {
            (true, Some(token)) => (token, stored.expires_at),
            _ => {
                self.renew(stored.id).await?;
                let mut tx = self.pool.begin().await?;
                let renewed = self.stored(&mut tx, stored.id, false).await?;
                tx.commit().await?;
                let token = renewed
                    .sealed
                    .access_token
                    .filter(|_| renewed.expires_at.is_some_and(|at| at - now() > SPARE));
                let Some(token) = token else {
                    return Err(self.unrenewed(stored.id).await?);
                };
                (token, renewed.expires_at)
            }
        };
        Ok((Credential::Token { token }, expires_at))
    }

    /// Why no token came of a renewal: the vendor refused the saved sign-in, or it is retried.
    async fn unrenewed(&self, id: Uuid) -> Result<ConnectError, IdentityError> {
        let status = sqlx::query_scalar!("SELECT status FROM connections WHERE id = $1", id)
            .fetch_one(&self.pool)
            .await?;
        Ok(match status.as_str() {
            "sign_in_again" => ConnectError::SignInAgain,
            _ => ConnectError::Refused("the sign-in could not be renewed".into()),
        })
    }

    /// The access token of the connection a linked sign-in borrows.
    async fn linked(&self, config: &Value) -> Result<(Credential, Option<i64>), ConnectError> {
        let linked = config["linked"]
            .as_str()
            .and_then(|id| id.parse::<Uuid>().ok())
            .ok_or_else(|| ConnectError::Refused("the linked connection is gone".into()))?;
        let mut tx = self.pool.begin().await?;
        let stored = self
            .stored(&mut tx, linked, false)
            .await
            .map_err(|_| ConnectError::Refused("the linked connection is gone".into()))?;
        tx.commit().await?;
        if !matches!(stored.auth.as_str(), "oauth2" | "github_app") {
            return Err(ConnectError::Refused(
                "the linked connection has no access token".into(),
            ));
        }
        self.token(stored).await
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub async fn agent_connections(
        &self,
        agent: AgentId,
    ) -> Result<Vec<AgentConnection>, IdentityError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, connector, slug, account, level, status, config,
                access->>'mode' = 'all_agents' AS "all_agents!",
                (access->>'mode' = 'all_agents' OR access->'agents' @> jsonb_build_array($1::uuid)) AS "enabled!"
            FROM connections
            ORDER BY connector, slug
            "#,
            agent.0,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| AgentConnection {
                events: events(&row.connector, &row.config),
                id: row.id,
                connector: row.connector,
                slug: row.slug,
                account: row.account,
                level: row.level,
                status: row.status,
                all_agents: row.all_agents,
                enabled: row.enabled,
            })
            .collect())
    }

    /// Grants or withdraws one agent's use of a connection with explicit grants.
    #[tracing::instrument(skip_all, fields(connection = %id, agent = %agent.0))]
    pub async fn grant_connection(
        &self,
        id: Uuid,
        agent: AgentId,
        enabled: bool,
    ) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        let access = ProviderAccess::ExplicitGrants {
            agents: vec![agent.0],
        };
        validate_access(&mut tx, &access).await?;
        let mode = sqlx::query_scalar!(
            r#"SELECT access->>'mode' AS "mode!" FROM connections WHERE id = $1 FOR UPDATE"#,
            id,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        if mode != "explicit_grants" {
            return Err(IdentityError::Conflict);
        }
        sqlx::query!(
            r#"
            UPDATE connections
            SET access = jsonb_build_object('mode', 'explicit_grants', 'agents', (
                    SELECT coalesce(jsonb_agg(DISTINCT granted), '[]')
                    FROM jsonb_array_elements(access->'agents' || jsonb_build_array($2::uuid)) granted
                    WHERE $3 OR granted <> to_jsonb($2::uuid)
                )),
                updated_at = clock_timestamp()
            WHERE id = $1
            "#,
            id,
            agent.0,
            enabled,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The connections `agent` may use, for one turn; no credentials.
    pub(crate) async fn runtime_connections(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        agent: AgentId,
    ) -> Result<Vec<Connection>, IdentityError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, connector, slug, account, config, level, disabled_tools AS "disabled_tools!: sqlx::types::Json<Vec<String>>"
            FROM connections
            WHERE access->>'mode' = 'all_agents' OR access->'agents' @> jsonb_build_array($1::uuid)
            ORDER BY slug
            "#,
            agent.0,
        )
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .into_iter()
            .filter(|row| cubebot_connectors::find(&row.connector).is_some())
            .map(|row| Connection {
                id: row.id,
                connector: row.connector,
                slug: row.slug,
                account: row.account,
                config: row.config,
                level: row.level,
                disabled_tools: row.disabled_tools.0,
            })
            .collect())
    }

    /// The credential an agent's runtime asks for. Access is checked again here, so a revoked
    /// grant takes effect at the next call. Never log it.
    #[tracing::instrument(skip_all, fields(agent = %agent.0, connection = %id))]
    pub async fn runtime_credential(
        &self,
        agent: AgentId,
        id: Uuid,
    ) -> Result<ConnectionCredential, ConnectError> {
        let status = sqlx::query_scalar!(
            r#"
            SELECT status FROM connections
            WHERE id = $1 AND (access->>'mode' = 'all_agents' OR access->'agents' @> jsonb_build_array($2::uuid))
            "#,
            id,
            agent.0,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::NotFound)?;
        if status == "sign_in_again" {
            return Err(ConnectError::Refused(
                "the connection needs an administrator to sign in again".into(),
            ));
        }
        let (credential, expires_at) = self.fresh_credential(id).await?;
        let credential = serde_json::to_value(&credential)
            .map_err(|error| IdentityError::Invalid(error.into()))?;
        Ok(ConnectionCredential {
            credential,
            expires_at,
        })
    }

    /// Recent events and the routines they start.
    pub async fn connection_activity(&self, id: Uuid) -> Result<ConnectionActivity, IdentityError> {
        let events = sqlx::query_as!(
            ConnectionEvent,
            r#"
            SELECT kind, summary, (to_jsonb(received_at) #>> '{}') AS "received_at!", routines
            FROM connection_events
            WHERE connection_id = $1
            ORDER BY received_at DESC
            LIMIT 20
            "#,
            id,
        )
        .fetch_all(&self.pool)
        .await?;
        let routines = sqlx::query_as!(
            EventRoutine,
            r#"
            SELECT r.id, r.name, r.agent_id AS agent, a.name AS agent_name, r.trigger_event AS "event!", r.enabled
            FROM routines r JOIN agents a ON a.id = r.agent_id
            WHERE r.trigger_connection = $1
            ORDER BY a.name, r.name
            "#,
            id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(ConnectionActivity { events, routines })
    }
}

/// What a sign-in will save once the vendor proves it.
pub(super) struct Draft {
    connector: &'static dyn Connector,
    slug: String,
    auth: &'static str,
    config: Value,
    access: ProviderAccess,
    level: Level,
}

/// Applies an update's settings, secrets and switched-off tools to `stored`, checked against the
/// manifest; answers the switched-off tools to save.
fn changed(
    stored: &mut Stored,
    input: &UpdateConnection,
) -> Result<Option<Vec<String>>, ConnectError> {
    let manifest = stored.connector.manifest();
    let auth = method(manifest, &stored.auth)?;
    let app = matches!(auth, Auth::GithubApp { .. });
    if let Some(secrets) = &input.secrets {
        if app {
            return Err(ConnectError::Invalid(
                "a GitHub App's key comes from GitHub; sign in again".into(),
            ));
        }
        stored.sealed.fields.extend(secrets.clone());
    }
    if input.config.is_some() || input.secrets.is_some() {
        let mut config = input
            .config
            .clone()
            .unwrap_or_else(|| stored.config.as_object().cloned().unwrap_or_default());
        config.retain(|key, _| !MANAGED.contains(&key.as_str()));
        let secrets = if app {
            BTreeMap::new()
        } else {
            stored.sealed.fields.clone()
        };
        stored.config = settings(manifest, auth, config, &secrets, &stored.config)?;
        // An app's token was granted to the old secret or settings; the next use asks again.
        if matches!(auth, Auth::ClientCredentials(_)) {
            stored.sealed.access_token = None;
        }
    }
    if let Some(name) = &input.slug {
        slug(name)?;
    }
    input
        .disabled_tools
        .clone()
        .map(|tools| tools_known(manifest, tools))
        .transpose()
}

/// Saves an update; false when another connection has the slug.
async fn updated(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    input: &UpdateConnection,
    (config, credential, disabled): (&Value, Vec<u8>, Option<Vec<String>>),
) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        r#"
        UPDATE connections
        SET slug = COALESCE($2, slug), config = $3, credential = $4, access = COALESCE($5, access),
            level = COALESCE($6, level), disabled_tools = COALESCE($7, disabled_tools),
            updated_at = clock_timestamp()
        WHERE id = $1 AND NOT EXISTS(SELECT 1 FROM connections o WHERE o.slug = $2 AND o.id <> $1)
        "#,
        id,
        input.slug,
        config,
        credential,
        input.access.as_ref().map(sqlx::types::Json) as _,
        input.level.map(Level::as_str),
        disabled.map(sqlx::types::Json) as _,
    )
    .execute(&mut **tx)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// The new row, unless its slug is taken.
async fn insert_row(
    tx: &mut Transaction<'_, Postgres>,
    (id, actor): (Uuid, Uuid),
    draft: &Draft,
    credential: Vec<u8>,
    (account, expires_in, problem): (Option<String>, Option<i64>, Option<String>),
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"
        INSERT INTO connections (id, connector, slug, auth, account, config, credential, access, level,
            checked_at, created_by, expires_at, refresh_after, status, problem)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, clock_timestamp(), $10,
            clock_timestamp() + make_interval(secs => $11),
            clock_timestamp() + make_interval(secs => $11 - LEAST(600, $11 / 2)),
            CASE WHEN $12::text IS NULL THEN 'connected' ELSE 'error' END, $12)
        ON CONFLICT (slug) DO NOTHING
        RETURNING id
        "#,
        id,
        draft.connector.manifest().id,
        draft.slug,
        draft.auth,
        account,
        draft.config,
        credential,
        sqlx::types::Json(&draft.access) as _,
        draft.level.as_str(),
        actor,
        expires_in.map(|seconds| seconds as f64),
        problem,
    )
    .fetch_optional(&mut **tx)
    .await
}

/// The event kinds a connection reports, given the products it has switched on.
fn events(connector: &str, config: &Value) -> Vec<String> {
    let Some(manifest) = cubebot_connectors::find(connector).map(|c| c.manifest()) else {
        return Vec::new();
    };
    let enabled = manifest.enabled(config);
    manifest
        .events
        .iter()
        .filter(|event| {
            event
                .product
                .is_none_or(|product| enabled.contains(&product))
        })
        .map(|event| event.kind.to_owned())
        .collect()
}

fn tools_known(manifest: &Manifest, tools: Vec<String>) -> Result<Vec<String>, ConnectError> {
    let known = tools.len() <= manifest.tools.len()
        && tools.iter().all(|name| manifest.tool(name).is_some());
    known
        .then_some(tools)
        .ok_or_else(|| ConnectError::Invalid("switch off only this connector's tools".into()))
}

/// This installation's own OAuth callback.
pub fn callback(origin: &str) -> String {
    format!("{}/api/connections/callback", origin.trim_end_matches('/'))
}

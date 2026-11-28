//! Mirrors linked Google Workspace groups into Cubebot groups. A service account with
//! domain-wide delegation reads the directory as one Workspace administrator; members match
//! accounts by the verified email of their single sign-on identity.

use std::time::Duration;

use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sqlx::PgPool;
use uuid::Uuid;

use super::{IdentityError, Receipt, commands::Command, groups};
use crate::connections::Credentials;

const CONTEXT: &[u8] = b"cubebot.identity.google";
const SCOPE: &str = "https://www.googleapis.com/auth/admin.directory.group.readonly";
/// Pages read per listing, so a misbehaving API cannot keep a sync going forever.
const MAX_PAGES: usize = 50;
const PERIOD: Duration = Duration::from_secs(15 * 60);

#[derive(Clone)]
pub struct GoogleDirectory {
    pool: PgPool,
    credentials: Option<Credentials>,
    client: reqwest::Client,
    token_url: String,
    api_base: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct GoogleSettings {
    pub enabled: bool,
    pub admin_email: Option<String>,
    pub customer: String,
    /// The stored service account; its key is never returned.
    pub client_email: Option<String>,
    pub has_key: bool,
    pub synced_at_unix: Option<i64>,
    pub problem: Option<String>,
    /// A credential key is configured, so a service account key can be stored.
    pub can_store_key: bool,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveGoogle {
    pub request_id: Uuid,
    pub enabled: bool,
    /// The Workspace administrator the service account acts as.
    #[serde(default)]
    pub admin_email: Option<String>,
    /// `my_customer` when left out.
    #[serde(default)]
    pub customer: Option<String>,
    /// The service account's JSON key; left out keeps the stored one.
    #[serde(default)]
    pub service_account: Option<String>,
}

/// What a save records: never the key, only which service account it belongs to.
#[derive(Serialize)]
struct Audited<'a> {
    enabled: bool,
    admin_email: Option<&'a str>,
    customer: &'a str,
    new_service_account: Option<&'a str>,
}

#[derive(Debug, Default, Serialize, utoipa::ToSchema)]
pub struct SyncOutcome {
    /// Groups whose members were mirrored.
    pub groups: usize,
    /// Memberships set across those groups.
    pub members: usize,
    pub problem: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct WorkspaceGroup {
    pub email: String,
    pub name: String,
    pub direct_members_count: i64,
}

#[derive(Deserialize)]
struct ServiceAccount {
    #[serde(rename = "type")]
    kind: String,
    client_email: String,
    private_key: String,
}

impl ServiceAccount {
    fn parse(json: &[u8]) -> Result<Self, IdentityError> {
        let account: Self =
            serde_json::from_slice(json).map_err(|error| IdentityError::Invalid(error.into()))?;
        if account.kind != "service_account" || !plausible_email(&account.client_email) {
            return Err(IdentityError::InvalidInput);
        }
        account.key_pair()?;
        Ok(account)
    }

    fn key_pair(&self) -> Result<RsaKeyPair, IdentityError> {
        let body: String = self
            .private_key
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .map(str::trim)
            .collect();
        let der = STANDARD
            .decode(body)
            .map_err(|error| IdentityError::Invalid(error.into()))?;
        // ring's errors are descriptions only, without `std::error::Error`.
        RsaKeyPair::from_pkcs8(&der).map_err(|error| {
            IdentityError::Invalid(format!("private key rejected: {error}").into())
        })
    }

    /// The RFC 7523 assertion that asks for a token acting as `subject`.
    fn assertion(&self, subject: &str, audience: &str, now: u64) -> Result<String, IdentityError> {
        let key = self.key_pair()?;
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let claims = serde_json::json!({
            "iss": self.client_email,
            "sub": subject,
            "scope": SCOPE,
            "aud": audience,
            "iat": now,
            "exp": now + 3600,
        });
        let message = format!("{header}.{}", URL_SAFE_NO_PAD.encode(claims.to_string()));
        let mut signature = vec![0; key.public().modulus_len()];
        key.sign(
            &RSA_PKCS1_SHA256,
            &ring::rand::SystemRandom::new(),
            message.as_bytes(),
            &mut signature,
        )
        .map_err(|error| unavailable(format!("signing the assertion failed: {error}")))?;
        Ok(format!("{message}.{}", URL_SAFE_NO_PAD.encode(signature)))
    }
}

struct Config {
    enabled: bool,
    admin_email: String,
    customer: String,
    account: ServiceAccount,
}

/// Google refused a request; its error body names why and carries no secret.
#[derive(Debug)]
struct Refused {
    status: reqwest::StatusCode,
    body: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Google answered {}: {}", self.status, self.body)
    }
}

impl std::error::Error for Refused {}

fn unavailable(error: impl Into<super::Source>) -> IdentityError {
    IdentityError::OidcUnavailable(error.into())
}

async fn checked(response: reqwest::Response) -> Result<reqwest::Response, IdentityError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    Err(unavailable(Refused {
        status,
        body: body.chars().take(300).collect(),
    }))
}

/// One page of a Directory API listing, of members or of groups.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", bound(deserialize = "T: Deserialize<'de>"))]
struct Page<T> {
    #[serde(default, rename = "members", alias = "groups")]
    items: Vec<T>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct Member {
    email: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    status: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listed {
    email: String,
    #[serde(default)]
    name: String,
    /// The API sends this int64 as a string.
    #[serde(default)]
    direct_members_count: Option<String>,
}

fn plausible_email(text: &str) -> bool {
    text.contains('@') && text.len() <= 320 && !text.chars().any(char::is_control)
}

/// The error's own cause when it has one: "OIDC unavailable" alone tells an administrator nothing.
fn describe(error: &IdentityError) -> String {
    std::error::Error::source(error).map_or_else(|| error.to_string(), ToString::to_string)
}

/// The problem column holds at most 2000 bytes.
fn bounded(mut text: String) -> String {
    let mut end = text.len().min(2000);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

impl GoogleDirectory {
    pub fn new(pool: PgPool, credentials: Option<Credentials>) -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .build()
            // Building fails only without a TLS backend, which the default client needs too.
            .unwrap_or_default();
        Self {
            pool,
            credentials,
            client,
            token_url: "https://oauth2.googleapis.com/token".into(),
            api_base: "https://admin.googleapis.com/admin/directory/v1".into(),
        }
    }

    /// Points at another token endpoint and Directory API, such as a local stand-in.
    pub fn with_endpoints(
        mut self,
        token_url: impl Into<String>,
        api_base: impl Into<String>,
    ) -> Self {
        self.token_url = token_url.into();
        self.api_base = api_base.into();
        self
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn settings(&self, actor: &super::Actor) -> Result<GoogleSettings, IdentityError> {
        super::overseer(&self.pool, actor.id).await?;
        let row = sqlx::query!(
            r#"
            SELECT enabled, admin_email, customer, client_email,
                   service_account IS NOT NULL AS "has_key!",
                   EXTRACT(EPOCH FROM synced_at)::bigint AS synced_at_unix, problem
            FROM google_directory WHERE singleton
            "#,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(GoogleSettings {
            enabled: row.enabled,
            admin_email: row.admin_email,
            customer: row.customer,
            client_email: row.client_email,
            has_key: row.has_key,
            synced_at_unix: row.synced_at_unix,
            problem: row.problem,
            can_store_key: self.credentials.is_some(),
        })
    }

    /// Enabling needs an administrator to act as and a key, given now or stored before.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %input.request_id))]
    pub async fn save(
        &self,
        actor: &super::Actor,
        input: &SaveGoogle,
    ) -> Result<Receipt, IdentityError> {
        // Before reading the key, so only administrators learn whether it is valid.
        super::administrator(&self.pool, actor.id).await?;
        let admin_email = input
            .admin_email
            .as_deref()
            .map(str::trim)
            .filter(|email| !email.is_empty())
            .map(str::to_ascii_lowercase);
        let customer = input
            .customer
            .as_deref()
            .map(str::trim)
            .filter(|customer| !customer.is_empty())
            .unwrap_or("my_customer");
        if admin_email
            .as_deref()
            .is_some_and(|email| !plausible_email(email))
            || customer.len() > 64
            || customer.chars().any(char::is_control)
        {
            return Err(IdentityError::InvalidInput);
        }
        let key = input
            .service_account
            .as_deref()
            .map(|json| self.seal(json))
            .transpose()?;
        let audited = Audited {
            enabled: input.enabled,
            admin_email: admin_email.as_deref(),
            customer,
            new_service_account: key.as_ref().map(|(email, _)| email.as_str()),
        };
        let command = Command::new(actor, "google.save", input.request_id, &audited)?;
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        let (client_email, sealed) = key.unzip();
        let revision = sqlx::query_scalar!(
            r#"
            UPDATE google_directory
            SET enabled = $1, admin_email = $2, customer = $3,
                service_account = COALESCE($4, service_account),
                client_email = COALESCE($5, client_email), revision = revision + 1
            WHERE singleton
              AND (NOT $1 OR ($2::text IS NOT NULL AND ($4::bytea IS NOT NULL OR service_account IS NOT NULL)))
            RETURNING revision
            "#,
            input.enabled,
            admin_email,
            customer,
            sealed,
            client_email,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IdentityError::InvalidInput)?;
        let id = sqlx::query_scalar!("SELECT id FROM installation WHERE singleton")
            .fetch_one(&mut *transaction)
            .await?;
        command.commit(transaction, Receipt { id, revision }).await
    }

    /// Validates a JSON key and seals it; returns its service account's address too.
    fn seal(&self, json: &str) -> Result<(String, Vec<u8>), IdentityError> {
        let credentials = self
            .credentials
            .as_ref()
            .ok_or(IdentityError::CredentialUnavailable(None))?;
        let account = ServiceAccount::parse(json.as_bytes())?;
        let sealed = credentials
            .encrypt(json.as_bytes(), CONTEXT)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
        Ok((account.client_email, sealed))
    }

    /// Mirrors every linked group now, enabled or not; `Conflict` until a key and administrator
    /// are saved.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn sync_now(&self, actor: &super::Actor) -> Result<SyncOutcome, IdentityError> {
        super::administrator(&self.pool, actor.id).await?;
        let config = self.config().await?.ok_or(IdentityError::Conflict)?;
        self.sync(&config).await
    }

    /// The Workspace's groups, for choosing which to link.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn workspace_groups(
        &self,
        actor: &super::Actor,
    ) -> Result<Vec<WorkspaceGroup>, IdentityError> {
        super::administrator(&self.pool, actor.id).await?;
        let config = self.config().await?.ok_or(IdentityError::Conflict)?;
        let token = self.token(&config).await?;
        let mut url = self.url(&["groups"])?;
        url.query_pairs_mut()
            .append_pair("customer", &config.customer)
            .append_pair("maxResults", "200");
        let listed: Vec<Listed> = self.list(&token, url).await?;
        Ok(listed
            .into_iter()
            .map(|group| WorkspaceGroup {
                email: group.email.to_ascii_lowercase(),
                name: group.name,
                direct_members_count: group
                    .direct_members_count
                    .and_then(|count| count.parse().ok())
                    .unwrap_or(0),
            })
            .collect())
    }

    /// The saved settings with the key unsealed; `None` until a key and administrator are saved.
    async fn config(&self) -> Result<Option<Config>, IdentityError> {
        let row = sqlx::query!(
            "SELECT enabled, admin_email, customer, service_account FROM google_directory WHERE singleton",
        )
        .fetch_one(&self.pool)
        .await?;
        let (Some(admin_email), Some(sealed)) = (row.admin_email, row.service_account) else {
            return Ok(None);
        };
        let credentials = self
            .credentials
            .as_ref()
            .ok_or(IdentityError::CredentialUnavailable(None))?;
        let json = credentials
            .decrypt(&sealed, CONTEXT)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
        Ok(Some(Config {
            enabled: row.enabled,
            admin_email,
            customer: row.customer,
            account: ServiceAccount::parse(&json)?,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn token(&self, config: &Config) -> Result<String, IdentityError> {
        #[derive(Deserialize)]
        struct Token {
            access_token: String,
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let assertion = config
            .account
            .assertion(&config.admin_email, &self.token_url, now)?;
        let response = self
            .client
            .post(&self.token_url)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &assertion),
            ])
            .send()
            .await
            .map_err(unavailable)?;
        let token: Token = checked(response).await?.json().await.map_err(unavailable)?;
        Ok(token.access_token)
    }

    fn url(&self, segments: &[&str]) -> Result<reqwest::Url, IdentityError> {
        let mut url = reqwest::Url::parse(&self.api_base).map_err(unavailable)?;
        url.path_segments_mut()
            .map_err(|()| unavailable("the Directory API address cannot take a path"))?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    /// Every page of a listing; more than `MAX_PAGES` fails rather than mirroring a partial list.
    async fn list<T: DeserializeOwned>(
        &self,
        token: &str,
        url: reqwest::Url,
    ) -> Result<Vec<T>, IdentityError> {
        let mut items = Vec::new();
        let mut next: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut request = self.client.get(url.clone()).bearer_auth(token);
            if let Some(page) = &next {
                request = request.query(&[("pageToken", page)]);
            }
            let response = request.send().await.map_err(unavailable)?;
            let page: Page<T> = checked(response).await?.json().await.map_err(unavailable)?;
            items.extend(page.items);
            next = page.next_page_token.filter(|token| !token.is_empty());
            if next.is_none() {
                return Ok(items);
            }
        }
        Err(unavailable("the listing ran past its page limit"))
    }

    /// Records how the sync went, so the settings show it.
    #[tracing::instrument(skip_all)]
    async fn sync(&self, config: &Config) -> Result<SyncOutcome, IdentityError> {
        let outcome = match self.token(config).await {
            Ok(token) => self.sync_groups(&token).await?,
            Err(error) => SyncOutcome {
                problem: Some(bounded(describe(&error))),
                ..SyncOutcome::default()
            },
        };
        sqlx::query!(
            "UPDATE google_directory SET synced_at = clock_timestamp(), problem = $1 WHERE singleton",
            outcome.problem,
        )
        .execute(&self.pool)
        .await?;
        Ok(outcome)
    }

    /// A group Google cannot list keeps its members and is named in the problem.
    async fn sync_groups(&self, token: &str) -> Result<SyncOutcome, IdentityError> {
        let linked = sqlx::query!(
            r#"SELECT id, external_id AS "external_id!" FROM human_teams WHERE active AND source = 'google' ORDER BY external_id"#,
        )
        .fetch_all(&self.pool)
        .await?;
        let mut outcome = SyncOutcome::default();
        let mut problems = Vec::new();
        for group in linked {
            match self.members(token, &group.external_id).await {
                Ok(emails) => {
                    outcome.members += self.mirror(group.id, &emails).await?;
                    outcome.groups += 1;
                }
                Err(error) => {
                    problems.push(format!("group {}: {}", group.external_id, describe(&error)))
                }
            }
        }
        outcome.problem = (!problems.is_empty()).then(|| bounded(problems.join("; ")));
        Ok(outcome)
    }

    /// The lowercase addresses of the group's active people, nested groups included.
    async fn members(&self, token: &str, group: &str) -> Result<Vec<String>, IdentityError> {
        let mut url = self.url(&["groups", group, "members"])?;
        url.query_pairs_mut()
            .append_pair("includeDerivedMembership", "true")
            .append_pair("maxResults", "200");
        let members: Vec<Member> = self.list(token, url).await?;
        Ok(members
            .into_iter()
            .filter(|member| {
                member.kind.as_deref() == Some("USER") && member.status.as_deref() == Some("ACTIVE")
            })
            .filter_map(|member| member.email.map(|email| email.to_ascii_lowercase()))
            .collect())
    }

    /// Sets the group to the active accounts signed in with these addresses; returns how many.
    async fn mirror(&self, group: Uuid, emails: &[String]) -> Result<usize, IdentityError> {
        let mut transaction = self.pool.begin().await?;
        let linked = sqlx::query_scalar!(
            "SELECT id FROM human_teams WHERE id = $1 AND active AND source = 'google' FOR UPDATE",
            group,
        )
        .fetch_optional(&mut *transaction)
        .await?;
        if linked.is_none() {
            return Ok(0);
        }
        let accounts = sqlx::query_scalar!(
            r#"
            SELECT DISTINCT b.account_id
            FROM oidc_bindings b JOIN human_accounts a ON a.id = b.account_id AND a.active
            WHERE b.active AND lower(b.email) = ANY($1)
            "#,
            emails,
        )
        .fetch_all(&mut *transaction)
        .await?;
        groups::sync_members(&mut transaction, group, &accounts).await?;
        transaction.commit().await?;
        Ok(accounts.len())
    }

    /// Syncs if enabled; `None` when there is nothing to sync.
    async fn tend(&self) -> Result<Option<SyncOutcome>, IdentityError> {
        match self.config().await? {
            Some(config) if config.enabled => self.sync(&config).await.map(Some),
            _ => Ok(None),
        }
    }
}

/// Mirrors linked groups every 15 minutes while the directory is enabled.
pub async fn keep_synced(directory: GoogleDirectory) {
    loop {
        let problem = match directory.tend().await {
            Ok(outcome) => outcome.and_then(|outcome| outcome.problem),
            Err(error) => Some(describe(&error)),
        };
        if let Some(problem) = problem {
            tracing::warn!(%problem, "Google group sync incomplete");
        }
        tokio::time::sleep(PERIOD).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> ServiceAccount {
        ServiceAccount {
            kind: "service_account".into(),
            client_email: "sync@project.iam.gserviceaccount.com".into(),
            private_key: include_str!("../../tests/fixtures/google_service_account.pem").into(),
        }
    }

    #[test]
    fn assertion_is_an_rs256_jwt_acting_as_the_administrator() {
        let account = account();
        let jwt = account
            .assertion(
                "admin@example.com",
                "https://oauth2.googleapis.com/token",
                1_000,
            )
            .unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let decode = |part: &str| -> serde_json::Value {
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
        };
        assert_eq!(decode(parts[0])["alg"], "RS256");
        let claims = decode(parts[1]);
        assert_eq!(claims["iss"], "sync@project.iam.gserviceaccount.com");
        assert_eq!(claims["sub"], "admin@example.com");
        assert_eq!(claims["scope"], SCOPE);
        assert_eq!(claims["aud"], "https://oauth2.googleapis.com/token");
        assert_eq!(
            (claims["iat"].as_u64(), claims["exp"].as_u64()),
            (Some(1_000), Some(4_600))
        );
        let key = account.key_pair().unwrap();
        ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            key.public().as_ref(),
        )
        .verify(
            format!("{}.{}", parts[0], parts[1]).as_bytes(),
            &URL_SAFE_NO_PAD.decode(parts[2]).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn keys_that_are_not_service_accounts_are_refused() {
        let mut json = serde_json::json!({
            "type": "service_account",
            "client_email": "sync@project.iam.gserviceaccount.com",
            "private_key": account().private_key,
        });
        assert!(ServiceAccount::parse(json.to_string().as_bytes()).is_ok());
        json["type"] = "authorized_user".into();
        assert!(ServiceAccount::parse(json.to_string().as_bytes()).is_err());
        json["type"] = "service_account".into();
        json["private_key"] = "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----".into();
        assert!(ServiceAccount::parse(json.to_string().as_bytes()).is_err());
    }
}

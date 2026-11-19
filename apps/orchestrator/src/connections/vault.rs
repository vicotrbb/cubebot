//! Secrets are Credentials bound to their item's id and leave this module only as one fill's text.
//! Callers authorize people; runtime calls name the authenticated agent.
use super::{Access, Connections, ProviderAccess, validate_access};
use crate::{
    agents::AgentId,
    identity::{
        IdentityError,
        audit::{Event, record},
    },
};
use cubebot_protocol::{
    desktop::typable,
    runtime::{VaultField, VaultItem, VaultKind, VaultPut},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveVaultItem {
    /// On create, the agent whose own item this is; an installation item when absent. An update
    /// keeps the item's scope.
    #[serde(default)]
    pub agent: Option<Uuid>,
    pub kind: VaultKind,
    pub name: String,
    #[serde(default)]
    pub site: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    /// Required on create; an update without it keeps the current secret.
    #[serde(default)]
    pub secret: Option<String>,
    /// Installation items only: all agents when absent on create, unchanged on update.
    #[serde(default)]
    pub access: Option<ProviderAccess>,
    #[serde(default)]
    pub card: Option<CardDetails>,
}

#[derive(Clone, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CardDetails {
    pub holder: String,
    pub number: String,
    pub expiry: String,
    pub cvc: String,
}

impl CardDetails {
    fn normalized(&self) -> Option<Self> {
        let number: String = self
            .number
            .chars()
            .filter(|c| !matches!(c, ' ' | '-'))
            .collect();
        let expiry: String = self.expiry.chars().filter(|c| !c.is_whitespace()).collect();
        let card = Self {
            holder: self.holder.trim().to_owned(),
            number,
            expiry,
            cvc: self.cvc.trim().to_owned(),
        };
        card.valid().then_some(card)
    }

    fn valid(&self) -> bool {
        let digits = |text: &str, sizes: std::ops::RangeInclusive<usize>| {
            sizes.contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit())
        };
        let month = self.expiry.get(..2).and_then(|m| m.parse::<u8>().ok());
        (1..=255).contains(&self.holder.len())
            && typable(&self.holder)
            && digits(&self.number, 12..=19)
            && luhn(&self.number)
            && self.expiry.len() == 5
            && self.expiry.as_bytes()[2] == b'/'
            && month.is_some_and(|m| (1..=12).contains(&m))
            && digits(&self.expiry[3..], 2..=2)
            && digits(&self.cvc, 3..=4)
    }

    fn field(&self, field: VaultField) -> Option<String> {
        let (month, year) = self.expiry.split_once('/')?;
        Some(match field {
            VaultField::Holder => self.holder.clone(),
            VaultField::Number => self.number.clone(),
            VaultField::Expiry => self.expiry.clone(),
            VaultField::ExpiryMonth => month.to_owned(),
            VaultField::ExpiryYear => format!("20{year}"),
            VaultField::Cvc => self.cvc.clone(),
            VaultField::Username | VaultField::Secret => return None,
        })
    }
}

fn luhn(number: &str) -> bool {
    let sum: u32 = number
        .bytes()
        .rev()
        .enumerate()
        .map(|(at, b)| {
            let digit = u32::from(b - b'0');
            match (at % 2, digit * 2) {
                (0, _) => digit,
                (_, doubled) if doubled > 9 => doubled - 9,
                (_, doubled) => doubled,
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

fn brand(number: &str) -> &'static str {
    let prefix = |len: usize| number.get(..len).and_then(|p| p.parse::<u32>().ok());
    match (prefix(1), prefix(2), prefix(4)) {
        (Some(4), ..) => "Visa",
        (_, Some(34 | 37), _) => "American Express",
        (_, Some(51..=55), _) | (.., Some(2221..=2720)) => "Mastercard",
        (.., Some(6011)) | (_, Some(65), _) => "Discover",
        (_, Some(36 | 38 | 39), _) | (.., Some(3000..=3059)) => "Diners Club",
        (.., Some(3528..=3589)) => "JCB",
        _ => "Card",
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportVault {
    #[serde(default)]
    pub access: Option<ProviderAccess>,
    pub logins: Vec<ImportedLogin>,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportedLogin {
    pub name: String,
    pub site: String,
    pub username: String,
    pub secret: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct VaultImported {
    pub imported: usize,
    pub skipped: usize,
}

pub const MOST_IMPORTED: usize = 5000;

#[derive(Serialize, utoipa::ToSchema)]
pub struct VaultSummary {
    pub id: Uuid,
    /// The agent whose own item this is; an installation item when absent.
    pub agent: Option<Uuid>,
    pub kind: VaultKind,
    pub name: String,
    pub site: Option<String>,
    pub username: Option<String>,
    pub brand: Option<String>,
    pub last4: Option<String>,
    pub expires: Option<String>,
    #[schema(value_type = ProviderAccess)]
    pub access: Access,
    pub last_used_at: Option<String>,
}

struct Row {
    id: Uuid,
    agent_id: Option<Uuid>,
    kind: String,
    name: String,
    site: Option<String>,
    username: Option<String>,
    card_brand: Option<String>,
    card_last4: Option<String>,
    card_expires: Option<String>,
    access: Access,
    last_used_at: Option<String>,
}

impl From<Row> for VaultSummary {
    fn from(row: Row) -> Self {
        Self {
            id: row.id,
            agent: row.agent_id,
            kind: kind(&row.kind),
            name: row.name,
            site: row.site,
            username: row.username,
            brand: row.card_brand,
            last4: row.card_last4,
            expires: row.card_expires,
            access: row.access,
            last_used_at: row.last_used_at,
        }
    }
}

fn kind(stored: &str) -> VaultKind {
    match stored {
        "login" => VaultKind::Login,
        "card" => VaultKind::Card,
        _ => VaultKind::Secret,
    }
}

fn stored(kind: VaultKind) -> &'static str {
    match kind {
        VaultKind::Login => "login",
        VaultKind::Secret => "secret",
        VaultKind::Card => "card",
    }
}

fn unique_name(taken: &mut HashSet<String>, name: &str, username: &str) -> String {
    let named = (username.len() <= 128).then(|| format!(" ({username})"));
    [Some(String::new()), named]
        .into_iter()
        .flatten()
        .chain((2..).map(|n| format!(" {n}")))
        .map(|suffix| format!("{}{suffix}", clip(name, 255 - suffix.len())))
        .find(|candidate| taken.insert(candidate.clone()))
        .unwrap_or_default()
}

fn clip(text: &str, most: usize) -> &str {
    let mut end = most.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

pub(crate) fn host(site: &str) -> String {
    let site = site.trim().to_ascii_lowercase();
    let site = site
        .split_once("://")
        .map_or(site.as_str(), |(_, rest)| rest);
    let site = site.split(['/', '?', '#']).next().unwrap_or_default();
    let site = site.rsplit_once('@').map_or(site, |(_, host)| host);
    let site = site
        .split(':')
        .next()
        .unwrap_or_default()
        .trim_end_matches('.');
    site.strip_prefix("www.").unwrap_or(site).to_owned()
}

/// Same host or one's subdomain: `amazon.com.br` serves `www.amazon.com.br`, never `amazon.com`.
pub(crate) fn same_site(saved: &str, wanted: &str) -> bool {
    let (saved, wanted) = (host(saved), host(wanted));
    let under =
        |child: &str, parent: &str| parent.contains('.') && child.ends_with(&format!(".{parent}"));
    !saved.is_empty() && (saved == wanted || under(&wanted, &saved) || under(&saved, &wanted))
}

pub fn problem(
    kind: VaultKind,
    name: &str,
    site: Option<&str>,
    username: Option<&str>,
    secret: Option<&str>,
) -> Option<&'static str> {
    let text = |value: &str| (1..=255).contains(&value.len()) && !value.trim().is_empty();
    let login = matches!(kind, VaultKind::Login);
    if !text(name) || name.contains('\0') {
        return Some("a name is 1 to 255 bytes");
    }
    if login != (site.is_some_and(text) && username.is_some_and(text))
        || (!login && (site.is_some() || username.is_some()))
    {
        return Some("a login has a site and a username of 1 to 255 bytes; a secret has neither");
    }
    if secret.is_some_and(|secret| secret.is_empty() || secret.len() > 4096) {
        return Some("a secret is 1 to 4096 bytes");
    }
    if !username.into_iter().chain(secret).all(typable) || site.is_some_and(|s| s.contains('\0')) {
        return Some("the desktop types only printable ASCII and spaces into a username or secret");
    }
    None
}

fn invalid(input: &SaveVaultItem) -> bool {
    let (site, username) = (input.site.as_deref(), input.username.as_deref());
    let card = matches!(input.kind, VaultKind::Card);
    let secret = input.secret.as_deref();
    let details = input.card.as_ref().map(CardDetails::normalized);
    problem(input.kind, &input.name, site, username, secret).is_some()
        || (input.agent.is_some() && input.access.is_some())
        || (card && secret.is_some())
        || (!card && details.is_some())
        || details.is_some_and(|card| card.is_none())
}

struct Sealing {
    secret: Option<String>,
    brand: Option<&'static str>,
    last4: Option<String>,
    expires: Option<String>,
}

fn sealing(input: &SaveVaultItem) -> Sealing {
    match input.card.as_ref().and_then(CardDetails::normalized) {
        Some(card) => Sealing {
            secret: serde_json::to_string(&card).ok(),
            brand: Some(brand(&card.number)),
            last4: Some(card.number[card.number.len() - 4..].to_owned()),
            expires: Some(card.expiry),
        },
        None => Sealing {
            secret: input.secret.clone(),
            brand: None,
            last4: None,
            expires: None,
        },
    }
}

/// Binds a sealed record to this installation, kind and row, so a copy on another row does not open.
/// Reads through the caller's transaction: a second connection can fail on an exhausted pool.
pub(super) async fn sealing_context<'e>(
    db: impl sqlx::PgExecutor<'e>,
    kind: &str,
    id: Uuid,
) -> Result<Vec<u8>, IdentityError> {
    let installation = sqlx::query_scalar!(
        r#"
        SELECT id AS "id!"
        FROM installation
        "#,
    )
    .fetch_one(db)
    .await?;
    Ok(format!("cubebot:{installation}:{kind}:{id}:key:1").into_bytes())
}

impl Connections {
    async fn seal<'e>(
        &self,
        db: impl sqlx::PgExecutor<'e>,
        item: Uuid,
        secret: &str,
    ) -> Result<Vec<u8>, IdentityError> {
        let context = sealing_context(db, "vault", item).await?;
        self.credentials
            .encrypt(secret.as_bytes(), &context)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))
    }

    #[tracing::instrument(skip_all, fields(item = %item))]
    pub async fn vault_scope(&self, item: Uuid) -> Result<Option<Option<AgentId>>, IdentityError> {
        let agent = sqlx::query_scalar!(
            "
            SELECT agent_id
            FROM vault_items
            WHERE id=$1
            ",
            item,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(agent.map(|agent| agent.map(AgentId)))
    }

    #[tracing::instrument(skip_all)]
    pub async fn installation_vault(&self) -> Result<Vec<VaultSummary>, IdentityError> {
        Ok(sqlx::query_as!(
            Row,
            r#"
            SELECT id, agent_id, kind, name, site, username, card_brand, card_last4, card_expires,
                access AS "access!: Access", (to_jsonb(last_used_at) #>> '{}') AS last_used_at
            FROM vault_items
            WHERE agent_id IS NULL
            ORDER BY name
            "#,
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(VaultSummary::from)
        .collect())
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub async fn agent_vault(&self, agent: AgentId) -> Result<Vec<VaultSummary>, IdentityError> {
        Ok(sqlx::query_as!(
            Row,
            r#"
            SELECT id, agent_id, kind, name, site, username, card_brand, card_last4, card_expires,
                access AS "access!: Access", (to_jsonb(last_used_at) #>> '{}') AS last_used_at
            FROM vault_items
            WHERE agent_id=$1
                OR (agent_id IS NULL AND (access->>'mode' = 'all_agents'
                    OR access->'agents' @> jsonb_build_array($1::uuid)))
            ORDER BY agent_id IS NULL, name
            "#,
            agent.0,
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(VaultSummary::from)
        .collect())
    }

    pub async fn runtime_vault(&self, agent: AgentId) -> Result<Vec<VaultItem>, IdentityError> {
        let items = self.agent_vault(agent).await?;
        Ok(items
            .into_iter()
            .map(|item| VaultItem {
                id: item.id,
                kind: item.kind,
                name: item.name,
                site: item.site,
                username: item.username,
                last4: item.last4,
                own: item.agent.is_some(),
            })
            .collect())
    }

    #[tracing::instrument(skip_all, fields(actor = %actor))]
    pub async fn create_vault_item(
        &self,
        actor: Uuid,
        input: &SaveVaultItem,
    ) -> Result<VaultSummary, IdentityError> {
        let sealed = sealing(input);
        let Some(secret) = sealed.secret.as_deref().filter(|_| !invalid(input)) else {
            return Err(IdentityError::InvalidInput);
        };
        let access = input.access.clone().unwrap_or_default();
        let mut tx = self.pool.begin().await?;
        validate_access(&mut tx, &access).await?;
        let id = Uuid::new_v4();
        let ciphertext = self.seal(&mut *tx, id, secret).await?;
        let row = sqlx::query_as!(
            Row,
            r#"
            INSERT INTO vault_items (id, agent_id, kind, name, site, username, ciphertext, access, created_by, card_brand, card_last4, card_expires)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING id, agent_id, kind, name, site, username, card_brand, card_last4, card_expires,
                access AS "access!: Access", NULL::text AS last_used_at
            "#,
            id,
            input.agent,
            stored(input.kind),
            input.name,
            input.site,
            input.username,
            ciphertext,
            sqlx::types::Json(&access) as _,
            actor,
            sealed.brand,
            sealed.last4,
            sealed.expires,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.into())
    }

    #[tracing::instrument(skip_all, fields(actor = %actor, logins = input.logins.len()))]
    pub async fn import_vault_items(
        &self,
        actor: Uuid,
        input: &ImportVault,
    ) -> Result<VaultImported, IdentityError> {
        let refused = |login: &ImportedLogin| {
            let (site, username) = (Some(login.site.as_str()), Some(login.username.as_str()));
            problem(
                VaultKind::Login,
                &login.name,
                site,
                username,
                Some(&login.secret),
            )
            .is_some()
        };
        if input.logins.len() > MOST_IMPORTED || input.logins.iter().any(refused) {
            return Err(IdentityError::InvalidInput);
        }
        let access = input.access.clone().unwrap_or_default();
        let mut tx = self.pool.begin().await?;
        validate_access(&mut tx, &access).await?;
        let mut saved: HashSet<(String, String)> = sqlx::query!(
            r#"
            SELECT site AS "site!", username AS "username!"
            FROM vault_items
            WHERE agent_id IS NULL AND kind = 'login'
            "#,
        )
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| (host(&row.site), row.username))
        .collect();
        let mut names: HashSet<String> =
            sqlx::query_scalar!("SELECT name FROM vault_items WHERE agent_id IS NULL")
                .fetch_all(&mut *tx)
                .await?
                .into_iter()
                .collect();
        let mut imported = 0;
        for login in &input.logins {
            if !saved.insert((host(&login.site), login.username.clone())) {
                continue;
            }
            let name = unique_name(&mut names, &login.name, &login.username);
            let id = Uuid::new_v4();
            let ciphertext = self.seal(&mut *tx, id, &login.secret).await?;
            sqlx::query!(
                r#"
                INSERT INTO vault_items (id, kind, name, site, username, ciphertext, access, created_by)
                VALUES ($1, 'login', $2, $3, $4, $5, $6, $7)
                "#,
                id,
                name,
                login.site,
                login.username,
                ciphertext,
                sqlx::types::Json(&access) as _,
                actor,
            )
            .execute(&mut *tx)
            .await?;
            imported += 1;
        }
        tx.commit().await?;
        let skipped = input.logins.len() - imported;
        Ok(VaultImported { imported, skipped })
    }

    #[tracing::instrument(skip_all, fields(item = %item))]
    pub async fn update_vault_item(
        &self,
        item: Uuid,
        input: &SaveVaultItem,
    ) -> Result<VaultSummary, IdentityError> {
        if invalid(input) {
            return Err(IdentityError::InvalidInput);
        }
        let mut tx = self.pool.begin().await?;
        if let Some(access) = &input.access {
            validate_access(&mut tx, access).await?;
        }
        let sealed = sealing(input);
        let ciphertext = match &sealed.secret {
            Some(secret) => Some(self.seal(&mut *tx, item, secret).await?),
            None => None,
        };
        let row = sqlx::query_as!(
            Row,
            r#"
            UPDATE vault_items
            SET kind=$2, name=$3, site=$4, username=$5,
                ciphertext=COALESCE($6, ciphertext),
                access=CASE WHEN agent_id IS NULL THEN COALESCE($7, access) ELSE access END,
                card_brand=CASE WHEN $2='card' THEN COALESCE($8, card_brand) END,
                card_last4=CASE WHEN $2='card' THEN COALESCE($9, card_last4) END,
                card_expires=CASE WHEN $2='card' THEN COALESCE($10, card_expires) END,
                updated_at=clock_timestamp()
            WHERE id=$1 AND ($7::jsonb IS NULL OR agent_id IS NULL) AND (kind=$2 OR $6::bytea IS NOT NULL)
            RETURNING id, agent_id, kind, name, site, username, card_brand, card_last4, card_expires,
                access AS "access!: Access", (to_jsonb(last_used_at) #>> '{}') AS last_used_at
            "#,
            item,
            stored(input.kind),
            input.name,
            input.site,
            input.username,
            ciphertext,
            input.access.as_ref().map(sqlx::types::Json) as _,
            sealed.brand,
            sealed.last4,
            sealed.expires,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        tx.commit().await?;
        Ok(row.into())
    }

    #[tracing::instrument(skip_all, fields(item = %item))]
    pub async fn delete_vault_item(&self, item: Uuid) -> Result<String, IdentityError> {
        let deleted = sqlx::query_scalar!(
            "
            DELETE FROM vault_items
            WHERE id=$1
            RETURNING name
            ",
            item,
        )
        .fetch_optional(&self.pool)
        .await?;
        deleted.ok_or(IdentityError::NotFound)
    }

    pub(crate) async fn card_name(
        &self,
        agent: AgentId,
        item: Uuid,
    ) -> Result<Option<String>, IdentityError> {
        Ok(sqlx::query_scalar!(
            "
            SELECT name
            FROM vault_items
            WHERE id=$2 AND kind='card' AND (agent_id=$1
                OR (agent_id IS NULL AND (access->>'mode' = 'all_agents'
                    OR access->'agents' @> jsonb_build_array($1::uuid))))
            ",
            agent.0,
            item,
        )
        .fetch_optional(&self.pool)
        .await?)
    }

    pub(crate) async fn tried_login_site(
        &self,
        agent: AgentId,
        item: Uuid,
    ) -> Result<Option<String>, IdentityError> {
        let site = sqlx::query_scalar!(
            "
            UPDATE vault_items
            SET last_tried_at=clock_timestamp()
            WHERE id=$2 AND kind='login' AND (agent_id=$1
                OR (agent_id IS NULL AND (access->>'mode' = 'all_agents'
                    OR access->'agents' @> jsonb_build_array($1::uuid))))
            RETURNING site
            ",
            agent.0,
            item,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(site.flatten())
    }

    /// Never log the returned text.
    #[tracing::instrument(skip_all, fields(agent = %agent.0, item = %item))]
    pub async fn vault_fill_text(
        &self,
        agent: AgentId,
        person: Option<Uuid>,
        item: Uuid,
        (field, page, amount): (VaultField, Option<&str>, Option<&str>),
    ) -> Result<Option<String>, IdentityError> {
        let row = sqlx::query!(
            "
            UPDATE vault_items
            SET last_used_at=clock_timestamp()
            WHERE id=$2 AND (kind='card')=$3 AND (agent_id=$1
                OR (agent_id IS NULL AND (access->>'mode' = 'all_agents'
                    OR access->'agents' @> jsonb_build_array($1::uuid))))
            RETURNING username, ciphertext, name, site, kind
            ",
            agent.0,
            item,
            field.card(),
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let event = match person {
            Some(person) => Event::by(person, "vault.fill"),
            None => Event::by_agent(agent.0, "vault.fill"),
        };
        let detail = json!({"agent": agent.0, "field": field, "name": row.name, "site": row.site, "page": page.map(host), "amount": amount});
        if let Err(error) = record(&self.pool, &event.on(item).detail(detail)).await {
            tracing::error!(%error, "vault fill not recorded in the audit log");
        }
        if field == VaultField::Username {
            return Ok(row.username);
        }
        let context = sealing_context(&self.pool, "vault", item).await?;
        let secret = self
            .credentials
            .decrypt(&row.ciphertext, &context)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
        // Only the UTF-8 position survives; FromUtf8Error would keep the decrypted bytes.
        let secret = String::from_utf8(secret).map_err(|error| {
            IdentityError::CredentialUnavailable(Some(error.utf8_error().into()))
        })?;
        if row.kind != "card" {
            return Ok(Some(secret));
        }
        let card: CardDetails = serde_json::from_str(&secret)
            .map_err(|_| IdentityError::CredentialUnavailable(None))?;
        Ok(card.field(field))
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub(crate) async fn vault_put(
        &self,
        agent: AgentId,
        input: &VaultPut,
    ) -> Result<Result<Uuid, &'static str>, IdentityError> {
        let (site, username) = (input.site.as_deref(), input.username.as_deref());
        if matches!(input.kind, VaultKind::Card) {
            return Ok(Err("only a person saves a card, in the console"));
        }
        if let Some(problem) = problem(input.kind, &input.name, site, username, Some(&input.secret))
        {
            return Ok(Err(problem));
        }
        let mut tx = self.pool.begin().await?;
        // A name not saved yet has no row to lock; the agent's row orders concurrent saves.
        sqlx::query!(
            "SELECT id FROM agents WHERE id=$1 FOR NO KEY UPDATE",
            agent.0
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        let existing = sqlx::query_scalar!(
            "
            SELECT id
            FROM vault_items
            WHERE agent_id=$1 AND name=$2
            FOR UPDATE
            ",
            agent.0,
            input.name,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let id = existing.unwrap_or_else(Uuid::new_v4);
        let ciphertext = self.seal(&mut *tx, id, &input.secret).await?;
        sqlx::query!(
            "
            INSERT INTO vault_items (id, agent_id, kind, name, site, username, ciphertext)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (id) DO UPDATE SET
                kind=EXCLUDED.kind, site=EXCLUDED.site, username=EXCLUDED.username,
                ciphertext=EXCLUDED.ciphertext, updated_at=clock_timestamp()
            ",
            id,
            agent.0,
            stored(input.kind),
            input.name,
            input.site,
            input.username,
            ciphertext,
        )
        .execute(&mut *tx)
        .await?;
        let kind = if existing.is_some() {
            "vault.update"
        } else {
            "vault.create"
        };
        let detail = json!({"name": input.name, "site": input.site, "kind": input.kind});
        record(
            &mut *tx,
            &Event::by_agent(agent.0, kind).on(id).detail(detail),
        )
        .await?;
        tx.commit().await?;
        Ok(Ok(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(number: &str, expiry: &str, cvc: &str) -> CardDetails {
        CardDetails {
            holder: "Ada Lovelace".into(),
            number: number.into(),
            expiry: expiry.into(),
            cvc: cvc.into(),
        }
    }

    #[test]
    fn a_card_is_checked_normalized_and_filled_one_field_at_a_time() {
        let saved = card("4242 4242-4242 4242", " 07/29", "123")
            .normalized()
            .unwrap();
        assert_eq!(saved.number, "4242424242424242");
        assert_eq!(saved.field(VaultField::Expiry).unwrap(), "07/29");
        assert_eq!(saved.field(VaultField::ExpiryMonth).unwrap(), "07");
        assert_eq!(saved.field(VaultField::ExpiryYear).unwrap(), "2029");
        assert_eq!(saved.field(VaultField::Cvc).unwrap(), "123");
        assert_eq!(saved.field(VaultField::Secret), None);
        assert_eq!(brand(&saved.number), "Visa");
        assert_eq!(brand("5555555555554444"), "Mastercard");
        assert_eq!(brand("2223003122003222"), "Mastercard");
        assert_eq!(brand("378282246310005"), "American Express");
        assert_eq!(brand("6011111111111117"), "Discover");
        for refused in [
            card("4242424242424241", "07/29", "123"),
            card("4242424242424242", "13/29", "123"),
            card("4242424242424242", "7/29", "123"),
            card("4242424242424242", "07/29", "12"),
            card("42424242", "07/29", "123"),
        ] {
            assert!(refused.normalized().is_none(), "{}", refused.number);
        }
    }

    #[test]
    fn an_imported_name_already_taken_gains_its_username_then_a_counter() {
        let mut taken = HashSet::from(["github.com".to_owned()]);
        let mut name = |name: &str, username: &str| unique_name(&mut taken, name, username);
        assert_eq!(name("github.com", "octocat"), "github.com (octocat)");
        assert_eq!(name("github.com", "octocat"), "github.com 2");
        assert_eq!(name("example.com", "me"), "example.com");
        let (long, user) = ("é".repeat(200), "u".repeat(200));
        assert_eq!(name(&long, &user), "é".repeat(127));
        assert_eq!(name(&long, &user), format!("{} 2", "é".repeat(126)));
    }

    #[test]
    fn a_login_serves_its_site_and_subdomains_only() {
        for page in [
            "amazon.com.br",
            "www.amazon.com.br",
            "https://www.amazon.com.br/gp/your-account/order-history",
            "https://www.amazon.com.br/ap/signin?openid.return_to=x",
            "HTTPS://Signin.Amazon.com.br:443/",
            "user@amazon.com.br.",
        ] {
            assert!(same_site("amazon.com.br", page), "{page}");
            assert!(same_site("https://www.amazon.com.br/", page), "{page}");
        }
        assert!(same_site("accounts.google.com", "google.com"));
        for page in [
            "amazon.com",
            "notamazon.com.br",
            "amazon.com.br.evil.io",
            "",
            "br",
        ] {
            assert!(!same_site("amazon.com.br", page), "{page}");
        }
        assert!(!same_site("", "amazon.com.br"));
        assert!(!same_site("br", "amazon.com.br"));
    }

    #[test]
    fn items_the_desktop_cannot_fill_are_refused() {
        let login = |username: &str, secret: &str| {
            problem(
                VaultKind::Login,
                "GitHub",
                Some("github.com"),
                Some(username),
                Some(secret),
            )
        };
        assert_eq!(login("octocat", "correct horse ~!"), None);
        assert!(
            login("octocat", "pässword")
                .unwrap()
                .contains("printable ASCII")
        );
        assert!(login("octocat", "tab\there").is_some());
        assert!(login("octo\ncat", "secret").is_some());
        assert!(problem(VaultKind::Secret, "key", Some("x"), None, Some("s")).is_some());
        assert!(problem(VaultKind::Login, "key", Some("x"), None, Some("s")).is_some());
        assert_eq!(
            problem(VaultKind::Secret, "API key", None, None, Some("abc")),
            None
        );
        assert!(problem(VaultKind::Secret, " ", None, None, Some("abc")).is_some());
    }
}

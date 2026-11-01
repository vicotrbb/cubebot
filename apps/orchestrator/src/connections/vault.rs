//! The vault (ADR 0086): logins and secrets an agent types into its desktop through the arbiter
//! without reading them. Secrets are Credentials records bound to their item's id and never
//! leave this module except as the text of one fill. Installation items follow ADR 0081 access;
//! an agent's own items belong to it alone. Callers authorize people; the runtime calls name
//! the authenticated agent.
use super::{Access, Connections, ProviderAccess, validate_access};
use crate::{agents::AgentId, identity::IdentityError};
use cubebot_protocol::{
    desktop::typable,
    runtime::{VaultField, VaultItem, VaultKind, VaultPut},
};
use serde::{Deserialize, Serialize};
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
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct VaultSummary {
    pub id: Uuid,
    /// The agent whose own item this is; an installation item when absent.
    pub agent: Option<Uuid>,
    pub kind: VaultKind,
    pub name: String,
    pub site: Option<String>,
    pub username: Option<String>,
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
            access: row.access,
            last_used_at: row.last_used_at,
        }
    }
}

fn kind(stored: &str) -> VaultKind {
    match stored {
        "login" => VaultKind::Login,
        _ => VaultKind::Secret,
    }
}

fn stored(kind: VaultKind) -> &'static str {
    match kind {
        VaultKind::Login => "login",
        VaultKind::Secret => "secret",
    }
}

/// Why an item cannot be saved, in words the model and the console can show.
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
    let secret = input.secret.as_deref();
    problem(input.kind, &input.name, site, username, secret).is_some()
        || (input.agent.is_some() && input.access.is_some())
}

impl Connections {
    async fn vault_context(&self, item: Uuid) -> Result<Vec<u8>, IdentityError> {
        let installation = sqlx::query_scalar!(
            r#"
            SELECT id AS "id!"
            FROM installation
            "#,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(format!("cubebot:{installation}:vault:{item}:key:1").into_bytes())
    }

    async fn seal(&self, item: Uuid, secret: &str) -> Result<Vec<u8>, IdentityError> {
        let context = self.vault_context(item).await?;
        self.credentials
            .encrypt(secret.as_bytes(), &context)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))
    }

    /// The scope of an item: `Some(None)` for an installation item, `None` when there is none.
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

    /// Installation items with their access; the caller is an administrator.
    #[tracing::instrument(skip_all)]
    pub async fn installation_vault(&self) -> Result<Vec<VaultSummary>, IdentityError> {
        Ok(sqlx::query_as!(
            Row,
            r#"
            SELECT id, agent_id, kind, name, site, username, access AS "access!: Access",
                (to_jsonb(last_used_at) #>> '{}') AS last_used_at
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

    /// Items `agent` may fill: its own and the installation's it has access to.
    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub async fn agent_vault(&self, agent: AgentId) -> Result<Vec<VaultSummary>, IdentityError> {
        Ok(sqlx::query_as!(
            Row,
            r#"
            SELECT id, agent_id, kind, name, site, username, access AS "access!: Access",
                (to_jsonb(last_used_at) #>> '{}') AS last_used_at
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

    /// The agent's view of [`Self::agent_vault`].
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
                own: item.agent.is_some(),
            })
            .collect())
    }

    /// The caller has authorized `actor` for the item's scope.
    #[tracing::instrument(skip_all, fields(actor = %actor))]
    pub async fn create_vault_item(
        &self,
        actor: Uuid,
        input: &SaveVaultItem,
    ) -> Result<VaultSummary, IdentityError> {
        let Some(secret) = input.secret.as_deref().filter(|_| !invalid(input)) else {
            return Err(IdentityError::InvalidInput);
        };
        let access = input.access.clone().unwrap_or_default();
        let mut tx = self.pool.begin().await?;
        validate_access(&mut tx, &access).await?;
        let id = Uuid::new_v4();
        let ciphertext = self.seal(id, secret).await?;
        let row = sqlx::query_as!(
            Row,
            r#"
            INSERT INTO vault_items (id, agent_id, kind, name, site, username, ciphertext, access, created_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING id, agent_id, kind, name, site, username, access AS "access!: Access",
                NULL::text AS last_used_at
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
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.into())
    }

    /// The caller has authorized the change for the item's scope.
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
        let ciphertext = match &input.secret {
            Some(secret) => Some(self.seal(item, secret).await?),
            None => None,
        };
        let row = sqlx::query_as!(
            Row,
            r#"
            UPDATE vault_items
            SET kind=$2, name=$3, site=$4, username=$5,
                ciphertext=COALESCE($6, ciphertext),
                access=CASE WHEN agent_id IS NULL THEN COALESCE($7, access) ELSE access END,
                updated_at=clock_timestamp()
            WHERE id=$1 AND ($7::jsonb IS NULL OR agent_id IS NULL)
            RETURNING id, agent_id, kind, name, site, username, access AS "access!: Access",
                (to_jsonb(last_used_at) #>> '{}') AS last_used_at
            "#,
            item,
            stored(input.kind),
            input.name,
            input.site,
            input.username,
            ciphertext,
            input.access.as_ref().map(sqlx::types::Json) as _,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        tx.commit().await?;
        Ok(row.into())
    }

    /// The caller has authorized the deletion for the item's scope.
    #[tracing::instrument(skip_all, fields(item = %item))]
    pub async fn delete_vault_item(&self, item: Uuid) -> Result<(), IdentityError> {
        let deleted = sqlx::query!(
            "
            DELETE FROM vault_items
            WHERE id=$1
            ",
            item,
        )
        .execute(&self.pool)
        .await?;
        if deleted.rows_affected() == 0 {
            return Err(IdentityError::NotFound);
        }
        Ok(())
    }

    /// The text to type for one field of an item `agent` may fill, and records its use; `None`
    /// when the agent has no such item. Never log it.
    #[tracing::instrument(skip_all, fields(agent = %agent.0, item = %item))]
    pub(crate) async fn vault_fill_text(
        &self,
        agent: AgentId,
        item: Uuid,
        field: VaultField,
    ) -> Result<Option<String>, IdentityError> {
        let row = sqlx::query!(
            "
            UPDATE vault_items
            SET last_used_at=clock_timestamp()
            WHERE id=$2 AND (agent_id=$1
                OR (agent_id IS NULL AND (access->>'mode' = 'all_agents'
                    OR access->'agents' @> jsonb_build_array($1::uuid))))
            RETURNING username, ciphertext
            ",
            agent.0,
            item,
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        if field == VaultField::Username {
            return Ok(row.username);
        }
        let context = self.vault_context(item).await?;
        let secret = self
            .credentials
            .decrypt(&row.ciphertext, &context)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
        // Only the UTF-8 position survives; FromUtf8Error would keep the decrypted bytes.
        String::from_utf8(secret)
            .map(Some)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.utf8_error().into())))
    }

    /// Saves the agent's own item, replacing the secret of one with the same name. The inner
    /// error is the refusal the model reads.
    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub(crate) async fn vault_put(
        &self,
        agent: AgentId,
        input: &VaultPut,
    ) -> Result<Result<Uuid, &'static str>, IdentityError> {
        let (site, username) = (input.site.as_deref(), input.username.as_deref());
        if let Some(problem) = problem(input.kind, &input.name, site, username, Some(&input.secret))
        {
            return Ok(Err(problem));
        }
        let mut tx = self.pool.begin().await?;
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
        let ciphertext = self.seal(id, &input.secret).await?;
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
        tx.commit().await?;
        Ok(Ok(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

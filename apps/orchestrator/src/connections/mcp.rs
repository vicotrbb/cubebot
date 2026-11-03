//! MCP servers (docs/adr/agents.md#local-and-remote-mcp-server-support) as installation configuration. Administrators add them; each is
//! available to every agent or to the agents granted it (docs/adr/identity.md#installation-wide-connection-access-by-default). A stdio
//! server's environment and an HTTP server's headers are sealed as one Credentials record bound
//! to the server's id: people see their names only, and the values leave this module only in
//! the turn delivered to an agent with access. Callers authorize people.
use super::{Access, Connections, ProviderAccess, validate_access};
use crate::{agents::AgentId, identity::IdentityError};
use cubebot_protocol::runtime::McpServer;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    /// A program the agent runs as its tool user, speaking JSON-RPC on standard I/O.
    Stdio,
    /// A remote Streamable HTTP endpoint.
    Http,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveMcpServer {
    /// Letters, digits or hyphens, at most 32; the server's tools are offered as `<name>__<tool>`.
    pub name: String,
    pub transport: McpTransport,
    /// Stdio only: the program, found on the tool user's PATH.
    #[serde(default)]
    pub command: Option<String>,
    /// Stdio only.
    #[serde(default)]
    pub args: Vec<String>,
    /// HTTP only: an `https://` or `http://` endpoint.
    #[serde(default)]
    pub url: Option<String>,
    /// Environment variables (stdio) or request headers (HTTP), stored encrypted. Required on
    /// create, where it may be empty; an update without it keeps the current ones, unless the
    /// transport changes.
    #[serde(default)]
    pub secrets: Option<BTreeMap<String, String>>,
    /// All agents when absent on create, unchanged on update.
    #[serde(default)]
    pub access: Option<ProviderAccess>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct McpServerSummary {
    pub id: Uuid,
    pub name: String,
    pub transport: McpTransport,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    /// The environment variable or header names; their values are never returned.
    pub secret_names: Vec<String>,
    #[schema(value_type = ProviderAccess)]
    pub access: Access,
}

/// A server as one agent's Connections tab shows it.
#[derive(Serialize, utoipa::ToSchema)]
pub struct AgentMcpServer {
    pub id: Uuid,
    pub name: String,
    pub transport: McpTransport,
    /// Every agent may use it; access is changed on the server, not per agent.
    pub all_agents: bool,
    /// This agent may use it.
    pub enabled: bool,
}

struct Row {
    id: Uuid,
    name: String,
    transport: String,
    command: Option<String>,
    args: sqlx::types::Json<Vec<String>>,
    url: Option<String>,
    secret_names: sqlx::types::Json<Vec<String>>,
    access: Access,
}

impl From<Row> for McpServerSummary {
    fn from(row: Row) -> Self {
        Self {
            id: row.id,
            name: row.name,
            transport: transport(&row.transport),
            command: row.command,
            args: row.args.0,
            url: row.url,
            secret_names: row.secret_names.0,
            access: row.access,
        }
    }
}

fn transport(stored: &str) -> McpTransport {
    match stored {
        "stdio" => McpTransport::Stdio,
        _ => McpTransport::Http,
    }
}

fn stored(transport: McpTransport) -> &'static str {
    match transport {
        McpTransport::Stdio => "stdio",
        McpTransport::Http => "http",
    }
}

/// Why a server cannot be saved, in words the console shows.
pub fn problem(input: &SaveMcpServer) -> Option<&'static str> {
    let name = &input.name;
    if name.is_empty()
        || name.len() > 32
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Some("a name is 1 to 32 letters, digits or hyphens");
    }
    match input.transport {
        McpTransport::Stdio => stdio_problem(input),
        McpTransport::Http => http_problem(input),
    }
    .or_else(|| secrets_problem(input))
}

fn plain(text: &str, limit: usize) -> bool {
    !text.trim().is_empty() && text.len() <= limit && !text.contains('\0')
}

fn stdio_problem(input: &SaveMcpServer) -> Option<&'static str> {
    if input.url.is_some() || !input.command.as_deref().is_some_and(|c| plain(c, 4096)) {
        return Some("a stdio server has a command and no URL");
    }
    let args = input.args.len() <= 64
        && input
            .args
            .iter()
            .all(|arg| arg.len() <= 4096 && !arg.contains('\0'));
    (!args).then_some("a stdio server has at most 64 arguments of up to 4096 bytes")
}

fn http_problem(input: &SaveMcpServer) -> Option<&'static str> {
    let url = input.url.as_deref().unwrap_or_default();
    let web = url.starts_with("https://") || url.starts_with("http://");
    if input.command.is_some()
        || !input.args.is_empty()
        || !web
        || !plain(url, 2048)
        || url.contains(char::is_whitespace)
    {
        return Some("an HTTP server has an http:// or https:// URL and no command");
    }
    None
}

fn secrets_problem(input: &SaveMcpServer) -> Option<&'static str> {
    let secrets = input.secrets.as_ref()?;
    let stdio = input.transport == McpTransport::Stdio;
    let named = |name: &str| {
        let starts = name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_');
        let rest = name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || (!stdio && b == b'-'));
        name.len() <= 128 && (starts || !stdio) && !name.is_empty() && rest
    };
    let valued = |value: &String| value.len() <= 8192 && !value.contains(['\0', '\r', '\n']);
    if secrets.len() > 32
        || !secrets
            .iter()
            .all(|(name, value)| named(name) && valued(value))
    {
        return Some(if stdio {
            "environment variables are named like SHELL_NAMES, at most 32, with single-line values"
        } else {
            "headers are named with letters, digits and hyphens, at most 32, with single-line values"
        });
    }
    None
}

impl Connections {
    async fn seal_secrets(
        &self,
        server: Uuid,
        secrets: &BTreeMap<String, String>,
    ) -> Result<Vec<u8>, IdentityError> {
        let context = self.sealing_context("mcp", server).await?;
        let plain = serde_json::to_vec(secrets).map_err(|_| IdentityError::InvalidInput)?;
        self.credentials
            .encrypt(&plain, &context)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))
    }

    async fn open_secrets(
        &self,
        server: Uuid,
        sealed: &[u8],
    ) -> Result<BTreeMap<String, String>, IdentityError> {
        let context = self.sealing_context("mcp", server).await?;
        let plain = self
            .credentials
            .decrypt(sealed, &context)
            .map_err(|error| IdentityError::CredentialUnavailable(Some(error.into())))?;
        serde_json::from_slice(&plain).map_err(|_| IdentityError::CredentialUnavailable(None))
    }

    /// Every server; the caller is an administrator.
    #[tracing::instrument(skip_all)]
    pub async fn mcp_servers(&self) -> Result<Vec<McpServerSummary>, IdentityError> {
        Ok(sqlx::query_as!(
            Row,
            r#"
            SELECT id, name, transport, command, args AS "args!: _", url,
                secret_names AS "secret_names!: _", access AS "access!: Access"
            FROM mcp_servers
            ORDER BY name
            "#,
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(McpServerSummary::from)
        .collect())
    }

    /// The caller is an administrator.
    #[tracing::instrument(skip_all, fields(actor = %actor))]
    pub async fn create_mcp_server(
        &self,
        actor: Uuid,
        input: &SaveMcpServer,
    ) -> Result<McpServerSummary, IdentityError> {
        let Some(secrets) = input.secrets.as_ref().filter(|_| problem(input).is_none()) else {
            return Err(IdentityError::InvalidInput);
        };
        let access = input.access.clone().unwrap_or_default();
        let mut tx = self.pool.begin().await?;
        validate_access(&mut tx, &access).await?;
        let id = Uuid::new_v4();
        let ciphertext = self.seal_secrets(id, secrets).await?;
        let names: Vec<&String> = secrets.keys().collect();
        let row = sqlx::query_as!(
            Row,
            r#"
            INSERT INTO mcp_servers (id, name, transport, command, args, url, secret_names, ciphertext, access, created_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (name) DO NOTHING
            RETURNING id, name, transport, command, args AS "args!: _", url,
                secret_names AS "secret_names!: _", access AS "access!: Access"
            "#,
            id,
            input.name,
            stored(input.transport),
            input.command,
            sqlx::types::Json(&input.args) as _,
            input.url,
            sqlx::types::Json(&names) as _,
            ciphertext,
            sqlx::types::Json(&access) as _,
            actor,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::Conflict)?;
        tx.commit().await?;
        Ok(row.into())
    }

    /// The caller is an administrator. Changing the transport needs new secrets, since names
    /// of environment variables are not names of headers.
    #[tracing::instrument(skip_all, fields(server = %server))]
    pub async fn update_mcp_server(
        &self,
        server: Uuid,
        input: &SaveMcpServer,
    ) -> Result<McpServerSummary, IdentityError> {
        if problem(input).is_some() {
            return Err(IdentityError::InvalidInput);
        }
        let mut tx = self.pool.begin().await?;
        let current = sqlx::query_scalar!(
            "SELECT transport FROM mcp_servers WHERE id=$1 FOR UPDATE",
            server
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        if current != stored(input.transport) && input.secrets.is_none() {
            return Err(IdentityError::InvalidInput);
        }
        if let Some(access) = &input.access {
            validate_access(&mut tx, access).await?;
        }
        let (ciphertext, names) = match &input.secrets {
            Some(secrets) => (
                Some(self.seal_secrets(server, secrets).await?),
                Some(secrets.keys().collect::<Vec<_>>()),
            ),
            None => (None, None),
        };
        let row = sqlx::query_as!(
            Row,
            r#"
            UPDATE mcp_servers
            SET name=$2, transport=$3, command=$4, args=$5, url=$6,
                ciphertext=COALESCE($7, ciphertext), secret_names=COALESCE($8, secret_names),
                access=COALESCE($9, access), updated_at=clock_timestamp()
            WHERE id=$1 AND NOT EXISTS(SELECT 1 FROM mcp_servers o WHERE o.name=$2 AND o.id<>$1)
            RETURNING id, name, transport, command, args AS "args!: _", url,
                secret_names AS "secret_names!: _", access AS "access!: Access"
            "#,
            server,
            input.name,
            stored(input.transport),
            input.command,
            sqlx::types::Json(&input.args) as _,
            input.url,
            ciphertext,
            names.map(sqlx::types::Json) as _,
            input.access.as_ref().map(sqlx::types::Json) as _,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::Conflict)?;
        tx.commit().await?;
        Ok(row.into())
    }

    #[tracing::instrument(skip_all, fields(server = %server))]
    pub async fn delete_mcp_server(&self, server: Uuid) -> Result<(), IdentityError> {
        let deleted = sqlx::query!("DELETE FROM mcp_servers WHERE id=$1", server)
            .execute(&self.pool)
            .await?;
        if deleted.rows_affected() == 0 {
            return Err(IdentityError::NotFound);
        }
        Ok(())
    }

    /// Every server with whether `agent` may use it.
    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub async fn agent_mcp_servers(
        &self,
        agent: AgentId,
    ) -> Result<Vec<AgentMcpServer>, IdentityError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, name, transport, access->>'mode' = 'all_agents' AS "all_agents!",
                (access->>'mode' = 'all_agents' OR access->'agents' @> jsonb_build_array($1::uuid)) AS "enabled!"
            FROM mcp_servers
            ORDER BY name
            "#,
            agent.0,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| AgentMcpServer {
                id: row.id,
                name: row.name,
                transport: transport(&row.transport),
                all_agents: row.all_agents,
                enabled: row.enabled,
            })
            .collect())
    }

    /// Grants or withdraws one agent's use of a server whose access is explicit grants; a
    /// server shared with every agent is changed on the server itself. The caller is an
    /// administrator.
    #[tracing::instrument(skip_all, fields(server = %server, agent = %agent.0))]
    pub async fn grant_mcp_server(
        &self,
        server: Uuid,
        agent: AgentId,
        enabled: bool,
    ) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        let access = ProviderAccess::ExplicitGrants {
            agents: vec![agent.0],
        };
        validate_access(&mut tx, &access).await?;
        let mode = sqlx::query_scalar!(
            r#"SELECT access->>'mode' AS "mode!" FROM mcp_servers WHERE id=$1 FOR UPDATE"#,
            server,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::NotFound)?;
        if mode != "explicit_grants" {
            return Err(IdentityError::Conflict);
        }
        sqlx::query!(
            r#"
            UPDATE mcp_servers
            SET access=jsonb_build_object('mode', 'explicit_grants', 'agents', (
                    SELECT coalesce(jsonb_agg(DISTINCT granted), '[]')
                    FROM jsonb_array_elements(access->'agents' || jsonb_build_array($2::uuid)) granted
                    WHERE $3 OR granted <> to_jsonb($2::uuid)
                )),
                updated_at=clock_timestamp()
            WHERE id=$1
            "#,
            server,
            agent.0,
            enabled,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The servers `agent` may use, with their secrets, for one turn. Never log them.
    pub(crate) async fn runtime_mcp(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        agent: AgentId,
    ) -> Result<Vec<McpServer>, IdentityError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, name, transport, command, args AS "args!: sqlx::types::Json<Vec<String>>", url, ciphertext
            FROM mcp_servers
            WHERE access->>'mode' = 'all_agents' OR access->'agents' @> jsonb_build_array($1::uuid)
            ORDER BY name
            "#,
            agent.0,
        )
        .fetch_all(&mut **tx)
        .await?;
        let mut servers = Vec::with_capacity(rows.len());
        for row in rows {
            let secrets = self.open_secrets(row.id, &row.ciphertext).await?;
            servers.push(match (row.command, row.url) {
                (Some(command), _) => McpServer::Stdio {
                    name: row.name,
                    command,
                    args: row.args.0,
                    env: secrets,
                },
                (None, url) => McpServer::Http {
                    name: row.name,
                    url: url.unwrap_or_default(),
                    headers: secrets,
                },
            });
        }
        Ok(servers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(transport: McpTransport) -> SaveMcpServer {
        SaveMcpServer {
            name: "github".into(),
            transport,
            command: None,
            args: Vec::new(),
            url: None,
            secrets: Some(BTreeMap::new()),
            access: None,
        }
    }

    #[test]
    fn servers_need_a_name_and_their_transport_fields() {
        let mut stdio = server(McpTransport::Stdio);
        assert!(problem(&stdio).is_some(), "no command");
        stdio.command = Some("npx".into());
        stdio.args = vec!["-y".into(), "@modelcontextprotocol/server-github".into()];
        stdio.secrets = Some([("GITHUB_TOKEN".into(), "t".into())].into());
        assert_eq!(problem(&stdio), None);
        stdio.name = "a__b".into();
        assert!(
            problem(&stdio).is_some(),
            "underscores would split tool names"
        );
        stdio.name = "github".into();
        stdio.secrets = Some([("1BAD".into(), "t".into())].into());
        assert!(problem(&stdio).is_some(), "not a shell name");

        let mut http = server(McpTransport::Http);
        http.url = Some("ftp://mcp.example".into());
        assert!(problem(&http).is_some());
        http.url = Some("https://mcp.example/mcp".into());
        http.secrets = Some([("Authorization".into(), "Bearer t".into())].into());
        assert_eq!(problem(&http), None);
        http.secrets = Some([("Authorization".into(), "Bearer t\r\nX: y".into())].into());
        assert!(problem(&http).is_some(), "header injection");
        http.secrets = None;
        http.command = Some("npx".into());
        assert!(problem(&http).is_some(), "HTTP takes no command");
    }
}

mod relay;
mod tool;

pub use relay::Links;

use crate::{
    agents::AgentId,
    identity::{
        Actor, Identity, IdentityError,
        access::{Capability, Scope},
        audit::{self, Event},
    },
    work::access,
};
use cubebot_protocol::apps::{
    AppNotice, AppReport, AppSpec, Desired, LOCAL_PORT, Reported, valid_folder,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

type Answer<T> = Result<T, String>;

#[derive(Clone)]
pub struct Apps {
    pool: PgPool,
    identity: Identity,
    links: Links,
    views: relay::Views,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AppState {
    Starting,
    Running,
    Stopped,
    Updating,
    Failed,
    Unavailable,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct App {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub agent_id: Uuid,
    pub agent_name: String,
    pub connections: Vec<String>,
    pub state: AppState,
    pub memory: Option<u64>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Register {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub folder: String,
    pub command: Option<String>,
    pub port: Option<u16>,
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub slug: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub folder: Option<String>,
    pub command: Option<String>,
    pub port: Option<u16>,
    pub tools: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentState {
    pub desired: Option<Desired>,
    pub updating: Option<bool>,
}

struct Row {
    id: Uuid,
    slug: String,
    name: String,
    description: String,
    icon: String,
    agent_id: Uuid,
    agent_name: String,
    tools: Vec<String>,
    desired: String,
    updating: bool,
    reported: String,
    memory: Option<i64>,
    updated_at: String,
}

impl From<Row> for App {
    fn from(row: Row) -> Self {
        let state = state(&row.desired, row.updating, &row.reported);
        Self {
            id: row.id,
            connections: connections(&row.tools),
            slug: row.slug,
            name: row.name,
            description: row.description,
            icon: row.icon,
            agent_id: row.agent_id,
            agent_name: row.agent_name,
            state,
            memory: row.memory.and_then(|memory| u64::try_from(memory).ok()),
            updated_at: row.updated_at,
        }
    }
}

fn state(desired: &str, updating: bool, reported: &str) -> AppState {
    match (updating, desired, reported) {
        (true, _, _) => AppState::Updating,
        (_, "stopped", _) => AppState::Stopped,
        (_, _, "running") => AppState::Running,
        (_, _, "stopped") => AppState::Stopped,
        (_, _, "failed") => AppState::Failed,
        _ => AppState::Starting,
    }
}

fn connections(tools: &[String]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for tool in tools {
        let name = tool
            .split_once("__")
            .map_or(tool.as_str(), |(name, _)| name);
        if !names.iter().any(|known| known == name) {
            names.push(name.to_owned());
        }
    }
    names
}

fn desired_text(desired: Desired) -> &'static str {
    match desired {
        Desired::Running => "running",
        Desired::Stopped => "stopped",
    }
}

fn reported_text(reported: Reported) -> &'static str {
    match reported {
        Reported::Starting => "starting",
        Reported::Running => "running",
        Reported::Stopped => "stopped",
        Reported::Failed => "failed",
    }
}

fn valid_slug(slug: &str) -> bool {
    let bytes = slug.as_bytes();
    (2..=40).contains(&bytes.len())
        && bytes[0] != b'-'
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn valid_tool(tool: &str) -> bool {
    tool.len() <= 128
        && tool
            .split_once("__")
            .is_some_and(|(connection, name)| !connection.is_empty() && !name.is_empty())
}

fn invalid_text(input: &Register) -> Option<&'static str> {
    let name = input.name.chars().count();
    if !valid_slug(&input.slug) {
        return Some(
            "slug is 2 to 40 lowercase letters, digits or hyphens, not starting with a hyphen",
        );
    }
    if !(1..=60).contains(&name) || input.name.trim().is_empty() {
        return Some("name is 1 to 60 characters");
    }
    if input.description.chars().count() > 300 || input.icon.chars().count() > 16 {
        return Some("description is at most 300 characters and icon is one emoji");
    }
    None
}

fn invalid_run(input: &Register) -> Option<&'static str> {
    if !valid_folder(&input.folder) {
        return Some(
            "folder is a relative path inside the workspace with no empty, . or .. part, no part starting with a dot and no backslash",
        );
    }
    if input.command.is_some() != input.port.is_some() {
        return Some("command and port are given together, or neither for a static app");
    }
    if input.command.as_ref().is_some_and(|command| {
        command.trim().is_empty() || command.len() > 4096 || command.contains('\0')
    }) {
        return Some("command is 1 to 4096 bytes");
    }
    if input
        .port
        .is_some_and(|port| port < 1024 || port == LOCAL_PORT)
    {
        return Some("port is 1024 to 65535 and not 7400");
    }
    if input.tools.len() > 64 || !input.tools.iter().all(|tool| valid_tool(tool)) {
        return Some("tools are at most 64 connection tool names such as whoop__recovery");
    }
    None
}

fn invalid(input: &Register) -> Option<&'static str> {
    invalid_text(input).or_else(|| invalid_run(input))
}

fn changed(current: Register, change: Change) -> Register {
    Register {
        slug: change.slug,
        name: change.name.unwrap_or(current.name),
        description: change.description.unwrap_or(current.description),
        icon: change.icon.unwrap_or(current.icon),
        folder: change.folder.unwrap_or(current.folder),
        command: change.command.or(current.command),
        port: change.port.or(current.port),
        tools: change.tools.unwrap_or(current.tools),
    }
}

impl Apps {
    pub fn new(pool: PgPool, identity: Identity) -> Self {
        Self {
            pool,
            identity,
            links: Links::default(),
            views: relay::Views::default(),
        }
    }

    pub fn links(&self) -> &Links {
        &self.links
    }

    fn shown(&self, row: Row) -> App {
        let mut app = App::from(row);
        if !matches!(app.state, AppState::Updating | AppState::Stopped)
            && !self.links.connected(AgentId(app.agent_id))
        {
            app.state = AppState::Unavailable;
        }
        app
    }

    fn changed(&self, agent: AgentId) {
        self.links.notify(agent, AppNotice::Changed);
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn list(&self, actor: &Actor) -> Result<Vec<App>, IdentityError> {
        let rows = sqlx::query_as!(
            Row,
            r#"
            SELECT p.id, p.slug, p.name, p.description, p.icon, p.agent_id, g.name AS agent_name,
                p.tools, p.desired, p.updating, p.reported, p.memory,
                (to_jsonb(p.updated_at) #>> '{}') AS "updated_at!"
            FROM apps p
            JOIN agents g ON g.id = p.agent_id AND g.desired_state <> 'retired'
            JOIN human_accounts a ON a.id = $1 AND a.active
            WHERE a.role = 'administrator' OR EXISTS(
                SELECT 1 FROM effective_human_grants e
                WHERE e.actor_id = a.id AND e.scope_kind = 'conversation'
                    AND e.scope_id = p.agent_id AND e.capability = 'conversation.send'
            )
            ORDER BY lower(p.name), p.slug
            "#,
            actor.id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|row| self.shown(row)).collect())
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, slug))]
    pub async fn set_desired(
        &self,
        actor: &Actor,
        slug: &str,
        desired: Desired,
    ) -> Result<App, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let (id, agent) = live(&mut tx, slug).await?;
        access::conversation(&mut tx, actor, agent, "conversation.send").await?;
        sqlx::query!(
            "UPDATE apps SET desired = $2, updated_at = now() WHERE id = $1",
            id,
            desired_text(desired),
        )
        .execute(&mut *tx)
        .await?;
        let event = Event::by(actor.id, "app.state")
            .on(id)
            .detail(json!({"slug": slug, "agent": agent, "desired": desired}));
        audit::record(&mut *tx, &event).await?;
        tx.commit().await?;
        self.changed(AgentId(agent));
        self.one(slug).await?.ok_or(IdentityError::NotFound)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, slug))]
    pub async fn remove(&self, actor: &Actor, slug: &str) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        let (id, agent) = live(&mut tx, slug).await?;
        access::conversation(&mut tx, actor, agent, "conversation.send").await?;
        self.identity
            .authorize(actor, Scope::Agent { id: agent }, Capability::AgentManage)
            .await?;
        sqlx::query!("DELETE FROM apps WHERE id = $1", id)
            .execute(&mut *tx)
            .await?;
        let event = Event::by(actor.id, "app.remove")
            .on(id)
            .detail(json!({"slug": slug, "agent": agent}));
        audit::record(&mut *tx, &event).await?;
        tx.commit().await?;
        self.changed(AgentId(agent));
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub async fn register(
        &self,
        agent: AgentId,
        input: Register,
    ) -> Result<Answer<App>, IdentityError> {
        if let Some(reason) = invalid(&input) {
            return Ok(Err(reason.into()));
        }
        let inserted = sqlx::query!(
            "
            INSERT INTO apps (id, agent_id, slug, name, description, icon, folder, command, port, tools)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ",
            Uuid::new_v4(),
            agent.0,
            input.slug,
            input.name,
            input.description,
            input.icon,
            input.folder,
            input.command,
            input.port.map(i32::from),
            &input.tools,
        )
        .execute(&self.pool)
        .await;
        match inserted {
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => Ok(Err(format!(
                "the slug {} is taken; pick another",
                input.slug
            ))),
            Err(error) => Err(error.into()),
            Ok(_) => {
                self.changed(agent);
                self.owned(agent, &input.slug).await
            }
        }
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub async fn update(
        &self,
        agent: AgentId,
        change: Change,
    ) -> Result<Answer<App>, IdentityError> {
        let current = sqlx::query!(
            "
            SELECT slug, name, description, icon, folder, command, port, tools
            FROM apps
            WHERE agent_id = $1 AND slug = $2
            ",
            agent.0,
            change.slug,
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(current) = current else {
            return Ok(Err(unknown(&change.slug)));
        };
        let current = Register {
            slug: current.slug,
            name: current.name,
            description: current.description,
            icon: current.icon,
            folder: current.folder,
            command: current.command,
            port: current.port.and_then(|port| u16::try_from(port).ok()),
            tools: current.tools,
        };
        let input = changed(current, change);
        if let Some(reason) = invalid(&input) {
            return Ok(Err(reason.into()));
        }
        sqlx::query!(
            "
            UPDATE apps
            SET name = $3, description = $4, icon = $5, folder = $6, command = $7, port = $8,
                tools = $9, updated_at = now()
            WHERE agent_id = $1 AND slug = $2
            ",
            agent.0,
            input.slug,
            input.name,
            input.description,
            input.icon,
            input.folder,
            input.command,
            input.port.map(i32::from),
            &input.tools,
        )
        .execute(&self.pool)
        .await?;
        self.changed(agent);
        self.owned(agent, &input.slug).await
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0, slug))]
    pub async fn agent_state(
        &self,
        agent: AgentId,
        slug: &str,
        input: AgentState,
    ) -> Result<Answer<App>, IdentityError> {
        let changed = sqlx::query!(
            "
            UPDATE apps
            SET desired = COALESCE($3, desired), updating = COALESCE($4, updating), updated_at = now()
            WHERE agent_id = $1 AND slug = $2
            ",
            agent.0,
            slug,
            input.desired.map(desired_text),
            input.updating,
        )
        .execute(&self.pool)
        .await?;
        if changed.rows_affected() == 0 {
            return Ok(Err(unknown(slug)));
        }
        self.changed(agent);
        self.owned(agent, slug).await
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0, slug))]
    pub async fn unregister(
        &self,
        agent: AgentId,
        slug: &str,
    ) -> Result<Answer<()>, IdentityError> {
        let removed = sqlx::query!(
            "DELETE FROM apps WHERE agent_id = $1 AND slug = $2",
            agent.0,
            slug,
        )
        .execute(&self.pool)
        .await?;
        if removed.rows_affected() == 0 {
            return Ok(Err(unknown(slug)));
        }
        self.changed(agent);
        Ok(Ok(()))
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub async fn specs(&self, agent: AgentId) -> Result<Vec<AppSpec>, IdentityError> {
        let rows = sqlx::query!(
            "
            SELECT slug, name, folder, command, port, tools, desired
            FROM apps
            WHERE agent_id = $1
            ORDER BY slug
            ",
            agent.0,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| AppSpec {
                slug: row.slug,
                name: row.name,
                folder: row.folder,
                command: row.command,
                port: row.port.and_then(|port| u16::try_from(port).ok()),
                tools: row.tools,
                desired: if row.desired == "stopped" {
                    Desired::Stopped
                } else {
                    Desired::Running
                },
            })
            .collect())
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub async fn report(&self, agent: AgentId, report: AppReport) -> Result<(), IdentityError> {
        if !reasonable(&report) {
            return Ok(());
        }
        match report {
            AppReport::State {
                slug,
                state,
                memory,
            } => {
                sqlx::query!(
                    "
                    UPDATE apps
                    SET reported = $3, memory = $4,
                        updated_at = CASE WHEN reported = $3 THEN updated_at ELSE now() END
                    WHERE agent_id = $1 AND slug = $2
                    ",
                    agent.0,
                    slug,
                    reported_text(state),
                    memory.and_then(|memory| i64::try_from(memory).ok()),
                )
                .execute(&self.pool)
                .await?;
            }
            AppReport::Called {
                slug,
                tool,
                viewer,
                ok,
            } => {
                let id = sqlx::query_scalar!(
                    "SELECT id FROM apps WHERE agent_id = $1 AND slug = $2",
                    agent.0,
                    slug,
                )
                .fetch_optional(&self.pool)
                .await?;
                if let Some(id) = id {
                    let event = Event::by_agent(agent.0, "app.call")
                        .on(id)
                        .detail(json!({"slug": slug, "tool": tool, "viewer": viewer, "ok": ok}));
                    audit::record(&self.pool, &event).await?;
                }
            }
        }
        Ok(())
    }

    async fn owned(&self, agent: AgentId, slug: &str) -> Result<Answer<App>, IdentityError> {
        Ok(match self.one(slug).await? {
            Some(app) if app.agent_id == agent.0 => Ok(app),
            _ => Err(unknown(slug)),
        })
    }

    async fn one(&self, slug: &str) -> Result<Option<App>, IdentityError> {
        let row = sqlx::query_as!(
            Row,
            r#"
            SELECT p.id, p.slug, p.name, p.description, p.icon, p.agent_id, g.name AS agent_name,
                p.tools, p.desired, p.updating, p.reported, p.memory,
                (to_jsonb(p.updated_at) #>> '{}') AS "updated_at!"
            FROM apps p
            JOIN agents g ON g.id = p.agent_id
            WHERE p.slug = $1
            "#,
            slug,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| self.shown(row)))
    }
}

fn reasonable(report: &AppReport) -> bool {
    match report {
        AppReport::State { slug, .. } => slug.len() <= 40,
        AppReport::Called { slug, tool, .. } => slug.len() <= 40 && tool.chars().count() <= 200,
    }
}

fn unknown(slug: &str) -> String {
    format!("you have no app {slug}")
}

async fn live(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    slug: &str,
) -> Result<(Uuid, Uuid), IdentityError> {
    let row = sqlx::query!(
        "
        SELECT p.id, p.agent_id
        FROM apps p
        JOIN agents g ON g.id = p.agent_id AND g.desired_state <> 'retired'
        WHERE p.slug = $1
        FOR UPDATE OF p
        ",
        slug,
    )
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(IdentityError::NotFound)?;
    Ok((row.id, row.agent_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_prefers_updating_then_stopped_then_the_report() {
        assert_eq!(state("stopped", true, "running"), AppState::Updating);
        assert_eq!(state("stopped", false, "running"), AppState::Stopped);
        assert_eq!(state("running", false, "failed"), AppState::Failed);
        assert_eq!(state("running", false, "starting"), AppState::Starting);
        assert_eq!(
            connections(&["whoop__a".into(), "mail__b".into(), "whoop__c".into()]),
            ["whoop", "mail"]
        );
    }
}

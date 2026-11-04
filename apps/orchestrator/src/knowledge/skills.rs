//! The shared skill library (docs/adr/memory.md#reusable-skills-created-by-humans-agents-and-dreaming, docs/adr/memory.md#shared-installation-skill-library-by-default and docs/adr/memory.md#shared-skill-revisions-by-any-agent-or-user). Any active person publishes and revises skills;
//! a revision names the one it read, so concurrent contributors get a conflict instead of
//! overwriting each other. Installing pins a revision per agent, and a new revision never moves
//! another agent's installation. Storing a package never runs its scripts.
use super::{Knowledge, authorize};
use crate::{
    agents::{AgentId, runtime::RuntimeActor},
    identity::{Actor, IdentityError},
};
use cubebot_protocol::runtime::{
    LibrarySkill, RoutineCall, SKILL_BYTES, SkillChange, SkillPackage,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction, types::Json};
use std::collections::BTreeMap;
use uuid::Uuid;

pub type Files = BTreeMap<String, String>;

/// Installed skills a dream sees in full.
/// shortcut: the first 20 by name, which bounds the dream's prompt; select by relevance once
/// agents install more.
pub(super) const DREAM_SKILLS: i64 = 20;

/// Skills that ship with the platform, each one `SKILL.md`. People edit them like any other.
const SHIPPED: [&str; 2] = [
    include_str!("../../skills/coding/SKILL.md"),
    include_str!("../../skills/design/SKILL.md"),
];

/// A library entry at its current revision.
#[derive(Serialize, utoipa::ToSchema)]
pub struct Skill {
    pub id: Uuid,
    pub name: String,
    pub revision: i64,
    pub description: String,
    pub updated_at: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct SkillRevision {
    pub skill: Uuid,
    pub name: String,
    pub revision: i64,
    pub description: String,
    pub files: Files,
    /// The person who wrote this revision, or `None` for a dream's.
    pub human: Option<Uuid>,
    /// The dream run that proposed this revision.
    pub dream_run: Option<Uuid>,
    /// The agent that wrote this revision during its work.
    pub agent: Option<String>,
    pub created_at: String,
}

/// An agent's installation, with the library's current revision to show available updates.
#[derive(Serialize, utoipa::ToSchema)]
pub struct InstalledSkill {
    pub skill: Uuid,
    pub name: String,
    pub revision: i64,
    pub current_revision: i64,
    pub description: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishSkill {
    pub files: Files,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviseSkill {
    pub files: Files,
    pub expected_revision: i64,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallSkill {
    pub revision: i64,
}

impl Knowledge {
    /// shortcut: the whole library by name; page once installations hold hundreds of skills.
    #[tracing::instrument(skip_all)]
    pub async fn skills(&self, _actor: &Actor) -> Result<Vec<Skill>, IdentityError> {
        Ok(sqlx::query_as!(
            Skill,
            r#"
            SELECT s.id, s.name, s.revision, r.description, (to_jsonb(r.created_at) #>> '{}') AS "updated_at!"
            FROM skills s
            JOIN skill_revisions r ON r.skill_id=s.id AND r.revision=s.revision
            ORDER BY s.name
            "#
        )
        .fetch_all(&self.pool)
        .await?)
    }

    #[tracing::instrument(skip_all, fields(skill = %skill, revision = revision))]
    pub async fn skill_revision(
        &self,
        _actor: &Actor,
        skill: Uuid,
        revision: i64,
    ) -> Result<SkillRevision, IdentityError> {
        let found = sqlx::query!(
            r#"
            SELECT s.id AS skill, s.name, r.revision, r.description, r.files AS "files: Json<Files>", r.human_id AS human, r.dream_run_id AS dream_run, g.name AS "agent?", (to_jsonb(r.created_at) #>> '{}') AS "created_at!"
            FROM skill_revisions r
            JOIN skills s ON s.id=r.skill_id
            LEFT JOIN chat_turns t ON t.id=r.turn_id
            LEFT JOIN agents g ON g.id=t.conversation_id
            WHERE r.skill_id=$1 AND r.revision=$2
            "#,
            skill,
            revision,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::NotFound)?;
        Ok(SkillRevision {
            skill: found.skill,
            name: found.name,
            revision: found.revision,
            description: found.description,
            files: found.files.0,
            human: found.human,
            dream_run: found.dream_run,
            agent: found.agent,
            created_at: found.created_at,
        })
    }

    /// Registers a new skill; 409 when its name is taken.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn publish_skill(
        &self,
        actor: &Actor,
        input: &PublishSkill,
    ) -> Result<Skill, IdentityError> {
        self.contribute(actor, None, None, &input.files).await
    }

    /// Adds a revision on top of `expected_revision`; 409 when that is no longer current. The
    /// package keeps the skill's name.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, skill = %skill))]
    pub async fn revise_skill(
        &self,
        actor: &Actor,
        skill: Uuid,
        input: &ReviseSkill,
    ) -> Result<Skill, IdentityError> {
        let expected = Some(input.expected_revision);
        self.contribute(actor, Some(skill), expected, &input.files)
            .await
    }

    async fn contribute(
        &self,
        actor: &Actor,
        skill: Option<Uuid>,
        expected: Option<i64>,
        files: &Files,
    ) -> Result<Skill, IdentityError> {
        let metadata = package(files).ok_or(IdentityError::InvalidInput)?;
        let mut tx = self.pool.begin().await?;
        if let Some(skill) = skill {
            let name = sqlx::query_scalar!(
                "
                SELECT name
                FROM skills
                WHERE id=$1
                ",
                skill,
            )
            .fetch_optional(&mut *tx)
            .await?;
            if name.ok_or(IdentityError::NotFound)? != metadata.0 {
                return Err(IdentityError::InvalidInput);
            }
        }
        let author = Author {
            human: Some(actor.id),
            ..Author::default()
        };
        let (id, _) = write(&mut tx, skill, expected, &metadata, files, author)
            .await?
            .ok_or(IdentityError::Conflict)?;
        let written = sqlx::query_as!(
            Skill,
            r#"
            SELECT s.id, s.name, s.revision, r.description, (to_jsonb(r.created_at) #>> '{}') AS "updated_at!"
            FROM skills s
            JOIN skill_revisions r ON r.skill_id=s.id AND r.revision=s.revision
            WHERE s.id=$1
            "#,
            id,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(written)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, agent = %agent.0))]
    pub async fn installed_skills(
        &self,
        actor: &Actor,
        agent: AgentId,
    ) -> Result<Vec<InstalledSkill>, IdentityError> {
        let mut tx = self.pool.begin().await?;
        authorize(&mut tx, actor, agent, "memory.read").await?;
        let installed = sqlx::query_as!(
            InstalledSkill,
            "
            SELECT s.id AS skill, s.name, i.revision, s.revision AS current_revision, r.description
            FROM skill_installations i
            JOIN skills s ON s.id=i.skill_id
            JOIN skill_revisions r ON r.skill_id=i.skill_id AND r.revision=i.revision
            WHERE i.agent_id=$1
            ORDER BY s.name
            ",
            agent.0,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(installed)
    }

    /// Installs a revision, or moves an installation to it.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, agent = %agent.0, skill = %skill, revision = revision))]
    pub async fn install_skill(
        &self,
        actor: &Actor,
        agent: AgentId,
        skill: Uuid,
        revision: i64,
    ) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        authorize(&mut tx, actor, agent, "agent.manage").await?;
        if !install(&mut tx, agent, skill, revision).await? {
            return Err(IdentityError::NotFound);
        }
        tx.commit().await?;
        Ok(())
    }

    /// Removes the installation; the skill stays in the library.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, agent = %agent.0, skill = %skill))]
    pub async fn uninstall_skill(
        &self,
        actor: &Actor,
        agent: AgentId,
        skill: Uuid,
    ) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        authorize(&mut tx, actor, agent, "agent.manage").await?;
        sqlx::query!(
            "
            DELETE FROM skill_installations
            WHERE agent_id=$1 AND skill_id=$2
            ",
            agent.0,
            skill,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

/// `false` when the revision does not exist.
pub(super) async fn install(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    skill: Uuid,
    revision: i64,
) -> Result<bool, sqlx::Error> {
    let installed = sqlx::query!(
        "
        INSERT INTO skill_installations (agent_id, skill_id, revision)
        SELECT $1, $2, $3
        WHERE EXISTS(SELECT 1 FROM skill_revisions WHERE skill_id=$2 AND revision=$3)
        ON CONFLICT (agent_id,skill_id) DO UPDATE SET revision=EXCLUDED.revision, installed_at=clock_timestamp()
        ",
        agent.0,
        skill,
        revision,
    )
    .execute(&mut **tx)
    .await?;
    Ok(installed.rows_affected() == 1)
}

/// Who wrote a revision. Nobody: it shipped with the platform.
#[derive(Clone, Copy, Default)]
pub(super) struct Author {
    pub human: Option<Uuid>,
    pub dream: Option<Uuid>,
    pub turn: Option<Uuid>,
}

/// Creates a skill, or adds a revision on top of `expected`. `None` when the name is taken or
/// `expected` is not current.
pub(super) async fn write(
    tx: &mut Transaction<'_, Postgres>,
    skill: Option<Uuid>,
    expected: Option<i64>,
    metadata: &(String, String),
    files: &Files,
    author: Author,
) -> Result<Option<(Uuid, i64)>, sqlx::Error> {
    match skill {
        None => register(tx, metadata, files, author).await,
        Some(skill) => add_revision(tx, skill, expected, metadata, files, author).await,
    }
}

/// A new skill with its first revision; `None` when the name is taken.
async fn register(
    tx: &mut Transaction<'_, Postgres>,
    (name, description): &(String, String),
    files: &Files,
    author: Author,
) -> Result<Option<(Uuid, i64)>, sqlx::Error> {
    let written = sqlx::query!(
        "
        WITH s AS (
            INSERT INTO skills (name)
            VALUES ($1)
            ON CONFLICT (name) DO NOTHING
            RETURNING id, revision
        )
        INSERT INTO skill_revisions (skill_id, revision, description, files, human_id, dream_run_id, turn_id)
        SELECT id, revision, $2, $3, $4, $5, $6
        FROM s
        RETURNING skill_id, revision
        ",
        name,
        description,
        Json(files) as _,
        author.human,
        author.dream,
        author.turn,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(written.map(|row| (row.skill_id, row.revision)))
}

/// A revision on top of `expected`; `None` when that is not current.
async fn add_revision(
    tx: &mut Transaction<'_, Postgres>,
    skill: Uuid,
    expected: Option<i64>,
    (name, description): &(String, String),
    files: &Files,
    author: Author,
) -> Result<Option<(Uuid, i64)>, sqlx::Error> {
    let written = sqlx::query!(
        "
        WITH s AS (
            UPDATE skills
            SET revision=revision+1
            WHERE id=$1 AND name=$2 AND revision=$3
            RETURNING id, revision
        )
        INSERT INTO skill_revisions (skill_id, revision, description, files, human_id, dream_run_id, turn_id)
        SELECT id, revision, $4, $5, $6, $7, $8
        FROM s
        RETURNING skill_id, revision
        ",
        skill,
        name,
        expected,
        description,
        Json(files) as _,
        author.human,
        author.dream,
        author.turn,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(written.map(|row| (row.skill_id, row.revision)))
}

/// Publishes a dream's proposed package and installs it for the agent. A revision of an unknown
/// skill, a stale revision, a taken name or a package identical to the current one is dropped.
pub(super) async fn propose(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    run: Uuid,
    change: &SkillChange,
    metadata: &(String, String),
) -> Result<(), sqlx::Error> {
    match proposed(tx, run, change, metadata).await? {
        // The revision was just written, so it exists.
        Some((skill, revision)) => install(tx, agent, skill, revision).await.map(|_| ()),
        None => {
            dropped(&metadata.0);
            Ok(())
        }
    }
}

fn dropped(name: &str) {
    tracing::info!(name, "dream skill dropped");
}

async fn proposed(
    tx: &mut Transaction<'_, Postgres>,
    run: Uuid,
    change: &SkillChange,
    metadata: &(String, String),
) -> Result<Option<(Uuid, i64)>, sqlx::Error> {
    let known = current(tx, &metadata.0).await?;
    if known
        .as_ref()
        .is_some_and(|known| known.files == change.files)
    {
        return Ok(None);
    }
    let skill = known.map(|known| known.id);
    let expected = change.expected_revision;
    let author = Author {
        dream: Some(run),
        ..Author::default()
    };
    match (skill, expected) {
        (None, Some(_)) => Ok(None),
        // A new package under a taken name inserts nothing.
        (_, None) => write(tx, None, None, metadata, &change.files, author).await,
        (Some(_), Some(_)) => write(tx, skill, expected, metadata, &change.files, author).await,
    }
}

struct Current {
    id: Uuid,
    revision: i64,
    description: String,
    files: Files,
}

/// The library's skill called `name`, at its current revision.
async fn current(
    tx: &mut Transaction<'_, Postgres>,
    name: &str,
) -> Result<Option<Current>, sqlx::Error> {
    let found = sqlx::query!(
        r#"
        SELECT s.id, s.revision, r.description, r.files AS "files: Json<Files>"
        FROM skills s
        JOIN skill_revisions r ON r.skill_id=s.id AND r.revision=s.revision
        WHERE s.name=$1
        "#,
        name,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(found.map(|found| Current {
        id: found.id,
        revision: found.revision,
        description: found.description,
        files: found.files.0,
    }))
}

/// Every skill in the library, for a dream to improve rather than duplicate.
/// shortcut: names and descriptions only, 200 at most; rank by relevance once libraries grow.
pub(super) async fn catalog(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<Vec<LibrarySkill>, sqlx::Error> {
    sqlx::query_as!(
        LibrarySkill,
        "
        SELECT s.name, r.description
        FROM skills s
        JOIN skill_revisions r ON r.skill_id=s.id AND r.revision=s.revision
        ORDER BY s.name
        LIMIT 200
        "
    )
    .fetch_all(&mut **tx)
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportFile {
    path: String,
    content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    name: String,
    description: String,
    instructions: String,
    #[serde(default)]
    files: Vec<SupportFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    name: String,
    expected_revision: i64,
    description: String,
    instructions: String,
    #[serde(default)]
    files: Vec<SupportFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Install {
    name: String,
}

const INVALID_PACKAGE: &str = "name is lowercase words joined by single hyphens, up to 64 characters; description is 1 to 1024 characters; instructions are not empty; files are under references/, scripts/ or assets/ with plain names; the whole package is text and at most 64 KiB";

/// Builds a package from a tool call: the description becomes one line of front matter.
fn compose(
    name: &str,
    description: &str,
    instructions: &str,
    extra: Vec<SupportFile>,
) -> Option<(Files, (String, String))> {
    if instructions.trim().is_empty() {
        return None;
    }
    let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
    let entry = format!(
        "---\nname: {name}\ndescription: {description}\n---\n\n{}\n",
        instructions.trim()
    );
    let mut files: Files = extra
        .into_iter()
        .map(|file| (file.path, file.content))
        .collect();
    files.insert("SKILL.md".into(), entry);
    let metadata = package(&files)?;
    // A description that looked like a block or a quote must not rename the package.
    (metadata.0 == name).then_some((files, metadata))
}

type Answer = Result<String, String>;

impl Knowledge {
    /// The `skill_*` tools of a running turn. `Err` is the refusal the model reads; the outer
    /// error is the orchestrator's own. The turn is the revision's author.
    #[tracing::instrument(skip_all, fields(agent = %actor.agent.0, turn = %turn, tool = %call.tool))]
    pub(crate) async fn skill_tool(
        &self,
        actor: &RuntimeActor,
        turn: Uuid,
        call: &RoutineCall,
    ) -> Result<Answer, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let running = sqlx::query_scalar!(
            "
            SELECT id
            FROM chat_turns
            WHERE id=$1 AND conversation_id=$2 AND status='running' AND attempt_id=$3
            FOR UPDATE
            ",
            turn,
            actor.agent.0,
            actor.attempt.0,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if running.is_none() {
            return Ok(Err("this turn is no longer running".into()));
        }
        let answer = match call.tool.as_str() {
            "skill_list" => list_library(&mut tx, actor.agent).await?,
            "skill_create" => create_skill(&mut tx, actor.agent, turn, &call.input).await?,
            "skill_update" => update_skill(&mut tx, actor.agent, turn, &call.input).await?,
            "skill_install" => install_named(&mut tx, actor.agent, &call.input).await?,
            _ => Err("no skill tool has this name".into()),
        };
        // A refused call changed nothing, so its transaction has nothing to keep.
        tx.commit().await?;
        Ok(answer)
    }

    /// Adds the skills that ship with the platform and installs each on every agent, the once:
    /// a skill already called that stays as it is, and uninstalling one from an agent sticks.
    #[tracing::instrument(skip_all)]
    pub async fn seed_shipped_skills(pool: &PgPool) -> Result<(), sqlx::Error> {
        for entry in SHIPPED {
            let files: Files = [("SKILL.md".to_owned(), (*entry).to_owned())].into();
            let Some(metadata) = package(&files) else {
                tracing::error!("a shipped skill is not a valid package");
                continue;
            };
            let mut tx = pool.begin().await?;
            let written = write(&mut tx, None, None, &metadata, &files, Author::default()).await?;
            if let Some((skill, revision)) = written {
                sqlx::query!(
                    "
                    UPDATE skills
                    SET installs_by_default=true
                    WHERE id=$1
                    ",
                    skill,
                )
                .execute(&mut *tx)
                .await?;
                sqlx::query!(
                    "
                    INSERT INTO skill_installations (agent_id, skill_id, revision)
                    SELECT id, $1, $2
                    FROM agents
                    WHERE desired_state<>'retired'
                    ",
                    skill,
                    revision,
                )
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
        }
        Ok(())
    }
}

async fn list_library(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
) -> Result<Answer, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT s.name, s.revision, r.description, i.revision AS "installed?"
        FROM skills s
        JOIN skill_revisions r ON r.skill_id=s.id AND r.revision=s.revision
        LEFT JOIN skill_installations i ON i.skill_id=s.id AND i.agent_id=$1
        ORDER BY s.name
        LIMIT 200
        "#,
        agent.0,
    )
    .fetch_all(&mut **tx)
    .await?;
    let listed: Vec<_> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "name": row.name,
                "revision": row.revision,
                "description": row.description,
                "installed_revision": row.installed,
            })
        })
        .collect();
    Ok(Ok(serde_json::Value::Array(listed).to_string()))
}

async fn create_skill(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    turn: Uuid,
    input: &serde_json::Value,
) -> Result<Answer, IdentityError> {
    let Ok(create) = Create::deserialize(input) else {
        return Ok(Err(INVALID_PACKAGE.into()));
    };
    let name = create.name.clone();
    let composed = compose(
        &name,
        &create.description,
        &create.instructions,
        create.files,
    );
    let Some((files, metadata)) = composed else {
        return Ok(Err(INVALID_PACKAGE.into()));
    };
    if let Some(known) = current(tx, &name).await? {
        // A resent call, whose first answer was lost, finds its own package.
        if known.files == files {
            install(tx, agent, known.id, known.revision).await?;
            return Ok(Ok(format!("Skill {name} is saved and installed.")));
        }
        return Ok(Err(format!(
            "the library already has a skill called {name}, revision {}: {}. Use skill_update to improve it, or skill_install to use it as it is",
            known.revision, known.description
        )));
    }
    if at_turn_limit(tx, turn).await? {
        return Ok(Err(TURN_LIMIT.into()));
    }
    let author = Author {
        turn: Some(turn),
        ..Author::default()
    };
    let written = write(tx, None, None, &metadata, &files, author).await?;
    let (skill, revision) = written.ok_or(IdentityError::Conflict)?;
    install(tx, agent, skill, revision).await?;
    Ok(Ok(format!(
        "Published skill {name}, revision {revision}, to the shared library and installed it for you. People find it under Skills and you load it with load_skill."
    )))
}

async fn update_skill(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    turn: Uuid,
    input: &serde_json::Value,
) -> Result<Answer, IdentityError> {
    let Ok(update) = Update::deserialize(input) else {
        return Ok(Err(INVALID_PACKAGE.into()));
    };
    let name = update.name.clone();
    let composed = compose(
        &name,
        &update.description,
        &update.instructions,
        update.files,
    );
    let Some((files, metadata)) = composed else {
        return Ok(Err(INVALID_PACKAGE.into()));
    };
    let Some(known) = current(tx, &name).await? else {
        return Ok(Err(format!(
            "the library has no skill called {name}; use skill_create"
        )));
    };
    if known.files == files {
        install(tx, agent, known.id, known.revision).await?;
        return Ok(Ok(format!(
            "Skill {name} already says this at revision {}; nothing new was written.",
            known.revision
        )));
    }
    if known.revision != update.expected_revision {
        return Ok(Err(format!(
            "{name} is at revision {}, not {}; someone changed it, so load it again and rework your change",
            known.revision, update.expected_revision
        )));
    }
    if at_turn_limit(tx, turn).await? {
        return Ok(Err(TURN_LIMIT.into()));
    }
    let author = Author {
        turn: Some(turn),
        ..Author::default()
    };
    let expected = Some(update.expected_revision);
    let written = write(tx, Some(known.id), expected, &metadata, &files, author).await?;
    let (skill, revision) = written.ok_or(IdentityError::Conflict)?;
    install(tx, agent, skill, revision).await?;
    Ok(Ok(format!(
        "Published {name} revision {revision} and moved your installation to it. Other agents stay on their revisions until someone updates them."
    )))
}

async fn install_named(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    input: &serde_json::Value,
) -> Result<Answer, IdentityError> {
    let Ok(Install { name }) = Install::deserialize(input) else {
        return Ok(Err("name is a skill from skill_list".into()));
    };
    let Some(known) = current(tx, &name).await? else {
        return Ok(Err(format!("the library has no skill called {name}")));
    };
    install(tx, agent, known.id, known.revision).await?;
    Ok(Ok(format!(
        "Installed {name} at revision {}. Load it with load_skill.",
        known.revision
    )))
}

/// Skill revisions one turn may publish, so a confused model cannot flood the shared library.
const TURN_SKILLS: i64 = 3;
const TURN_LIMIT: &str =
    "one turn publishes at most 3 skill revisions; that is plenty, so finish with what you have";

async fn at_turn_limit(
    tx: &mut Transaction<'_, Postgres>,
    turn: Uuid,
) -> Result<bool, sqlx::Error> {
    let written = sqlx::query_scalar!(
        r#"SELECT count(*) AS "written!" FROM skill_revisions WHERE turn_id=$1"#,
        turn,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(written >= TURN_SKILLS)
}

/// The agent's installed packages, or only the one called `name`.
#[tracing::instrument(skip_all, fields(agent = %agent.0))]
pub(crate) async fn packages<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    agent: AgentId,
    name: Option<&str>,
    limit: i64,
) -> Result<Vec<SkillPackage>, sqlx::Error> {
    let found = sqlx::query!(
        r#"
        SELECT s.name, i.revision, r.files AS "files: Json<Files>"
        FROM skill_installations i
        JOIN skills s ON s.id=i.skill_id
        JOIN skill_revisions r ON r.skill_id=i.skill_id AND r.revision=i.revision
        WHERE i.agent_id=$1 AND ($2::text IS NULL OR s.name=$2)
        ORDER BY s.name
        LIMIT $3
        "#,
        agent.0,
        name,
        limit,
    )
    .fetch_all(executor)
    .await?;
    Ok(found
        .into_iter()
        .map(|found| SkillPackage {
            name: found.name,
            revision: found.revision,
            files: found.files.0,
        })
        .collect())
}

/// A turn's system prompt: the agent's instructions, then each installed skill's name and
/// description. The model loads a package with `load_skill` only when a task needs it.
pub(crate) async fn instructions(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
) -> Result<String, sqlx::Error> {
    let instructions = sqlx::query_scalar!(
        "
        SELECT instructions
        FROM agents
        WHERE id=$1
        ",
        agent.0,
    )
    .fetch_one(&mut **tx)
    .await?;
    let installed = sqlx::query!(
        "
        SELECT s.name, r.description
        FROM skill_installations i
        JOIN skills s ON s.id=i.skill_id
        JOIN skill_revisions r ON r.skill_id=i.skill_id AND r.revision=i.revision
        WHERE i.agent_id=$1
        ORDER BY s.name
        ",
        agent.0,
    )
    .fetch_all(&mut **tx)
    .await?;
    if installed.is_empty() {
        return Ok(instructions);
    }
    let lines: Vec<String> = installed
        .iter()
        .map(|skill| format!("- {}: {}", skill.name, skill.description))
        .collect();
    Ok(format!(
        "{instructions}\n\nInstalled skills. Before work one of them covers, call load_skill with its name and follow its instructions:\n{}",
        lines.join("\n")
    ))
}

/// The name and description of a valid package: at most `SKILL_BYTES` and 64 files, text only,
/// a `SKILL.md` at the root whose front matter names it, and other files under `references/`,
/// `scripts/` or `assets/`.
pub fn package(files: &Files) -> Option<(String, String)> {
    let size: usize = files
        .iter()
        .map(|(path, text)| path.len() + text.len())
        .sum();
    let contents = files
        .iter()
        .all(|(path, text)| place(path) && !text.contains('\0'));
    if size > SKILL_BYTES || files.len() > 64 || !contents {
        return None;
    }
    let (name, description) = metadata(files.get("SKILL.md")?)?;
    let described = !description.is_empty() && description.chars().count() <= 1024;
    (named(&name) && described).then_some((name, description))
}

/// A relative path of plain segments, which cannot escape the package once materialized.
fn place(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').collect();
    let plain = segments.iter().all(|segment| {
        !segment.is_empty()
            && !segment.starts_with('.')
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    });
    let folder = segments.len() >= 2 && ["references", "scripts", "assets"].contains(&segments[0]);
    plain && segments.len() <= 4 && (path == "SKILL.md" || folder)
}

/// The Agent Skills name rule: lowercase words of letters and digits joined by single hyphens.
fn named(name: &str) -> bool {
    name.len() <= 64
        && name.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

/// `name` and `description` from the front matter. A block value (`|` or `>`) continues on the
/// indented lines below it and reads as one line, as a system prompt lists it.
/// shortcut: `key: value` fields, quoted or as a block; parse YAML once imports bring packages
/// with other forms.
fn metadata(entrypoint: &str) -> Option<(String, String)> {
    let (head, _) = entrypoint.strip_prefix("---\n")?.split_once("\n---")?;
    let lines: Vec<&str> = head.lines().collect();
    let field = |key: &str| {
        let at = lines.iter().position(|line| {
            line.strip_prefix(key)
                .is_some_and(|rest| rest.starts_with(':'))
        })?;
        let value = lines[at][key.len() + 1..].trim();
        if !value.starts_with(['|', '>']) {
            return Some(value.trim_matches(|c| c == '"' || c == '\'').to_string());
        }
        let block = lines[at + 1..]
            .iter()
            .take_while(|line| line.starts_with([' ', '\t']))
            .map(|line| line.trim());
        Some(block.collect::<Vec<_>>().join(" "))
    };
    Some((field("name")?, field("description")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(entries: &[(&str, &str)]) -> Files {
        entries
            .iter()
            .map(|(path, text)| (path.to_string(), text.to_string()))
            .collect()
    }

    #[test]
    fn shipped_skills_are_valid_packages() {
        for entry in SHIPPED {
            let package = package(&files(&[("SKILL.md", entry)]));
            let (name, description) = package.expect("a shipped skill must validate");
            assert!(["coding", "design"].contains(&name.as_str()), "{name}");
            assert!(description.len() > 80, "{description}");
        }
    }

    #[test]
    fn a_tool_call_composes_a_package_with_one_line_of_front_matter() {
        let (files, metadata) = compose(
            "disk-triage",
            "Find what\n fills a disk.",
            "Run du.",
            vec![],
        )
        .unwrap();
        assert_eq!(metadata.1, "Find what fills a disk.");
        assert!(files["SKILL.md"].ends_with("---\n\nRun du.\n"));
        assert!(compose("Disk Triage", "x", "y", vec![]).is_none());
        assert!(compose("disk-triage", "x", " ", vec![]).is_none());
        let escaping = vec![SupportFile {
            path: "../x".into(),
            content: "x".into(),
        }];
        assert!(compose("disk-triage", "x", "y", escaping).is_none());
    }

    #[test]
    fn packages_need_a_named_entrypoint_and_contained_paths() {
        let entry =
            "---\nname: disk-triage\ndescription: \"Find what fills a disk.\"\n---\n# Steps\n";
        let valid = files(&[
            ("SKILL.md", entry),
            ("scripts/usage.sh", "du -sh *"),
            ("references/notes.md", "x"),
        ]);
        assert_eq!(
            package(&valid),
            Some(("disk-triage".into(), "Find what fills a disk.".into()))
        );
        for path in [
            "../x",
            "scripts/../../x",
            "/etc/x",
            "scripts/.hidden",
            "x.sh",
            "scripts",
            "scripts//x",
        ] {
            let bad = files(&[("SKILL.md", entry), (path, "x")]);
            assert_eq!(package(&bad), None, "{path}");
        }
        assert_eq!(package(&files(&[("scripts/x.sh", "x")])), None);
        assert_eq!(package(&files(&[("SKILL.md", "# no front matter")])), None);
        let renamed = "---\nname: Disk_Triage\ndescription: x\n---\n";
        assert_eq!(package(&files(&[("SKILL.md", renamed)])), None);
        let undescribed = "---\nname: disk-triage\n---\n";
        assert_eq!(package(&files(&[("SKILL.md", undescribed)])), None);
        let block = "---\nname: disk-triage\ndescription: |\n  Find what fills\n  a disk.\nlicense: MIT\n---\n";
        let block = package(&files(&[("SKILL.md", block)])).unwrap();
        assert_eq!(block.1, "Find what fills a disk.");
        let empty = "---\nname: disk-triage\ndescription: >-\n---\n";
        assert_eq!(package(&files(&[("SKILL.md", empty)])), None);
        let binary = files(&[("SKILL.md", entry), ("assets/a", "\0")]);
        assert_eq!(package(&binary), None);
        let large = files(&[("SKILL.md", entry), ("assets/a", &"x".repeat(SKILL_BYTES))]);
        assert_eq!(package(&large), None);
    }
}

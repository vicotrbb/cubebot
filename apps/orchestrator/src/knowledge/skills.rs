//! The shared skill library (ADRs 0055 to 0057). Any active person publishes and revises skills;
//! a revision names the one it read, so concurrent contributors get a conflict instead of
//! overwriting each other. Installing pins a revision per agent, and a new revision never moves
//! another agent's installation. Storing a package never runs its scripts.
use super::{Knowledge, authorize};
use crate::{
    agents::AgentId,
    identity::{Actor, IdentityError},
};
use cubebot_protocol::runtime::{SKILL_BYTES, SkillChange, SkillPackage};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction, types::Json};
use std::collections::BTreeMap;
use uuid::Uuid;

pub type Files = BTreeMap<String, String>;

/// Installed skills a dream sees in full.
/// shortcut: the first 20 by name, which bounds the dream's prompt; select by relevance once
/// agents install more.
pub(super) const DREAM_SKILLS: i64 = 20;

/// A library entry at its current revision.
#[derive(Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Skill {
    pub id: Uuid,
    pub name: String,
    pub revision: i64,
    pub description: String,
    pub updated_at: String,
}

#[derive(Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct SkillRevision {
    pub skill: Uuid,
    pub name: String,
    pub revision: i64,
    pub description: String,
    #[sqlx(json)]
    pub files: Files,
    /// The person who wrote this revision, or `None` for a dream's.
    pub human: Option<Uuid>,
    /// The dream run that proposed this revision.
    pub dream_run: Option<Uuid>,
    pub created_at: String,
}

/// An agent's installation, with the library's current revision to show available updates.
#[derive(Serialize, sqlx::FromRow, utoipa::ToSchema)]
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

const LIBRARY: &str = "SELECT s.id,s.name,s.revision,r.description,(to_jsonb(r.created_at) #>> '{}') AS updated_at FROM skills s JOIN skill_revisions r ON r.skill_id=s.id AND r.revision=s.revision";

impl Knowledge {
    /// shortcut: the whole library by name; page once installations hold hundreds of skills.
    pub async fn skills(&self, _actor: &Actor) -> Result<Vec<Skill>, IdentityError> {
        Ok(sqlx::query_as(&format!("{LIBRARY} ORDER BY s.name"))
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn skill_revision(
        &self,
        _actor: &Actor,
        skill: Uuid,
        revision: i64,
    ) -> Result<SkillRevision, IdentityError> {
        sqlx::query_as("SELECT s.id AS skill,s.name,r.revision,r.description,r.files,r.human_id AS human,r.dream_run_id AS dream_run,(to_jsonb(r.created_at) #>> '{}') AS created_at FROM skill_revisions r JOIN skills s ON s.id=r.skill_id WHERE r.skill_id=$1 AND r.revision=$2")
            .bind(skill).bind(revision).fetch_optional(&self.pool).await?
            .ok_or(IdentityError::NotFound)
    }

    /// Registers a new skill; 409 when its name is taken.
    pub async fn publish_skill(
        &self,
        actor: &Actor,
        input: &PublishSkill,
    ) -> Result<Skill, IdentityError> {
        self.contribute(actor, None, None, &input.files).await
    }

    /// Adds a revision on top of `expected_revision`; 409 when that is no longer current. The
    /// package keeps the skill's name.
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
            let name: Option<String> = sqlx::query_scalar("SELECT name FROM skills WHERE id=$1")
                .bind(skill)
                .fetch_optional(&mut *tx)
                .await?;
            if name.ok_or(IdentityError::NotFound)? != metadata.0 {
                return Err(IdentityError::InvalidInput);
            }
        }
        let author = (Some(actor.id), None);
        let (id, _) = write(&mut tx, skill, expected, &metadata, files, author)
            .await?
            .ok_or(IdentityError::Conflict)?;
        let written = sqlx::query_as(&format!("{LIBRARY} WHERE s.id=$1"))
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(written)
    }

    pub async fn installed_skills(
        &self,
        actor: &Actor,
        agent: AgentId,
    ) -> Result<Vec<InstalledSkill>, IdentityError> {
        let mut tx = self.pool.begin().await?;
        authorize(&mut tx, actor, agent, "memory.read").await?;
        let installed = sqlx::query_as("SELECT s.id AS skill,s.name,i.revision,s.revision AS current_revision,r.description FROM skill_installations i JOIN skills s ON s.id=i.skill_id JOIN skill_revisions r ON r.skill_id=i.skill_id AND r.revision=i.revision WHERE i.agent_id=$1 ORDER BY s.name")
            .bind(agent).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok(installed)
    }

    /// Installs a revision, or moves an installation to it.
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
    pub async fn uninstall_skill(
        &self,
        actor: &Actor,
        agent: AgentId,
        skill: Uuid,
    ) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        authorize(&mut tx, actor, agent, "agent.manage").await?;
        sqlx::query("DELETE FROM skill_installations WHERE agent_id=$1 AND skill_id=$2")
            .bind(agent)
            .bind(skill)
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
    let installed = sqlx::query("INSERT INTO skill_installations (agent_id,skill_id,revision) SELECT $1,$2,$3 WHERE EXISTS(SELECT 1 FROM skill_revisions WHERE skill_id=$2 AND revision=$3) ON CONFLICT (agent_id,skill_id) DO UPDATE SET revision=EXCLUDED.revision,installed_at=clock_timestamp()")
        .bind(agent).bind(skill).bind(revision).execute(&mut **tx).await?;
    Ok(installed.rows_affected() == 1)
}

/// Creates a skill, or adds a revision on top of `expected`. `None` when the name is taken or
/// `expected` is not current. `author` is a person or a dream run.
pub(super) async fn write(
    tx: &mut Transaction<'_, Postgres>,
    skill: Option<Uuid>,
    expected: Option<i64>,
    (name, description): &(String, String),
    files: &Files,
    (human, dream): (Option<Uuid>, Option<Uuid>),
) -> Result<Option<(Uuid, i64)>, sqlx::Error> {
    let changed = match skill {
        None => {
            "INSERT INTO skills (name) VALUES ($2) ON CONFLICT (name) DO NOTHING RETURNING id,revision"
        }
        Some(_) => {
            "UPDATE skills SET revision=revision+1 WHERE id=$1 AND name=$2 AND revision=$3 RETURNING id,revision"
        }
    };
    sqlx::query_as(&format!("WITH s AS ({changed}) INSERT INTO skill_revisions (skill_id,revision,description,files,human_id,dream_run_id) SELECT id,revision,$4,$5,$6,$7 FROM s RETURNING skill_id,revision"))
        .bind(skill).bind(name).bind(expected).bind(description).bind(Json(files)).bind(human).bind(dream)
        .fetch_optional(&mut **tx).await
}

/// Publishes a dream's proposed package and installs it for the agent. A revision of an unknown
/// skill, a stale revision or a taken name is dropped.
pub(super) async fn propose(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    run: Uuid,
    change: &SkillChange,
    metadata: &(String, String),
) -> Result<(), sqlx::Error> {
    let skill: Option<Uuid> = sqlx::query_scalar("SELECT id FROM skills WHERE name=$1")
        .bind(&metadata.0)
        .fetch_optional(&mut **tx)
        .await?;
    let expected = change.expected_revision;
    let author = (None, Some(run));
    let written = match (skill, expected) {
        (None, Some(_)) => None,
        // A new package under a taken name inserts nothing.
        (_, None) => write(tx, None, None, metadata, &change.files, author).await?,
        (Some(_), Some(_)) => write(tx, skill, expected, metadata, &change.files, author).await?,
    };
    match written {
        // The revision was just written, so it exists.
        Some((skill, revision)) => {
            install(tx, agent, skill, revision).await?;
        }
        None => tracing::info!(name = metadata.0, ?expected, "dream skill dropped"),
    }
    Ok(())
}

/// The agent's installed packages, or only the one called `name`.
pub(crate) async fn packages<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    agent: AgentId,
    name: Option<&str>,
    limit: i64,
) -> Result<Vec<SkillPackage>, sqlx::Error> {
    let found: Vec<(String, i64, Json<Files>)> = sqlx::query_as("SELECT s.name,i.revision,r.files FROM skill_installations i JOIN skills s ON s.id=i.skill_id JOIN skill_revisions r ON r.skill_id=i.skill_id AND r.revision=i.revision WHERE i.agent_id=$1 AND ($2::text IS NULL OR s.name=$2) ORDER BY s.name LIMIT $3")
        .bind(agent).bind(name).bind(limit).fetch_all(executor).await?;
    Ok(found
        .into_iter()
        .map(|(name, revision, Json(files))| SkillPackage {
            name,
            revision,
            files,
        })
        .collect())
}

/// A turn's system prompt: the agent's instructions, then each installed skill's name and
/// description. The model loads a package with `load_skill` only when a task needs it.
pub(crate) async fn instructions(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
) -> Result<String, sqlx::Error> {
    let instructions: String = sqlx::query_scalar("SELECT instructions FROM agents WHERE id=$1")
        .bind(agent)
        .fetch_one(&mut **tx)
        .await?;
    let installed: Vec<(String, String)> = sqlx::query_as("SELECT s.name,r.description FROM skill_installations i JOIN skills s ON s.id=i.skill_id JOIN skill_revisions r ON r.skill_id=i.skill_id AND r.revision=i.revision WHERE i.agent_id=$1 ORDER BY s.name")
        .bind(agent).fetch_all(&mut **tx).await?;
    if installed.is_empty() {
        return Ok(instructions);
    }
    let lines: Vec<String> = installed
        .iter()
        .map(|(name, description)| format!("- {name}: {description}"))
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

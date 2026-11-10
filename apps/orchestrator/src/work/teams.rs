//! Agent teams (docs/adr/agents.md#agent-teams). An installation's organization is small, so it is
//! loaded whole and checked in Rust; every write holds one advisory lock, so two edits cannot
//! combine into a cycle, and a team room's agents are recomputed from the rosters after each write.
mod brief;
mod tool;

use super::{Work, access};
use crate::identity::{Actor, IdentityError};
pub(super) use brief::briefed;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use std::collections::HashSet;
pub(super) use tool::tool;
use uuid::Uuid;

const MEMBERS: usize = 64;

#[derive(Clone, Serialize, utoipa::ToSchema)]
pub struct Team {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub instructions: String,
    /// The team this one sits under.
    pub parent: Option<Uuid>,
    /// The parent team's member this team's manager reports to.
    pub reports_to: Option<Uuid>,
    pub room: Uuid,
    pub revision: i64,
    /// The manager first, then by name.
    pub members: Vec<TeamMember>,
}

#[derive(Clone, Serialize, utoipa::ToSchema)]
pub struct TeamMember {
    pub agent: Uuid,
    pub name: String,
    pub title: String,
    pub role: String,
    /// The member this one reports to; null for the manager.
    pub reports_to: Option<Uuid>,
    pub retired: bool,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveTeam {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub parent: Option<Uuid>,
    pub reports_to: Option<Uuid>,
    pub members: Vec<SaveMember>,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveMember {
    pub agent: Uuid,
    pub title: String,
    pub role: String,
    pub reports_to: Option<Uuid>,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateTeam {
    pub request_id: Uuid,
    pub team: SaveTeam,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateTeam {
    pub expected_revision: i64,
    pub team: SaveTeam,
}

impl From<&Team> for SaveTeam {
    fn from(team: &Team) -> Self {
        Self {
            name: team.name.clone(),
            description: team.description.clone(),
            instructions: team.instructions.clone(),
            parent: team.parent,
            reports_to: team.reports_to,
            members: team
                .members
                .iter()
                .map(|member| SaveMember {
                    agent: member.agent,
                    title: member.title.clone(),
                    role: member.role.clone(),
                    reports_to: member.reports_to,
                })
                .collect(),
        }
    }
}

impl SaveTeam {
    fn manager(&self) -> Option<Uuid> {
        self.members
            .iter()
            .find(|member| member.reports_to.is_none())
            .map(|member| member.agent)
    }
}

impl Work {
    /// Administrators see every team; others see the teams whose room they can read.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn teams(&self, actor: &Actor) -> Result<Vec<Team>, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let readable = sqlx::query_scalar!(
            r#"
            SELECT t.room_id
            FROM teams t
            JOIN human_accounts a ON a.id=$1 AND a.active
            WHERE t.deleted_at IS NULL AND (a.role='administrator'
                OR EXISTS(SELECT 1 FROM effective_human_grants g WHERE g.actor_id=a.id AND g.scope_kind='room' AND g.scope_id=t.room_id AND g.capability='conversation.read'))
            "#,
            actor.id,
        )
        .fetch_all(&mut *tx)
        .await?;
        let org = load(&mut tx).await?;
        tx.commit().await?;
        Ok(org
            .into_iter()
            .filter(|team| readable.contains(&team.room))
            .collect())
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn create_team(
        &self,
        actor: &Actor,
        input: &CreateTeam,
    ) -> Result<Team, IdentityError> {
        checked(&input.team).map_err(|_| IdentityError::InvalidInput)?;
        let mut tx = self.begin_change(actor).await?;
        let previous = sqlx::query_scalar!(
            "SELECT id FROM teams WHERE created_by=$1 AND request_id=$2",
            actor.id,
            input.request_id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let id = match previous {
            Some(id) => id,
            None => insert(&mut tx, actor, input).await?,
        };
        let team = find(&load(&mut tx).await?, id)?;
        commit(tx).await?;
        Ok(team)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, team = %id))]
    pub async fn update_team(
        &self,
        actor: &Actor,
        id: Uuid,
        input: &UpdateTeam,
    ) -> Result<Team, IdentityError> {
        checked(&input.team).map_err(|_| IdentityError::InvalidInput)?;
        let mut tx = self.begin_change(actor).await?;
        let org = load(&mut tx).await?;
        if find(&org, id)?.revision != input.expected_revision {
            return Err(IdentityError::Conflict);
        }
        placed(&org, Some(id), &input.team).map_err(|_| IdentityError::InvalidInput)?;
        write(&mut tx, id, &input.team).await?;
        let team = find(&load(&mut tx).await?, id)?;
        commit(tx).await?;
        Ok(team)
    }

    /// Only a team with no team under it; its room is archived with its history.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, team = %id))]
    pub async fn delete_team(
        &self,
        actor: &Actor,
        id: Uuid,
        expected_revision: i64,
    ) -> Result<Team, IdentityError> {
        let mut tx = self.begin_change(actor).await?;
        let org = load(&mut tx).await?;
        let team = find(&org, id)?;
        if team.revision != expected_revision || org.iter().any(|t| t.parent == Some(id)) {
            return Err(IdentityError::Conflict);
        }
        // Posting holds this lock while it checks `archived`, so no message lands after the archive.
        sqlx::query!("SELECT id FROM rooms WHERE id=$1 FOR UPDATE", team.room)
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query!(
            "UPDATE teams SET deleted_at=clock_timestamp(), parent_id=NULL, reports_to=NULL, revision=revision+1 WHERE id=$1",
            id,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!("DELETE FROM team_members WHERE team_id=$1", id)
            .execute(&mut *tx)
            .await?;
        sqlx::query!("UPDATE rooms SET archived=true WHERE id=$1", team.room)
            .execute(&mut *tx)
            .await?;
        sync_rooms(&mut tx).await?;
        commit(tx).await?;
        Ok(team)
    }

    async fn begin_change(
        &self,
        actor: &Actor,
    ) -> Result<Transaction<'static, Postgres>, IdentityError> {
        let mut tx = self.pool.begin().await?;
        access::lock_actor(&mut tx, actor).await?;
        crate::identity::administrator(&mut *tx, actor.id).await?;
        lock(&mut tx).await?;
        Ok(tx)
    }
}

/// A deferred line that no longer holds fails at commit; a lost race, not bad input.
async fn commit(tx: Transaction<'_, Postgres>) -> Result<(), IdentityError> {
    tx.commit().await.map_err(|error| match error {
        sqlx::Error::Database(ref database) if database.is_foreign_key_violation() => {
            IdentityError::Conflict
        }
        other => other.into(),
    })
}

fn find(org: &[Team], id: Uuid) -> Result<Team, IdentityError> {
    org.iter()
        .find(|team| team.id == id)
        .cloned()
        .ok_or(IdentityError::NotFound)
}

async fn lock(tx: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    sqlx::query!("SELECT pg_advisory_xact_lock(hashtext('cubebot.teams'))")
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    input: &CreateTeam,
) -> Result<Uuid, IdentityError> {
    let org = load(tx).await?;
    placed(&org, None, &input.team).map_err(|_| IdentityError::InvalidInput)?;
    let room = sqlx::query_scalar!(
        "INSERT INTO rooms (name, created_by, request_id) VALUES ($1, $2, $3) RETURNING id",
        input.team.name.trim(),
        actor.id,
        input.request_id,
    )
    .fetch_one(&mut **tx)
    .await?;
    let id = sqlx::query_scalar!(
        "INSERT INTO teams (name, room_id, created_by, request_id) VALUES ($1, $2, $3, $4) RETURNING id",
        input.team.name.trim(),
        room,
        actor.id,
        input.request_id,
    )
    .fetch_one(&mut **tx)
    .await?;
    write(tx, id, &input.team).await?;
    // The insert counted as the first revision.
    sqlx::query!("UPDATE teams SET revision=1 WHERE id=$1", id)
        .execute(&mut **tx)
        .await?;
    Ok(id)
}

/// Replaces the team's text, place and roster. Lines into a removed member move to the manager.
async fn write(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    team: &SaveTeam,
) -> Result<(), IdentityError> {
    let agents: Vec<Uuid> = team.members.iter().map(|member| member.agent).collect();
    if usable(tx, id, &agents).await? != agents.len() as i64 {
        return Err(IdentityError::InvalidInput);
    }
    retext(tx, id, team).await?;
    sqlx::query!(
        "UPDATE teams SET reports_to=$2, revision=revision+1 WHERE parent_id=$1 AND deleted_at IS NULL AND reports_to<>ALL($3)",
        id,
        team.manager(),
        &agents,
    )
    .execute(&mut **tx)
    .await?;
    seat(tx, id, &team.members).await?;
    sync_rooms(tx).await?;
    Ok(())
}

/// How many of `agents` exist and are not retired, or already sit in the team.
async fn usable(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    agents: &[Uuid],
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"
        SELECT count(*) AS "count!"
        FROM agents g
        WHERE g.id=ANY($1) AND (g.desired_state<>'retired'
            OR EXISTS(SELECT 1 FROM team_members m WHERE m.team_id=$2 AND m.agent_id=g.id))
        "#,
        agents,
        id,
    )
    .fetch_one(&mut **tx)
    .await
}

/// The team's text and place, and its room's name.
async fn retext(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    team: &SaveTeam,
) -> Result<(), sqlx::Error> {
    let room = sqlx::query_scalar!(
        "
        UPDATE teams
        SET name=$2, description=$3, instructions=$4, parent_id=$5, reports_to=$6, revision=revision+1
        WHERE id=$1
        RETURNING room_id
        ",
        id,
        team.name.trim(),
        team.description.trim(),
        team.instructions.trim(),
        team.parent,
        team.reports_to,
    )
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query!(
        "UPDATE rooms SET name=$2 WHERE id=$1",
        room,
        team.name.trim()
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Replaces the roster; the deferred lines are checked when the transaction commits.
async fn seat(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    members: &[SaveMember],
) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM team_members WHERE team_id=$1", id)
        .execute(&mut **tx)
        .await?;
    for member in members {
        sqlx::query!(
            "INSERT INTO team_members (team_id, agent_id, title, role, reports_to) VALUES ($1, $2, $3, $4, $5)",
            id,
            member.agent,
            member.title.trim(),
            member.role.trim(),
            member.reports_to,
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// A team room holds the team's agents and the managers of the teams under it, nothing else.
async fn sync_rooms(tx: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "
        DELETE FROM room_members r
        USING teams t
        WHERE t.room_id=r.room_id
            AND NOT EXISTS(SELECT 1 FROM team_room_agents w WHERE w.room_id=r.room_id AND w.agent_id=r.agent_id)
        "
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query!(
        r#"
        INSERT INTO room_members (room_id, agent_id)
        SELECT room_id AS "room_id!", agent_id AS "agent_id!" FROM team_room_agents
        ON CONFLICT DO NOTHING
        "#
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Every live team with its members, the manager first.
pub(super) async fn load(tx: &mut Transaction<'_, Postgres>) -> Result<Vec<Team>, sqlx::Error> {
    let mut teams: Vec<Team> = sqlx::query!(
        "
        SELECT id, name, description, instructions, parent_id, reports_to, room_id, revision
        FROM teams
        WHERE deleted_at IS NULL
        ORDER BY lower(name), id
        "
    )
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|row| Team {
        id: row.id,
        name: row.name,
        description: row.description,
        instructions: row.instructions,
        parent: row.parent_id,
        reports_to: row.reports_to,
        room: row.room_id,
        revision: row.revision,
        members: Vec::new(),
    })
    .collect();
    let members = sqlx::query!(
        r#"
        SELECT m.team_id, m.agent_id, g.name, m.title, m.role, m.reports_to, g.desired_state='retired' AS "retired!"
        FROM team_members m
        JOIN teams t ON t.id=m.team_id AND t.deleted_at IS NULL
        JOIN agents g ON g.id=m.agent_id
        ORDER BY m.reports_to IS NOT NULL, lower(g.name), g.id
        "#
    )
    .fetch_all(&mut **tx)
    .await?;
    for row in members {
        if let Some(team) = teams.iter_mut().find(|team| team.id == row.team_id) {
            team.members.push(TeamMember {
                agent: row.agent_id,
                name: row.name,
                title: row.title,
                role: row.role,
                reports_to: row.reports_to,
                retired: row.retired,
            });
        }
    }
    Ok(teams)
}

/// Checks a team on its own: sizes, one manager, and reporting lines that form one tree.
pub(super) fn checked(team: &SaveTeam) -> Result<(), &'static str> {
    let name = team.name.trim();
    let text = [&team.description, &team.instructions];
    if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        return Err("a team's name is 1 to 128 bytes on one line");
    }
    if text.iter().any(|text| text.contains('\0')) {
        return Err("text cannot contain NUL");
    }
    if team.description.trim().len() > 2048 || team.instructions.trim().len() > 8192 {
        return Err("a description is at most 2 KiB and instructions at most 8 KiB");
    }
    if team.parent.is_some() != team.reports_to.is_some() {
        return Err("a team under another names who its manager reports to there");
    }
    if !(1..=MEMBERS).contains(&team.members.len()) {
        return Err("a team has 1 to 64 members");
    }
    team.members.iter().try_for_each(member_checked)?;
    lines_checked(&team.members)
}

fn member_checked(member: &SaveMember) -> Result<(), &'static str> {
    let title = member.title.trim();
    if title.is_empty() || title.len() > 64 || title.chars().any(char::is_control) {
        return Err("a title is 1 to 64 bytes on one line");
    }
    if member.role.trim().len() > 4096 || member.role.contains('\0') {
        return Err("role instructions are at most 4 KiB");
    }
    Ok(())
}

fn lines_checked(members: &[SaveMember]) -> Result<(), &'static str> {
    let agents: HashSet<Uuid> = members.iter().map(|member| member.agent).collect();
    if agents.len() != members.len() {
        return Err("an agent is in a team once");
    }
    if members.iter().filter(|m| m.reports_to.is_none()).count() != 1 {
        return Err("a team has exactly one manager");
    }
    for member in members {
        let mut seen = HashSet::from([member.agent]);
        let mut next = member.reports_to;
        while let Some(boss) = next {
            if !agents.contains(&boss) {
                return Err("everyone reports to a member of the same team");
            }
            if !seen.insert(boss) {
                return Err("reporting lines cannot loop");
            }
            next = members
                .iter()
                .find(|m| m.agent == boss)
                .and_then(|m| m.reports_to);
        }
    }
    Ok(())
}

/// Checks a team against the rest: a unique name, and a parent that exists outside its subtree.
pub(super) fn placed(org: &[Team], id: Option<Uuid>, team: &SaveTeam) -> Result<(), &'static str> {
    let name = team.name.trim().to_lowercase();
    if org
        .iter()
        .any(|other| Some(other.id) != id && other.name.trim().to_lowercase() == name)
    {
        return Err("another team has this name");
    }
    let (Some(parent), Some(boss)) = (team.parent, team.reports_to) else {
        return Ok(());
    };
    let Some(above) = org.iter().find(|other| other.id == parent) else {
        return Err("the team above does not exist");
    };
    if id.is_some_and(|id| id == parent || below(org, id).contains(&parent)) {
        return Err("a team cannot sit under itself or a team below it");
    }
    if !above.members.iter().any(|member| member.agent == boss) {
        return Err("the manager reports to a member of the team above");
    }
    if team.manager() == Some(boss) {
        return Err("a manager cannot report to itself in the team above");
    }
    Ok(())
}

/// Every team under `id`, at any depth.
fn below(org: &[Team], id: Uuid) -> Vec<Uuid> {
    let mut found = vec![id];
    let mut index = 0;
    while let Some(&current) = found.get(index) {
        let children: Vec<Uuid> = org
            .iter()
            .filter(|team| team.parent == Some(current) && !found.contains(&team.id))
            .map(|team| team.id)
            .collect();
        found.extend(children);
        index += 1;
    }
    found.remove(0);
    found
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub fn member(agent: Uuid, reports_to: Option<Uuid>) -> SaveMember {
        SaveMember {
            agent,
            title: "Engineer".into(),
            role: String::new(),
            reports_to,
        }
    }

    pub fn draft(members: Vec<SaveMember>) -> SaveTeam {
        SaveTeam {
            name: "Platform".into(),
            description: String::new(),
            instructions: String::new(),
            parent: None,
            reports_to: None,
            members,
        }
    }

    pub fn team(
        id: Uuid,
        name: &str,
        parent: Option<(Uuid, Uuid)>,
        members: &[SaveMember],
    ) -> Team {
        Team {
            id,
            name: name.into(),
            description: String::new(),
            instructions: String::new(),
            parent: parent.map(|(team, _)| team),
            reports_to: parent.map(|(_, boss)| boss),
            room: Uuid::new_v4(),
            revision: 1,
            members: members
                .iter()
                .map(|member| TeamMember {
                    agent: member.agent,
                    name: member.agent.to_string()[..4].into(),
                    title: member.title.clone(),
                    role: member.role.clone(),
                    reports_to: member.reports_to,
                    retired: false,
                })
                .collect(),
        }
    }

    #[test]
    fn a_team_needs_one_manager_and_lines_that_end_at_them() {
        let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        assert!(checked(&draft(vec![member(a, None), member(b, Some(a))])).is_ok());
        assert!(checked(&draft(vec![member(a, None), member(b, None)])).is_err());
        assert!(checked(&draft(vec![member(a, Some(b)), member(b, Some(a))])).is_err());
        assert!(
            checked(&draft(vec![
                member(a, None),
                member(b, Some(c)),
                member(c, Some(b))
            ]))
            .is_err()
        );
        assert!(checked(&draft(vec![member(a, None), member(b, Some(c))])).is_err());
        assert!(checked(&draft(vec![member(a, None), member(a, None)])).is_err());
        assert!(checked(&draft(vec![])).is_err());
    }

    #[test]
    fn a_team_sits_under_a_member_of_another_team_outside_its_subtree() {
        let (cto, lead, dev) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let (top, mid) = (Uuid::new_v4(), Uuid::new_v4());
        let org = vec![
            team(top, "Leadership", None, &[member(cto, None)]),
            team(mid, "Engineering", Some((top, cto)), &[member(lead, None)]),
        ];
        let mut under = draft(vec![member(dev, None)]);
        under.name = "KTLO".into();
        under.parent = Some(mid);
        under.reports_to = Some(lead);
        assert!(placed(&org, None, &under).is_ok());
        under.reports_to = Some(cto);
        assert!(placed(&org, None, &under).is_err());
        let mut looping = SaveTeam::from(&org[0]);
        looping.parent = Some(mid);
        looping.reports_to = Some(lead);
        assert!(placed(&org, Some(top), &looping).is_err());
        under.name = "engineering ".into();
        under.reports_to = Some(lead);
        assert!(placed(&org, None, &under).is_err());
        assert_eq!(below(&org, top), vec![mid]);
    }
}

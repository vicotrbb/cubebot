//! Tools for an agent that manages a team: it may change that team and every team under it, but
//! never its own place or who manages a team it leads, so a manager cannot hand itself away. Its
//! changes carry the turn's person's authority: that person must be able to use every agent whose
//! instructions the change touches.
use super::{SaveMember, SaveTeam, Team, below, checked, load, lock, placed, write};
use crate::{
    agents::runtime::RuntimeActor,
    identity::{IdentityError, audit},
};
use cubebot_protocol::runtime::RoutineCall;
use serde::Deserialize;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

type Answer = Result<String, String>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Place {
    team: String,
    agent: String,
    title: Option<String>,
    role: Option<String>,
    reports_to: Option<String>,
    manager: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Removal {
    team: String,
    agent: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Text {
    team: String,
    name: Option<String>,
    description: Option<String>,
    instructions: Option<String>,
}

/// `person` is the turn's person, whose access bounds which agents can be added.
pub(in crate::work) async fn tool(
    tx: &mut Transaction<'_, Postgres>,
    actor: &RuntimeActor,
    person: Uuid,
    call: &RoutineCall,
) -> Result<Answer, IdentityError> {
    lock(tx).await?;
    let org = load(tx).await?;
    let me = actor.agent.0;
    let (id, draft) = match changed(tx, &org, (me, person), call).await? {
        Ok(change) => change,
        Err(refusal) => return Ok(Err(refusal)),
    };
    if let Err(refusal) = checked(&draft).and_then(|()| placed(&org, Some(id), &draft)) {
        return Ok(Err(refusal.into()));
    }
    if let Some(name) = unusable(tx, &org, (me, person), id, &draft).await? {
        return Ok(Err(format!(
            "the person this turn is for cannot use {name}, so this change is for an administrator"
        )));
    }
    write(tx, id, &draft).await?;
    let event = audit::Event::by_agent(me, "agent_team.update")
        .on(id)
        .detail(
            serde_json::json!({ "name": draft.name.trim(), "tool": call.tool, "person": person }),
        );
    audit::record(&mut **tx, &event).await?;
    Ok(Ok(format!("{} is updated.", draft.name.trim())))
}

/// The first agent whose instructions the change touches that the turn's person may not use.
async fn unusable(
    tx: &mut Transaction<'_, Postgres>,
    org: &[Team],
    (me, person): (Uuid, Uuid),
    id: Uuid,
    draft: &SaveTeam,
) -> Result<Option<String>, sqlx::Error> {
    let usable: Vec<Uuid> = crate::work::tasks::eligible(tx, person)
        .await?
        .into_iter()
        .map(|agent| agent.id)
        .collect();
    let before = org
        .iter()
        .filter(|team| team.id == id)
        .flat_map(|team| &team.members);
    Ok(before
        .filter(|member| member.agent != me && !usable.contains(&member.agent))
        .map(|member| member.name.clone())
        .next()
        .or_else(|| {
            draft
                .members
                .iter()
                .find(|member| member.agent != me && !usable.contains(&member.agent))
                .map(|_| "an agent in the new roster".to_owned())
        }))
}

/// The team the call changes and its whole new state, or why the call is refused.
async fn changed(
    tx: &mut Transaction<'_, Postgres>,
    org: &[Team],
    (me, person): (Uuid, Uuid),
    call: &RoutineCall,
) -> Result<Result<(Uuid, SaveTeam), String>, IdentityError> {
    Ok(match call.tool.as_str() {
        "team_member" => match Place::deserialize(&call.input) {
            Ok(input) => placing(tx, org, me, person, &input).await?,
            Err(_) => Err("team, agent, title, role, reports_to and manager are required".into()),
        },
        "team_remove" => match Removal::deserialize(&call.input) {
            Ok(input) => removing(org, me, &input),
            Err(_) => Err("team and agent are required".into()),
        },
        "team_update" => match Text::deserialize(&call.input) {
            Ok(input) => target(org, me, &input.team).map(|team| (team.id, retexted(team, input))),
            Err(_) => Err("team, name, description and instructions are required".into()),
        },
        _ => Err("no team tool has this name".into()),
    })
}

fn removing(org: &[Team], me: Uuid, input: &Removal) -> Result<(Uuid, SaveTeam), String> {
    let team = target(org, me, &input.team)?;
    let mut draft = SaveTeam::from(team);
    let agent = teammate(team, &input.agent)?;
    guard(org, me, team, agent)?;
    removed(&mut draft, agent)?;
    Ok((team.id, draft))
}

async fn placing(
    tx: &mut Transaction<'_, Postgres>,
    org: &[Team],
    me: Uuid,
    person: Uuid,
    input: &Place,
) -> Result<Result<(Uuid, SaveTeam), String>, IdentityError> {
    let team = match target(org, me, &input.team) {
        Ok(team) => team,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let agent = match chosen(tx, team, person, &input.agent).await? {
        Ok(agent) => agent,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let boss = match input.reports_to.as_deref().map(|name| teammate(team, name)) {
        Some(Err(refusal)) => return Ok(Err(refusal)),
        found => found.and_then(Result::ok),
    };
    if input.manager && led(org, me).any(|id| id == team.id) {
        return Ok(Err(
            "who manages a team you lead is for an administrator or the manager above you to change"
                .into(),
        ));
    }
    let mut draft = SaveTeam::from(team);
    let change = guard(org, me, team, agent)
        .and_then(|()| upsert(&mut draft, agent, boss, input))
        .map(|()| (team.id, draft));
    Ok(change)
}

/// A member by name, or an agent the person may use when none is.
async fn chosen(
    tx: &mut Transaction<'_, Postgres>,
    team: &Team,
    person: Uuid,
    name: &str,
) -> Result<Result<Uuid, String>, sqlx::Error> {
    Ok(match teammate(team, name) {
        Err(refusal) if !refusal.contains("several") => reachable(tx, person, name)
            .await?
            .ok_or_else(|| format!("no agent you may add is named exactly {name}")),
        found => found,
    })
}

/// A team the agent manages, or one under it.
fn target<'a>(org: &'a [Team], me: Uuid, name: &str) -> Result<&'a Team, String> {
    let team = org
        .iter()
        .find(|team| team.name.trim().eq_ignore_ascii_case(name.trim()))
        .ok_or_else(|| format!("no team is named {name}"))?;
    let reach: Vec<Uuid> = led(org, me)
        .flat_map(|id| std::iter::once(id).chain(below(org, id)))
        .collect();
    if reach.contains(&team.id) {
        Ok(team)
    } else {
        Err(format!(
            "you manage neither {} nor a team above it",
            team.name
        ))
    }
}

fn led(org: &[Team], me: Uuid) -> impl Iterator<Item = Uuid> + '_ {
    org.iter()
        .filter(move |team| {
            team.members
                .iter()
                .any(|m| m.agent == me && m.reports_to.is_none())
        })
        .map(|team| team.id)
}

/// Only an administrator or a manager above changes a manager's own place.
fn guard(org: &[Team], me: Uuid, team: &Team, agent: Uuid) -> Result<(), String> {
    if agent == me && led(org, me).any(|id| id == team.id) {
        return Err("your own place in a team you manage is for an administrator or the manager above you to change".into());
    }
    Ok(())
}

/// Agent names are not unique, so an ambiguous name is refused rather than guessed.
fn teammate(team: &Team, name: &str) -> Result<Uuid, String> {
    let matching: Vec<Uuid> = team
        .members
        .iter()
        .filter(|member| member.name.eq_ignore_ascii_case(name.trim()))
        .map(|member| member.agent)
        .collect();
    match matching.as_slice() {
        [agent] => Ok(*agent),
        [] => Err(format!("{name} is not in {}", team.name)),
        _ => Err(format!(
            "several members of {} are named {name}; ask an administrator",
            team.name
        )),
    }
}

/// Adds or changes a member. `manager` makes it the manager, and the old manager then reports to
/// it; without `reports_to` a member keeps its line, and a new one reports to the manager.
fn upsert(
    draft: &mut SaveTeam,
    agent: Uuid,
    boss: Option<Uuid>,
    input: &Place,
) -> Result<(), String> {
    let previous = draft.manager();
    if input.manager {
        for member in &mut draft.members {
            if Some(member.agent) == previous && member.agent != agent {
                member.reports_to = Some(agent);
            }
        }
    }
    let line = |current: Option<Uuid>| {
        if input.manager {
            None
        } else {
            boss.or(current).or(previous)
        }
    };
    match draft.members.iter_mut().find(|m| m.agent == agent) {
        Some(member) => {
            member.reports_to = line(member.reports_to);
            member.title = input.title.clone().unwrap_or(member.title.clone());
            member.role = input.role.clone().unwrap_or(member.role.clone());
        }
        None => draft.members.push(SaveMember {
            agent,
            title: input.title.clone().ok_or("a new member needs a title")?,
            role: input.role.clone().unwrap_or_default(),
            reports_to: line(None),
        }),
    }
    // A new manager's old boss line would point at itself.
    for member in &mut draft.members {
        if member.reports_to == Some(member.agent) {
            member.reports_to = None;
        }
    }
    Ok(())
}

/// Removes a member; whoever reported to it reports to its own boss.
fn removed(draft: &mut SaveTeam, agent: Uuid) -> Result<(), String> {
    let gone = draft
        .members
        .iter()
        .position(|member| member.agent == agent)
        .ok_or("not a member")?;
    let Some(boss) = draft.members[gone].reports_to else {
        return Err("a team keeps its manager; make another member manager first".into());
    };
    draft.members.remove(gone);
    for member in &mut draft.members {
        if member.reports_to == Some(agent) {
            member.reports_to = Some(boss);
        }
    }
    Ok(())
}

fn retexted(team: &Team, input: Text) -> SaveTeam {
    let mut draft = SaveTeam::from(team);
    draft.name = input.name.unwrap_or(draft.name);
    draft.description = input.description.unwrap_or(draft.description);
    draft.instructions = input.instructions.unwrap_or(draft.instructions);
    draft
}

/// An agent the turn's person may send to, by exact name.
async fn reachable(
    tx: &mut Transaction<'_, Postgres>,
    person: Uuid,
    name: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    let matching: Vec<Uuid> = crate::work::tasks::eligible(tx, person)
        .await?
        .into_iter()
        .filter(|agent| agent.name.eq_ignore_ascii_case(name.trim()))
        .map(|agent| agent.id)
        .collect();
    Ok(match matching.as_slice() {
        [agent] => Some(*agent),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{draft, member, team};
    use super::*;

    fn place(title: Option<&str>) -> Place {
        Place {
            team: String::new(),
            agent: String::new(),
            title: title.map(Into::into),
            role: None,
            reports_to: None,
            manager: false,
        }
    }

    #[test]
    fn making_someone_manager_puts_the_old_manager_under_them() {
        let (old, new) = (Uuid::new_v4(), Uuid::new_v4());
        let mut team = draft(vec![member(old, None), member(new, Some(old))]);
        let promote = Place {
            manager: true,
            ..place(None)
        };
        upsert(&mut team, new, None, &promote).unwrap();
        assert!(checked(&team).is_ok());
        assert_eq!(team.manager(), Some(new));
        assert_eq!(team.members[0].reports_to, Some(new));
    }

    #[test]
    fn a_new_member_needs_a_title_and_keeps_lines_whole_when_its_boss_leaves() {
        let (boss, mid, dev, extra) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let mut team = draft(vec![
            member(boss, None),
            member(mid, Some(boss)),
            member(dev, Some(mid)),
        ]);
        assert!(upsert(&mut team, extra, Some(mid), &place(None)).is_err());
        upsert(&mut team, extra, Some(mid), &place(Some("QA"))).unwrap();
        removed(&mut team, mid).unwrap();
        assert!(checked(&team).is_ok());
        assert!(
            team.members
                .iter()
                .filter(|m| m.agent != boss)
                .all(|m| m.reports_to == Some(boss))
        );
        assert!(removed(&mut team, boss).is_err());
    }

    #[test]
    fn an_edit_without_a_line_keeps_it_and_names_must_be_unique() {
        let (boss, mid, dev) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let mut team = draft(vec![
            member(boss, None),
            member(mid, Some(boss)),
            member(dev, Some(mid)),
        ]);
        upsert(&mut team, dev, None, &place(Some("Staff"))).unwrap();
        assert_eq!(team.members[2].reports_to, Some(mid));
        assert_eq!(team.manager(), Some(boss));
        let mut twins = super::super::tests::team(
            Uuid::new_v4(),
            "Twins",
            None,
            &[member(boss, None), member(dev, Some(boss))],
        );
        for member in &mut twins.members {
            member.name = "Dev".into();
        }
        assert!(teammate(&twins, "dev").unwrap_err().contains("several"));
    }

    #[test]
    fn a_manager_reaches_its_teams_and_those_below_but_not_its_own_place() {
        let (lead, dev, ops, other) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let (eng, ktlo, sales) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let org = vec![
            team(
                eng,
                "Engineering",
                None,
                &[member(lead, None), member(dev, Some(lead))],
            ),
            team(ktlo, "KTLO", Some((eng, lead)), &[member(ops, None)]),
            team(sales, "Sales", None, &[member(other, None)]),
        ];
        assert!(target(&org, lead, "ktlo").is_ok());
        assert!(target(&org, lead, "Sales").is_err());
        assert!(target(&org, dev, "Engineering").is_err());
        assert!(guard(&org, lead, &org[0], lead).is_err());
        assert!(guard(&org, lead, &org[1], ops).is_ok());
    }
}

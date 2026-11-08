//! The part of every turn's instructions that tells an agent which teams it is in and its place
//! in each. Administrators' team text is quoted as written; the framing around it stays short.
use super::{Team, TeamMember, load};
use crate::agents::AgentId;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

const INTRO: &str = "You belong to the teams below. When you work for a team, follow its instructions and your role in it, together with your own instructions. Each team has a room with its name, shared by its members and by the managers of the teams under it.";

const MANAGES: &str = "You manage a team: you answer for its results. Split work that comes to it among members by their roles, follow it through and report the outcome. You can change the teams you manage, and the teams under them, with team_member, team_remove and team_update.";

/// The instructions with the agent's teams appended, and whether it manages one.
pub(in crate::work) async fn briefed(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    instructions: String,
) -> Result<(String, bool), sqlx::Error> {
    let org = load(tx).await?;
    Ok(match brief(&org, agent.0) {
        Some((teams, manager)) => (format!("{instructions}\n\n{teams}"), manager),
        None => (instructions, false),
    })
}

fn brief(org: &[Team], me: Uuid) -> Option<(String, bool)> {
    let mine: Vec<&Team> = org
        .iter()
        .filter(|team| member(team, me).is_some())
        .collect();
    if mine.is_empty() {
        return None;
    }
    let manager = mine.iter().any(|team| manages(team, me));
    let mut parts = vec![INTRO.to_owned()];
    parts.extend(manager.then(|| MANAGES.to_owned()));
    parts.extend(mine.iter().map(|team| section(org, team, me)));
    Some((capped(parts.join("\n\n")), manager))
}

/// Bounds the block for an agent in many large teams; the first teams stay whole.
fn capped(text: String) -> String {
    const LIMIT: usize = 32 * 1024;
    if text.len() <= LIMIT {
        return text;
    }
    let mut end = LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[Team details cut here; ask your manager for the rest.]",
        &text[..end]
    )
}

fn section(org: &[Team], team: &Team, me: Uuid) -> String {
    let mut lines = vec![format!("## Team {}", team.name), position(org, team, me)];
    let text = [
        ("Purpose", team.description.trim()),
        ("Team instructions", team.instructions.trim()),
        (
            "Your role",
            member(team, me).map_or("", |mine| mine.role.trim()),
        ),
    ];
    lines.extend(
        text.iter()
            .filter(|(_, text)| !text.is_empty())
            .map(|(label, text)| format!("{label}:\n{text}")),
    );
    lines.push(format!("Members: {}.", roster(team, me)));
    let under: Vec<String> = org
        .iter()
        .filter(|child| child.parent == Some(team.id))
        .filter_map(|child| child_line(team, child))
        .collect();
    if !under.is_empty() {
        lines.push(format!("Teams under it: {}.", under.join("; ")));
    }
    lines.join("\n")
}

/// One sentence placing the agent: its title, and whom it reports to.
fn position(org: &[Team], team: &Team, me: Uuid) -> String {
    let Some(mine) = member(team, me) else {
        return String::new();
    };
    let Some(boss) = mine.reports_to else {
        let above = org
            .iter()
            .find(|parent| Some(parent.id) == team.parent)
            .and_then(|parent| Some((parent, member(parent, team.reports_to?)?)));
        return match above {
            Some((parent, boss)) => format!(
                "You manage it as {}, and report to {} in {}.",
                mine.title,
                named(boss),
                parent.name
            ),
            None => format!("You manage it as {}.", mine.title),
        };
    };
    match member(team, boss) {
        Some(boss) => format!("You are {} and report to {}.", mine.title, named(boss)),
        None => format!("You are {}.", mine.title),
    }
}

fn roster(team: &Team, me: Uuid) -> String {
    team.members
        .iter()
        .filter(|member| !member.retired)
        .map(|member| {
            let who = if member.agent == me {
                format!("you ({})", member.title)
            } else {
                named(member)
            };
            match member.reports_to.and_then(|boss| self::member(team, boss)) {
                None => format!("{who}, manager"),
                Some(boss) if boss.agent == me => format!("{who}, reports to you"),
                Some(boss) => format!("{who}, reports to {}", boss.name),
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn child_line(team: &Team, child: &Team) -> Option<String> {
    let manager = child.members.iter().find(|m| m.reports_to.is_none())?;
    let boss = member(team, child.reports_to?)?;
    Some(format!(
        "{}, managed by {}, who reports to {}",
        child.name,
        named(manager),
        boss.name
    ))
}

fn named(member: &TeamMember) -> String {
    format!("{} ({})", member.name, member.title)
}

fn member(team: &Team, agent: Uuid) -> Option<&TeamMember> {
    team.members.iter().find(|member| member.agent == agent)
}

fn manages(team: &Team, agent: Uuid) -> bool {
    member(team, agent).is_some_and(|member| member.reports_to.is_none())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{member as save, team};
    use super::*;

    fn titled(mut team: Team, names: &[(&str, &str)]) -> Team {
        for (member, (name, title)) in team.members.iter_mut().zip(names) {
            member.name = (*name).into();
            member.title = (*title).into();
        }
        team
    }

    #[test]
    fn an_agent_sees_its_place_its_team_and_the_teams_under_it() {
        let (cto, lead, dev, ops) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let (top, eng, ktlo) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let mut engineering = titled(
            team(
                eng,
                "Engineering",
                Some((top, cto)),
                &[save(lead, None), save(dev, Some(lead))],
            ),
            &[("Lia", "Engineering manager"), ("Dev", "Backend engineer")],
        );
        engineering.instructions = "Ship small changes.".into();
        engineering.members[1].role = "Own the API.".into();
        let org = vec![
            titled(
                team(top, "Leadership", None, &[save(cto, None)]),
                &[("Cyd", "CTO")],
            ),
            engineering,
            titled(
                team(ktlo, "KTLO", Some((eng, lead)), &[save(ops, None)]),
                &[("Opal", "KTLO lead")],
            ),
        ];

        let (text, manager) = brief(&org, dev).unwrap();
        assert!(!manager);
        assert!(!text.contains("team_member"));
        assert!(text.contains(
            "## Team Engineering\nYou are Backend engineer and report to Lia (Engineering manager)."
        ));
        assert!(text.contains("Team instructions:\nShip small changes."));
        assert!(text.contains("Your role:\nOwn the API."));
        assert!(text.contains(
            "Members: Lia (Engineering manager), manager; you (Backend engineer), reports to Lia."
        ));
        assert!(
            text.contains("Teams under it: KTLO, managed by Opal (KTLO lead), who reports to Lia.")
        );

        let (text, manager) = brief(&org, lead).unwrap();
        assert!(manager);
        assert!(text.contains("team_member"));
        assert!(text.contains(
            "You manage it as Engineering manager, and report to Cyd (CTO) in Leadership."
        ));
        assert!(text.contains("Dev (Backend engineer), reports to you"));
        assert!(!text.contains("Purpose:"));

        assert!(brief(&org, Uuid::new_v4()).is_none());
    }
}

//! An agent asks to create an agent, room or team, and a person settles the request in the console.
//! Approving runs the console's own creation as that person, so their authority, the connection
//! defaults and the default skills apply exactly as when they create it themselves. Names resolve
//! to ids only then, so one request may name an agent another pending request creates.
use super::{CreateRoom, CreateTeam, SaveMember, SaveTeam, Team, Work, teams};
use crate::{
    agents::{AgentId, Agents, CreateAgent, runtime::RuntimeActor},
    identity::{
        Actor, IdentityError,
        audit::{Event, record},
    },
};
use cubebot_protocol::runtime::RoutineCall;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use std::collections::HashMap;
use uuid::Uuid;

type Answer = Result<String, String>;

/// Enough to settle a stream of requests without letting one agent flood the console.
const PENDING: i64 = 20;

#[derive(Serialize, utoipa::ToSchema)]
pub struct CreationRequest {
    pub id: Uuid,
    /// `agent`, `room` or `team`.
    pub kind: String,
    /// The tool input as the agent sent it: untrusted text to show, never markup.
    #[schema(value_type = Object)]
    pub input: Value,
    pub created_at: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclineCreation {
    /// Returned to the agent; empty when the person gave none.
    pub reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewAgent {
    name: String,
    instructions: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewRoom {
    name: String,
    agents: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewTeam {
    name: String,
    description: String,
    instructions: String,
    parent: Option<String>,
    reports_to: Option<String>,
    members: Vec<NewMember>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewMember {
    agent: String,
    title: String,
    role: String,
    reports_to: Option<String>,
}

/// `platform_list` and the creation requests; `person` is the turn's person.
pub(super) async fn tool(
    tx: &mut Transaction<'_, Postgres>,
    actor: &RuntimeActor,
    (turn, person): (Uuid, Uuid),
    call: &RoutineCall,
) -> Result<Answer, IdentityError> {
    let creates = sqlx::query_scalar!("SELECT creates FROM agents WHERE id=$1", actor.agent.0)
        .fetch_one(&mut **tx)
        .await?;
    if !creates {
        return Ok(Err(
            "your capability to create agents, rooms and teams is off; a person can turn it on in your settings".into(),
        ));
    }
    let (kind, name) = match call.tool.as_str() {
        "platform_list" => return Ok(Ok(listed(tx, person).await?.to_string())),
        "agent_create" => ("agent", parsed::<NewAgent>(&call.input).map(|new| new.name)),
        "room_create" => ("room", parsed::<NewRoom>(&call.input).map(|new| new.name)),
        "team_create" => ("team", parsed::<NewTeam>(&call.input).map(|new| new.name)),
        _ => return Ok(Err("no creation tool has this name".into())),
    };
    let Some(name) = name.filter(|name| !name.trim().is_empty() && name.len() <= 255) else {
        return Ok(Err(format!(
            "the {kind} needs a name of 1 to 255 bytes and every field the tool lists"
        )));
    };
    ask(tx, actor, (turn, call), kind, name.trim()).await
}

fn parsed<T: for<'de> Deserialize<'de>>(input: &Value) -> Option<T> {
    T::deserialize(input).ok()
}

async fn ask(
    tx: &mut Transaction<'_, Postgres>,
    actor: &RuntimeActor,
    (turn, call): (Uuid, &RoutineCall),
    kind: &str,
    name: &str,
) -> Result<Answer, IdentityError> {
    let asked = format!(
        "A person was asked to approve creating the {kind} {name}. End this turn now with a short note; a new message tells you when they approve or decline."
    );
    let previous = sqlx::query_scalar!(
        "SELECT agent_id FROM creation_requests WHERE call_id=$1",
        call.call,
    )
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(agent) = previous {
        return Ok((agent == actor.agent.0)
            .then_some(asked)
            .ok_or_else(|| "this call id was already used".into()));
    }
    let pending = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM creation_requests WHERE agent_id=$1 AND status='pending'"#,
        actor.agent.0,
    )
    .fetch_one(&mut **tx)
    .await?;
    if pending >= PENDING {
        return Ok(Err(format!(
            "{PENDING} of your requests already wait for a person; wait until they settle"
        )));
    }
    sqlx::query!(
        "
        INSERT INTO creation_requests (agent_id, turn_id, call_id, kind, input)
        VALUES ($1, $2, $3, $4, $5)
        ",
        actor.agent.0,
        turn,
        call.call,
        kind,
        call.input,
    )
    .execute(&mut **tx)
    .await?;
    let event = Event::by_agent(actor.agent.0, "creation.request").on(actor.agent.0);
    record(
        &mut **tx,
        &event.detail(json!({ "kind": kind, "name": name })),
    )
    .await?;
    Ok(Ok(asked))
}

/// What the person can see: the agents they may use, and the rooms and teams they may read.
async fn listed(tx: &mut Transaction<'_, Postgres>, person: Uuid) -> Result<Value, sqlx::Error> {
    let mut agents = super::tasks::eligible(tx, person).await?;
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    let rooms = sqlx::query!(
        r#"
        SELECT r.id, r.name, ARRAY(SELECT g.name FROM room_members m JOIN agents g ON g.id=m.agent_id WHERE m.room_id=r.id AND g.desired_state<>'retired' ORDER BY g.name) AS "agents!"
        FROM rooms r
        JOIN human_accounts a ON a.id=$1 AND a.active
        WHERE NOT r.task AND NOT r.archived AND (a.role='administrator'
            OR EXISTS(SELECT 1 FROM effective_human_grants g WHERE g.actor_id=a.id AND g.scope_kind='room' AND g.scope_id=r.id AND g.capability='conversation.read'))
        ORDER BY r.name
        "#,
        person,
    )
    .fetch_all(&mut **tx)
    .await?;
    let readable: Vec<Uuid> = rooms.iter().map(|room| room.id).collect();
    let org = teams::load(tx).await?;
    let names: HashMap<Uuid, &str> = org.iter().map(|t| (t.id, t.name.as_str())).collect();
    Ok(json!({
        "agents": agents.iter().map(|agent| json!({"name": agent.name, "about": agent.about, "skills": agent.skills})).collect::<Vec<_>>(),
        "rooms": rooms.iter().map(|room| json!({"name": room.name, "agents": room.agents})).collect::<Vec<_>>(),
        "teams": org.iter().filter(|team| readable.contains(&team.room)).map(|team| json!({
            "name": team.name,
            "description": team.description,
            "parent": team.parent.and_then(|parent| names.get(&parent)),
            "members": team.members.iter().filter(|m| !m.retired).map(|m| json!({"agent": m.name, "title": m.title})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    }))
}

/// What was made: its kind, id and name, or why it could not be.
type Made = Result<(Uuid, String), String>;

impl Work {
    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub async fn pending_creations(
        &self,
        agent: AgentId,
    ) -> Result<Vec<CreationRequest>, IdentityError> {
        Ok(sqlx::query_as!(
            CreationRequest,
            r#"
            SELECT id, kind, input, (to_jsonb(created_at) #>> '{}') AS "created_at!"
            FROM creation_requests
            WHERE agent_id=$1 AND status='pending'
            ORDER BY created_at, id
            "#,
            agent.0,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Creates it as `actor`. A refusal the agent can fix settles the request as failed and tells
    /// the agent; any other error leaves it pending, such as for an administrator to approve.
    #[tracing::instrument(skip_all, fields(agent = %agent.0, request = %id))]
    pub async fn approve_creation(
        &self,
        actor: &Actor,
        agent: AgentId,
        id: Uuid,
    ) -> Result<Made, IdentityError> {
        let (kind, input) = self.claim_creation(agent, id, actor.id).await?;
        let made = match self.make_creation(actor, id, &kind, &input).await {
            Ok(made) => made,
            Err(IdentityError::InvalidInput) => Err("the platform refused its fields".into()),
            Err(IdentityError::Conflict) => Err("it conflicts with what exists now".into()),
            Err(error) => {
                self.reopen_creation(id).await?;
                return Err(error);
            }
        };
        let message = match &made {
            Ok((_, name)) => {
                format!("A person approved your request: the {kind} {name} now exists.")
            }
            Err(reason) => format!(
                "A person approved your request to create a {kind}, but it could not be created: {reason}. Fix it and ask again."
            ),
        };
        let (status, outcome, created) = match &made {
            Ok((created, _)) => ("approved", None, Some(*created)),
            Err(reason) => ("failed", Some(reason.as_str()), None),
        };
        self.settle_creation(id, (status, outcome, created), &message)
            .await?;
        Ok(made)
    }

    #[tracing::instrument(skip_all, fields(agent = %agent.0, request = %id))]
    pub async fn decline_creation(
        &self,
        actor: &Actor,
        agent: AgentId,
        id: Uuid,
        reason: &str,
    ) -> Result<String, IdentityError> {
        let (kind, _) = self.claim_creation(agent, id, actor.id).await?;
        let message = format!("A person declined your request to create a {kind}: {reason}");
        self.settle_creation(id, ("declined", Some(reason), None), &message)
            .await?;
        Ok(kind)
    }

    /// Takes the pending request, so two people settling it at once cannot both create it.
    async fn claim_creation(
        &self,
        agent: AgentId,
        id: Uuid,
        person: Uuid,
    ) -> Result<(String, Value), IdentityError> {
        let claimed = sqlx::query!(
            "
            UPDATE creation_requests
            SET status='approved', settled_by=$3, settled_at=clock_timestamp()
            WHERE id=$1 AND agent_id=$2 AND status='pending'
            RETURNING kind, input
            ",
            id,
            agent.0,
            person,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::NotFound)?;
        Ok((claimed.kind, claimed.input))
    }

    async fn reopen_creation(&self, id: Uuid) -> Result<(), IdentityError> {
        sqlx::query!(
            "UPDATE creation_requests SET status='pending', settled_by=NULL, settled_at=NULL WHERE id=$1",
            id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Records the outcome and tells the agent, as the person whose turn asked.
    async fn settle_creation(
        &self,
        id: Uuid,
        (status, outcome, created): (&str, Option<&str>, Option<Uuid>),
        message: &str,
    ) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        let asked = sqlx::query!(
            "
            UPDATE creation_requests c
            SET status=$2, outcome=$3, created_id=$4
            FROM chat_turns t
            WHERE c.id=$1 AND t.id=c.turn_id
            RETURNING c.agent_id, t.actor_id
            ",
            id,
            status,
            outcome,
            created,
        )
        .fetch_one(&mut *tx)
        .await?;
        super::wake(&mut tx, AgentId(asked.agent_id)).await?;
        sqlx::query!(
            "
            INSERT INTO chat_turns (conversation_id, actor_id, request_id, message)
            VALUES ($1, $2, gen_random_uuid(), $3)
            ",
            asked.agent_id,
            asked.actor_id,
            message,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The request id is the creation's request id, so a retried approval finds what it made.
    async fn make_creation(
        &self,
        actor: &Actor,
        id: Uuid,
        kind: &str,
        input: &Value,
    ) -> Result<Made, IdentityError> {
        let invalid = || Ok(Err("its fields do not match the tool".into()));
        match kind {
            "agent" => {
                let Some(new) = parsed::<NewAgent>(input) else {
                    return invalid();
                };
                let create = CreateAgent {
                    request_id: id,
                    name: new.name.trim().to_owned(),
                    instructions: new.instructions,
                    avatar: None,
                    voice: None,
                };
                let receipt = Agents::new(self.pool.clone())
                    .create(actor, &create)
                    .await?;
                Ok(Ok((receipt.id, create.name)))
            }
            "room" => match parsed::<NewRoom>(input) {
                Some(new) => self.make_room(actor, id, new).await,
                None => invalid(),
            },
            _ => match parsed::<NewTeam>(input) {
                Some(new) => self.make_team(actor, id, new).await,
                None => invalid(),
            },
        }
    }

    async fn make_room(
        &self,
        actor: &Actor,
        id: Uuid,
        new: NewRoom,
    ) -> Result<Made, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let agents = match resolved(&mut tx, &new.agents).await? {
            Ok(agents) => agents,
            Err(reason) => return Ok(Err(reason)),
        };
        tx.commit().await?;
        let create = CreateRoom {
            request_id: id,
            name: new.name,
            agents: new.agents.iter().map(|name| agents[name]).collect(),
        };
        let room = self.create_room(actor, &create).await?;
        Ok(Ok((room.id, room.name)))
    }

    async fn make_team(
        &self,
        actor: &Actor,
        id: Uuid,
        new: NewTeam,
    ) -> Result<Made, IdentityError> {
        let mut tx = self.pool.begin().await?;
        let org = teams::load(&mut tx).await?;
        let mut names: Vec<String> = new.members.iter().map(|m| m.agent.clone()).collect();
        names.extend(new.reports_to.iter().cloned());
        let agents = match resolved(&mut tx, &names).await? {
            Ok(agents) => agents,
            Err(reason) => return Ok(Err(reason)),
        };
        tx.commit().await?;
        let team = match drafted(&org, &agents, new) {
            Ok(team) => team,
            Err(reason) => return Ok(Err(reason)),
        };
        if let Err(reason) = teams::checked(&team).and_then(|()| teams::placed(&org, None, &team)) {
            return Ok(Err(reason.into()));
        }
        let team = self
            .create_team(
                actor,
                &CreateTeam {
                    request_id: id,
                    team,
                },
            )
            .await?;
        Ok(Ok((team.id, team.name)))
    }
}

/// Each live agent's id by its exact name, or why a name does not pick exactly one.
async fn resolved(
    tx: &mut Transaction<'_, Postgres>,
    names: &[String],
) -> Result<Result<HashMap<String, Uuid>, String>, sqlx::Error> {
    let found = sqlx::query!(
        "SELECT id, name FROM agents WHERE desired_state<>'retired' AND name = ANY($1)",
        names,
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut ids = HashMap::new();
    for name in names {
        let matching: Vec<Uuid> = found
            .iter()
            .filter(|a| &a.name == name)
            .map(|a| a.id)
            .collect();
        match matching[..] {
            [id] => ids.insert(name.clone(), id),
            [] => return Ok(Err(format!("no agent is called {name}"))),
            _ => return Ok(Err(format!("several agents are called {name}"))),
        };
    }
    Ok(Ok(ids))
}

/// The team with names swapped for ids; members already resolved through `agents`.
fn drafted(org: &[Team], agents: &HashMap<String, Uuid>, new: NewTeam) -> Result<SaveTeam, String> {
    let parent = match &new.parent {
        Some(name) => Some(
            org.iter()
                .find(|team| team.name.trim().eq_ignore_ascii_case(name.trim()))
                .map(|team| team.id)
                .ok_or_else(|| format!("no team is called {name}"))?,
        ),
        None => None,
    };
    let members = new
        .members
        .into_iter()
        .map(|member| {
            let reports_to = match member.reports_to {
                Some(name) => Some(
                    agents
                        .get(&name)
                        .copied()
                        .ok_or_else(|| format!("{name} is not a member"))?,
                ),
                None => None,
            };
            Ok(SaveMember {
                agent: agents[&member.agent],
                title: member.title,
                role: member.role,
                reports_to,
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(SaveTeam {
        name: new.name,
        description: new.description,
        instructions: new.instructions,
        parent,
        reports_to: new.reports_to.map(|name| agents[&name]),
        members,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_team_names_its_parent_and_lines_by_name() {
        let parent = Uuid::from_u128(1);
        let org = vec![Team {
            id: parent,
            name: "Engineering".into(),
            description: String::new(),
            instructions: String::new(),
            parent: None,
            reports_to: None,
            room: Uuid::nil(),
            revision: 1,
            members: Vec::new(),
        }];
        let (lead, dev) = (Uuid::from_u128(2), Uuid::from_u128(3));
        let agents = HashMap::from([("Lead".to_owned(), lead), ("Dev".to_owned(), dev)]);
        let new = |parent: &str| NewTeam {
            name: "Web".into(),
            description: "Ships the site.".into(),
            instructions: String::new(),
            parent: Some(parent.into()),
            reports_to: Some("Lead".into()),
            members: vec![
                NewMember {
                    agent: "Lead".into(),
                    title: "Lead".into(),
                    role: String::new(),
                    reports_to: None,
                },
                NewMember {
                    agent: "Dev".into(),
                    title: "Dev".into(),
                    role: String::new(),
                    reports_to: Some("Lead".into()),
                },
            ],
        };
        let team = drafted(&org, &agents, new("engineering")).unwrap();
        assert_eq!((team.parent, team.reports_to), (Some(parent), Some(lead)));
        assert_eq!(team.members[1].reports_to, Some(lead));
        assert_eq!(
            drafted(&org, &agents, new("Sales")).unwrap_err(),
            "no team is called Sales"
        );
    }
}

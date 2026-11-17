use super::{AgentState, Apps, Change, Register};
use crate::{
    agents::{AgentId, runtime::RuntimeActor},
    identity::IdentityError,
};
use cubebot_protocol::{apps::Desired, runtime::RoutineCall};
use serde::Deserialize;
use serde_json::Value;

type Answer = Result<String, String>;

const INVALID: &str = "the input does not match this tool's fields";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Slug {
    slug: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Move {
    slug: String,
    state: String,
}

fn parsed<T: for<'de> Deserialize<'de>>(input: &Value) -> Result<T, String> {
    T::deserialize(input).map_err(|_| INVALID.into())
}

fn url(slug: &str) -> String {
    format!(
        "http://127.0.0.1:{}/{slug}/",
        cubebot_protocol::apps::LOCAL_PORT
    )
}

fn moved(input: Move) -> Result<(String, AgentState), String> {
    let (desired, updating) = match input.state.as_str() {
        "running" => (Some(Desired::Running), Some(false)),
        "stopped" => (Some(Desired::Stopped), Some(false)),
        "updating" => (None, Some(true)),
        _ => return Err("state is running, stopped or updating".into()),
    };
    Ok((input.slug, AgentState { desired, updating }))
}

impl Apps {
    pub(crate) async fn tool(
        &self,
        actor: &RuntimeActor,
        call: &RoutineCall,
    ) -> Result<Answer, IdentityError> {
        let (agent, input) = (actor.agent, &call.input);
        match call.tool.as_str() {
            "app_list" => Ok(Ok(
                serde_json::to_string(&self.specs(agent).await?).unwrap_or_default()
            )),
            "app_register" => self.registered(agent, input).await,
            "app_update" => self.app_updated(agent, input).await,
            "app_state" => self.moved(agent, input).await,
            "app_remove" => self.removed(agent, input).await,
            _ => Ok(Err("no app tool has this name".into())),
        }
    }

    async fn registered(&self, agent: AgentId, input: &Value) -> Result<Answer, IdentityError> {
        let Ok(input) = parsed::<Register>(input) else {
            return Ok(Err(INVALID.into()));
        };
        let app = self.register(agent, input).await?;
        Ok(app.map(|app| format!("Registered {}. Test it at {}", app.slug, url(&app.slug))))
    }

    async fn app_updated(&self, agent: AgentId, input: &Value) -> Result<Answer, IdentityError> {
        let Ok(change) = parsed::<Change>(input) else {
            return Ok(Err(INVALID.into()));
        };
        let app = self.update(agent, change).await?;
        Ok(app.map(|app| format!("Updated {}. Test it at {}", app.slug, url(&app.slug))))
    }

    async fn moved(&self, agent: AgentId, input: &Value) -> Result<Answer, IdentityError> {
        let parsed = parsed::<Move>(input).and_then(moved);
        let (slug, state) = match parsed {
            Ok(parsed) => parsed,
            Err(reason) => return Ok(Err(reason)),
        };
        let app = self.agent_state(agent, &slug, state).await?;
        Ok(app.map(|app| {
            let state = serde_json::to_value(app.state).unwrap_or_default();
            format!(
                "{} is now {}. {}",
                app.slug,
                state.as_str().unwrap_or_default(),
                url(&app.slug)
            )
        }))
    }

    async fn removed(&self, agent: AgentId, input: &Value) -> Result<Answer, IdentityError> {
        let Ok(input) = parsed::<Slug>(input) else {
            return Ok(Err(INVALID.into()));
        };
        let removed = self.unregister(agent, &input.slug).await?;
        Ok(removed.map(|()| format!("Removed {}.", input.slug)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_state_maps_to_desired_and_the_updating_flag() {
        let to = |state: &str| {
            moved(Move {
                slug: "a".into(),
                state: state.into(),
            })
        };
        let (_, running) = to("running").unwrap();
        assert_eq!(
            (running.desired, running.updating),
            (Some(Desired::Running), Some(false))
        );
        let (_, updating) = to("updating").unwrap();
        assert_eq!((updating.desired, updating.updating), (None, Some(true)));
        assert!(to("paused").is_err());
    }

    #[test]
    fn unknown_fields_are_refused() {
        assert!(parsed::<Slug>(&json!({"slug": "a", "x": 1})).is_err());
        assert_eq!(url("notes"), "http://127.0.0.1:7400/notes/");
    }
}

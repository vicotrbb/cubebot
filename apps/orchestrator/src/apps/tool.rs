use super::{AgentState, AppState, Apps, Change, Register};
use crate::{
    agents::{AgentId, runtime::RuntimeActor},
    identity::IdentityError,
};
use cubebot_protocol::{
    apps::{AppSpec, Desired},
    runtime::RoutineCall,
};
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

fn listing(apps: Vec<(AppSpec, AppState)>) -> String {
    let apps: Vec<Value> = apps
        .into_iter()
        .map(|(spec, state)| {
            let mut app = serde_json::to_value(&spec).unwrap_or_default();
            app["state"] = serde_json::to_value(state).unwrap_or_default();
            if state == AppState::Failed {
                app["log"] = format!(
                    "It keeps exiting. Read .cubebot-apps/{}.log in your workspace to see why.",
                    spec.slug
                )
                .into();
            }
            app
        })
        .collect();
    serde_json::to_string(&apps).unwrap_or_default()
}

impl Apps {
    pub(crate) async fn tool(
        &self,
        actor: &RuntimeActor,
        call: &RoutineCall,
    ) -> Result<Answer, IdentityError> {
        let (agent, input) = (actor.agent, &call.input);
        match call.tool.as_str() {
            "app_list" => Ok(Ok(listing(self.listed(agent).await?))),
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
    fn a_listing_shows_each_state_and_points_a_failed_app_at_its_log() {
        let spec = |slug: &str| AppSpec {
            slug: slug.into(),
            name: slug.into(),
            folder: slug.into(),
            command: None,
            port: None,
            tools: Vec::new(),
            desired: Desired::Running,
        };
        let listed: Value = serde_json::from_str(&listing(vec![
            (spec("notes"), AppState::Running),
            (spec("budget"), AppState::Failed),
        ]))
        .unwrap();
        assert_eq!(listed[0]["state"], "running");
        assert_eq!(listed[0]["folder"], "notes");
        assert!(listed[0].get("log").is_none());
        assert_eq!(listed[1]["state"], "failed");
        assert!(
            listed[1]["log"]
                .as_str()
                .unwrap()
                .contains(".cubebot-apps/budget.log")
        );
    }

    #[test]
    fn unknown_fields_are_refused() {
        assert!(parsed::<Slug>(&json!({"slug": "a", "x": 1})).is_err());
        assert_eq!(url("notes"), "http://127.0.0.1:7400/notes/");
    }
}

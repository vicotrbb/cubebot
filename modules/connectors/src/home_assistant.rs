//! Home Assistant's REST API with a long-lived access token
//! (https://developers.home-assistant.io/docs/api/rest/, now served at
//! https://contributors.home-assistant.io/core/internals/rest-api/; tokens:
//! https://www.home-assistant.io/docs/authentication/#your-account-profile). Every request goes
//! to the address the person entered, so the token never leaves their server.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Event, EventSpec, Field, FieldKind,
    Level, Manifest, PRIVATE_ADDRESS, Poll, Risk, Step, ToolSpec,
};
use reqwest::{Method, RequestBuilder};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const STATES: usize = 100;
const MOST_STATES: usize = 500;
/// The longest an entity's attributes may be once written as JSON.
const LONGEST_ATTRIBUTES: usize = 4_000;
const HISTORY: usize = 200;
const LONGEST_HOURS: u32 = 7 * 24;
/// Watched entities read per poll.
const WATCHED: usize = 20;

pub struct HomeAssistant;

static MANIFEST: Manifest = Manifest {
    id: "home-assistant",
    name: "Home Assistant",
    description: "Read and control the devices of a Home Assistant server, and hear when they change.",
    category: Category::Home,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "Long-lived access token",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "eyJhbGciOi…",
            help: "From your Home Assistant profile's Security tab.",
        }],
    }],
    config: &[
        Field {
            key: "url",
            label: "Server address",
            kind: FieldKind::Url,
            required: true,
            placeholder: "http://homeassistant.local:8123",
            help: "The address you open Home Assistant at.",
        },
        Field {
            key: "watched",
            label: "Watched entities",
            kind: FieldKind::Lines,
            required: false,
            placeholder: "binary_sensor.front_door",
            help: "Entity ids whose changes wake the agent, one per line; the first 20 are watched.",
        },
    ],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_states",
            description: "Entities and their current state, optionally only one domain (such as light) or those whose id or name contains a text.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"domain":{"type":"string","description":"Such as light, switch, sensor or climate."},"text":{"type":"string","description":"Part of an entity id or friendly name, such as kitchen."},"limit":{"type":"integer","minimum":1,"maximum":500}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_state",
            description: "One entity's state, attributes and when it last changed.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"entity_id":{"type":"string","description":"Such as light.kitchen."}},"required":["entity_id"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "call_service",
            description: "Run an action such as light.turn_on or lock.unlock on entities, with its data. Answers the states it changed.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"service":{"type":"string","description":"domain.service, such as light.turn_on."},"entity_id":{"type":"array","items":{"type":"string"},"description":"The entities to act on."},"data":{"type":"object","description":"The action's other fields, such as {\"brightness_pct\": 40}."}},"required":["service"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "history",
            description: "How an entity's state changed over the last hours, oldest first.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"entity_id":{"type":"string"},"hours":{"type":"integer","minimum":1,"maximum":168,"description":"How far back to look; 24 when absent."}},"required":["entity_id"],"additionalProperties":false}"#,
        },
    ],
    events: &[EventSpec {
        kind: "entity.changed",
        description: "A watched entity's state changed.",
        product: None,
    }],
    level: Level::Write,
    guide: &[
        Step {
            title: "Create a long-lived access token",
            body: "In Home Assistant open your profile (your name at the bottom of the sidebar), choose the Security tab, and under Long-lived access tokens choose Create token. Name it Cubebot and copy it: it is shown only once.",
            link: None,
            copy: None,
        },
        Step {
            title: "Enter the server's address",
            body: "Use the address you open Home Assistant at, such as http://homeassistant.local:8123 or http://192.168.1.20:8123, then paste the token. Optionally list entity ids to watch, such as binary_sensor.front_door, to wake the agent when they change.",
            link: None,
            copy: None,
        },
        PRIVATE_ADDRESS,
    ],
    warning: Some(
        "The token acts as your Home Assistant user: with Write, agents can run any action that user can, including unlocking doors. Use a non-administrator user for Cubebot if you can.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    domain: Option<String>,
    #[serde(default)]
    text: String,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    entity_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    service: String,
    #[serde(default)]
    entity_id: Vec<String>,
    data: Option<Map<String, Value>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct History {
    entity_id: String,
    hours: Option<u32>,
}

impl Connector for HomeAssistant {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let config = cx.json(api(cx, Method::GET, "config")?).await?;
            let name = config["location_name"].as_str().unwrap_or("Home Assistant");
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "list_states" => list(cx, input).await,
                "read_state" => read(cx, input).await,
                "call_service" => call(cx, input).await,
                "history" => history(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(poll(cx, cursor))
    }
}

/// A request to `/api/<path>` on the configured server, with the token.
fn api(cx: &Context, method: Method, path: &str) -> Result<RequestBuilder, Error> {
    let server = cx
        .setting("url")
        .ok_or_else(|| Error::Invalid("the connection has no server address".into()))?;
    let url = format!("{}/api/{path}", server.trim_end_matches('/'));
    Ok(cx.request(method, &url)?.bearer_auth(cx.token()?))
}

/// Lowercase letters, digits and underscores around one dot, so an id cannot change the path.
fn checked(id: &str, what: &str) -> Result<String, Error> {
    let word = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    match id.split_once('.') {
        Some((domain, name)) if word(domain) && word(name) => Ok(id.to_owned()),
        _ => Err(Error::Invalid(format!(
            "{id:?} is not a {what} such as light.kitchen"
        ))),
    }
}

/// A state as the model needs it, without its attributes.
fn brief(state: &Value) -> Value {
    json!({
        "entity_id": state["entity_id"],
        "name": state["attributes"]["friendly_name"],
        "state": state["state"],
        "unit": state["attributes"]["unit_of_measurement"],
        "last_changed": state["last_changed"],
    })
}

async fn list(cx: &Context, input: Value) -> Result<Value, Error> {
    let list: List = crate::input(input)?;
    let limit = list.limit.unwrap_or(STATES).clamp(1, MOST_STATES);
    let domain = list.domain.map(|domain| format!("{domain}."));
    let text = list.text.to_lowercase();
    let states = cx.json(api(cx, Method::GET, "states")?).await?;
    let matching: Vec<&Value> = states
        .as_array()
        .into_iter()
        .flatten()
        .filter(|state| {
            let id = state["entity_id"].as_str().unwrap_or_default();
            let name = state["attributes"]["friendly_name"]
                .as_str()
                .unwrap_or_default();
            domain.as_ref().is_none_or(|domain| id.starts_with(domain))
                && (id.contains(&text) || name.to_lowercase().contains(&text))
        })
        .collect();
    let shown: Vec<Value> = matching
        .iter()
        .take(limit)
        .map(|state| brief(state))
        .collect();
    Ok(json!({"states": shown, "more": matching.len().saturating_sub(limit)}))
}

async fn read(cx: &Context, input: Value) -> Result<Value, Error> {
    let id = checked(&crate::input::<Read>(input)?.entity_id, "entity id")?;
    let state = cx
        .json(api(cx, Method::GET, &format!("states/{id}"))?)
        .await?;
    let mut shown = brief(&state);
    let attributes = state["attributes"].to_string();
    shown["attributes"] = if attributes.len() <= LONGEST_ATTRIBUTES {
        state["attributes"].clone()
    } else {
        json!(
            attributes
                .chars()
                .take(LONGEST_ATTRIBUTES)
                .collect::<String>()
                + "…"
        )
    };
    shown["last_updated"] = state["last_updated"].clone();
    Ok(shown)
}

async fn call(cx: &Context, input: Value) -> Result<Value, Error> {
    let call: Call = crate::input(input)?;
    let service = checked(&call.service, "service")?.replacen('.', "/", 1);
    let mut body = call.data.unwrap_or_default();
    if !call.entity_id.is_empty() {
        for id in &call.entity_id {
            checked(id, "entity id")?;
        }
        body.insert("entity_id".into(), json!(call.entity_id));
    }
    let request = api(cx, Method::POST, &format!("services/{service}"))?.json(&body);
    let changed = cx.json(request).await?;
    // Older servers answer the bare list, newer ones may wrap it.
    let list = changed.get("changed_states").unwrap_or(&changed);
    let changed: Vec<Value> = list.as_array().into_iter().flatten().map(brief).collect();
    Ok(json!({"changed": changed}))
}

async fn history(cx: &Context, input: Value) -> Result<Value, Error> {
    let history: History = crate::input(input)?;
    let id = checked(&history.entity_id, "entity id")?;
    let hours = history.hours.unwrap_or(24).clamp(1, LONGEST_HOURS);
    let now = crate::dav::now_nanos() as i64 / 1_000_000_000;
    let start = crate::dav::stamp(now - i64::from(hours) * 3600);
    let request = api(cx, Method::GET, &format!("history/period/{start}"))?.query(&[
        ("filter_entity_id", id.as_str()),
        ("end_time", &crate::dav::stamp(now)),
        ("minimal_response", ""),
        ("no_attributes", ""),
    ]);
    let found = cx.json(request).await?;
    let states = found[0].as_array().map(Vec::as_slice).unwrap_or_default();
    let changes: Vec<Value> = states
        .iter()
        .skip(states.len().saturating_sub(HISTORY))
        .map(|state| json!({"state": state["state"], "at": state["last_changed"]}))
        .collect();
    let dropped = states.len().saturating_sub(HISTORY);
    Ok(json!({"entity_id": id, "changes": changes, "earlier_dropped": dropped}))
}

/// Each watched entity's current state against the one remembered. The cursor maps entity ids to
/// their last `last_changed`, so a change is reported once. shortcut: several changes between
/// two polls show as the latest one; read the history endpoint instead if that bites.
async fn poll(cx: &Context, cursor: Option<&str>) -> Result<Poll, Error> {
    let seen: Map<String, Value> = cursor
        .and_then(|cursor| serde_json::from_str(cursor).ok())
        .unwrap_or_default();
    let watched = cx.config()["watched"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut now = Map::new();
    let mut events = Vec::new();
    for id in watched.iter().filter_map(Value::as_str).take(WATCHED) {
        let Ok(id) = checked(id, "entity id") else {
            continue;
        };
        let state = match cx
            .json(api(cx, Method::GET, &format!("states/{id}"))?)
            .await
        {
            // A removed entity stops nothing else from being watched.
            Err(Error::NotFound(_)) => continue,
            other => other?,
        };
        let changed = state["last_changed"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if seen
            .get(&id)
            .is_some_and(|before| before != &json!(changed))
        {
            events.push(event(&id, &state, &changed));
        }
        now.insert(id, json!(changed));
    }
    Ok(Poll {
        events: if cursor.is_some() { events } else { Vec::new() },
        cursor: Some(Value::Object(now).to_string()),
    })
}

fn event(id: &str, state: &Value, changed: &str) -> Event {
    let name = state["attributes"]["friendly_name"].as_str().unwrap_or(id);
    let value = state["state"].as_str().unwrap_or_default();
    Event {
        kind: "entity.changed",
        id: format!("{id}@{changed}"),
        summary: format!("{name} is now {value}").chars().take(200).collect(),
        data: brief(state),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    const TOKEN: &str = "ha-long-lived-token";

    fn cx(fixture: &Fixture, watched: Value) -> Context {
        let credential = Credential::Fields {
            fields: [("token".into(), TOKEN.into())].into(),
        };
        let config = json!({"url": fixture.origin(), "watched": watched});
        Context::new(config, credential).with_origin(&fixture.origin())
    }

    /// Answers recorded from a fresh Home Assistant 2026.11.0 with a "Guest mode" toggle helper.
    fn recorded(name: &str) -> Value {
        let path = format!(
            "{}/fixtures/home-assistant/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn list_states_filters_by_domain_and_text_and_reads_one_entity() {
        let states = recorded("states");
        let sun = states
            .as_array()
            .unwrap()
            .iter()
            .find(|state| state["entity_id"] == "sun.sun")
            .unwrap()
            .clone();
        let fixture = Fixture::start(vec![
            Exchange::new("GET", "/api/states", 200, states),
            Exchange::new("GET", "/api/states/sun.sun", 200, sun),
        ])
        .await;
        let cx = cx(&fixture, json!([]));
        let input = json!({"domain": "sun", "text": "SUN", "limit": 5});
        let listed = HomeAssistant.call(&cx, "list_states", input).await.unwrap();
        assert_eq!(listed["states"].as_array().unwrap().len(), 1);
        assert_eq!(listed["states"][0]["entity_id"], "sun.sun");
        assert_eq!(listed["states"][0]["name"], "Sun");
        assert_eq!(listed["more"], 0);
        let read = HomeAssistant
            .call(&cx, "read_state", json!({"entity_id": "sun.sun"}))
            .await
            .unwrap();
        assert!(read["attributes"]["next_rising"].is_string());
        let seen = fixture.finish().await;
        let bearer = format!("Bearer {TOKEN}");
        assert_eq!(seen[0].header("authorization"), Some(bearer.as_str()));
    }

    #[tokio::test]
    async fn call_service_posts_the_entities_and_data_and_answers_what_changed() {
        let fixture = Fixture::start(vec![Exchange::new(
            "POST",
            "/api/services/input_boolean/turn_on",
            200,
            recorded("changed"),
        )])
        .await;
        let input = json!({"service": "input_boolean.turn_on", "entity_id": ["input_boolean.guest_mode"], "data": {"note": 1}});
        let called = HomeAssistant
            .call(&cx(&fixture, json!([])), "call_service", input)
            .await
            .unwrap();
        assert_eq!(
            called["changed"][0]["entity_id"],
            "input_boolean.guest_mode"
        );
        assert_eq!(called["changed"][0]["state"], "on");
        let sent = fixture.finish().await[0].json();
        assert_eq!(
            sent,
            json!({"note": 1, "entity_id": ["input_boolean.guest_mode"]})
        );
    }

    #[tokio::test]
    async fn history_asks_for_a_bounded_window_of_one_entity() {
        let input = json!({"entity_id": "input_boolean.guest_mode", "hours": 2});
        // The window starts two hours before now, so a second ticking over mid-call is retried.
        for _ in 0..3 {
            let now = crate::dav::now_nanos() as i64 / 1_000_000_000;
            let path = format!("/api/history/period/{}", crate::dav::stamp(now - 2 * 3600));
            let fixture =
                Fixture::start(vec![Exchange::new("GET", &path, 200, recorded("history"))]).await;
            let Ok(found) = HomeAssistant
                .call(&cx(&fixture, json!([])), "history", input.clone())
                .await
            else {
                continue;
            };
            assert_eq!(
                found["changes"],
                json!([
                    {"state": "off", "at": "2026-11-06T02:04:10.815019+00:00"},
                    {"state": "on", "at": "2026-11-06T02:04:22.660825+00:00"},
                    {"state": "off", "at": "2026-11-06T02:04:28.139692+00:00"},
                ])
            );
            let seen = fixture.finish().await;
            let query = seen[0].target.split_once('?').unwrap().1.to_owned();
            assert!(query.starts_with("filter_entity_id=input_boolean.guest_mode&end_time="));
            assert!(query.ends_with("&minimal_response=&no_attributes="));
            return;
        }
        panic!("history never asked for the expected window");
    }

    #[tokio::test]
    async fn poll_records_a_baseline_then_reports_each_change_once() {
        let mut state = recorded("guest_mode");
        let watched = json!(["input_boolean.guest_mode", "light.gone"]);
        let path = "/api/states/input_boolean.guest_mode";
        let first = state.clone();
        state["state"] = json!("on");
        state["last_changed"] = json!("2026-11-06T02:04:22.660825+00:00");
        let fixture = Fixture::start(vec![
            Exchange::new("GET", path, 200, first),
            Exchange::new(
                "GET",
                "/api/states/light.gone",
                404,
                json!({"message": "Entity not found."}),
            ),
            Exchange::new("GET", path, 200, state.clone()),
            Exchange::new("GET", "/api/states/light.gone", 404, json!({})),
            Exchange::new("GET", path, 200, state),
            Exchange::new("GET", "/api/states/light.gone", 404, json!({})),
        ])
        .await;
        let cx = cx(&fixture, watched);
        let baseline = HomeAssistant.poll(&cx, None).await.unwrap();
        assert!(baseline.events.is_empty());
        let cursor = baseline.cursor.unwrap();
        let changed = HomeAssistant.poll(&cx, Some(&cursor)).await.unwrap();
        assert_eq!(changed.events.len(), 1);
        let event = &changed.events[0];
        assert_eq!(
            event.id,
            "input_boolean.guest_mode@2026-11-06T02:04:22.660825+00:00"
        );
        assert_eq!(event.summary, "Guest mode is now on");
        let cursor = changed.cursor.unwrap();
        let again = HomeAssistant.poll(&cx, Some(&cursor)).await.unwrap();
        assert!(again.events.is_empty());
        fixture.finish().await;
    }

    #[tokio::test]
    async fn refusals_map_to_errors_and_bad_input_never_reaches_the_server() {
        let cases = [
            (401, Error::Unauthorized),
            (403, Error::Forbidden("Forbidden".into())),
            (404, Error::NotFound("Entity not found.".into())),
            (
                429,
                Error::Unavailable("rate limited: no reason given".into()),
            ),
        ];
        for (status, expected) in cases {
            let body = match status {
                403 => json!({"message": "Forbidden"}),
                404 => json!({"message": "Entity not found."}),
                _ => Value::Null,
            };
            let fixture = Fixture::start(vec![
                Exchange::new("GET", "/api/states/light.kitchen", status, body)
                    .header("retry-after", "3600"),
            ])
            .await;
            let read = HomeAssistant
                .call(
                    &cx(&fixture, json!([])),
                    "read_state",
                    json!({"entity_id": "light.kitchen"}),
                )
                .await;
            assert_eq!(read, Err(expected), "{status}");
            fixture.finish().await;
        }
        let fixture = Fixture::start(Vec::new()).await;
        let cx = cx(&fixture, json!([]));
        let bad = [
            ("read_state", json!({"entity_id": "../config"})),
            ("read_state", json!({"entity": "light.kitchen"})),
            ("call_service", json!({"service": "turn_on"})),
            (
                "call_service",
                json!({"service": "light.turn_on", "entity_id": ["light/x"]}),
            ),
        ];
        for (tool, input) in bad {
            let refused = HomeAssistant.call(&cx, tool, input).await;
            assert!(matches!(refused, Err(Error::Invalid(_))), "{refused:?}");
        }
        assert!(fixture.finish().await.is_empty());
    }

    /// Every tool and the poll against a real server: set `CUBEBOT_LIVE_HOME_ASSISTANT_URL` and
    /// `CUBEBOT_LIVE_HOME_ASSISTANT_TOKEN`, make an `input_boolean.guest_mode` toggle helper, and
    /// run `cargo test -p cubebot-connectors live_home_assistant -- --ignored`. It flips the toggle.
    #[tokio::test]
    #[ignore = "needs a Home Assistant server"]
    async fn live_home_assistant() {
        let env = |key: &str| std::env::var(format!("CUBEBOT_LIVE_HOME_ASSISTANT_{key}")).unwrap();
        let credential = Credential::Fields {
            fields: [("token".into(), env("TOKEN"))].into(),
        };
        let config = json!({"url": env("URL"), "watched": ["input_boolean.guest_mode"]});
        let cx = Context::new(config, credential);
        println!("check: {:?}", HomeAssistant.check(&cx).await.unwrap());
        let call = |tool: &'static str, input: Value| {
            let cx = &cx;
            async move {
                let answer = HomeAssistant.call(cx, tool, input).await;
                println!("{tool}: {answer:?}");
                answer.unwrap()
            }
        };
        let listed = call("list_states", json!({"domain": "input_boolean"})).await;
        assert_eq!(listed["states"][0]["entity_id"], "input_boolean.guest_mode");
        let guest = json!({"entity_id": "input_boolean.guest_mode"});
        let before = call("read_state", guest.clone()).await;
        let baseline = HomeAssistant.poll(&cx, None).await.unwrap();
        assert!(baseline.events.is_empty());
        let toggle = json!({"service": "input_boolean.toggle", "entity_id": [guest["entity_id"]]});
        let changed = call("call_service", toggle).await;
        assert_ne!(changed["changed"][0]["state"], before["state"]);
        let polled = HomeAssistant.poll(&cx, baseline.cursor.as_deref());
        let polled = polled.await.unwrap();
        println!("poll: {:?}", polled.events);
        assert_eq!(polled.events.len(), 1);
        let again = HomeAssistant.poll(&cx, polled.cursor.as_deref());
        assert!(again.await.unwrap().events.is_empty());
        call(
            "history",
            json!({"entity_id": guest["entity_id"], "hours": 1}),
        )
        .await;
        let missing = HomeAssistant.call(&cx, "read_state", json!({"entity_id": "light.nowhere"}));
        let missing = missing.await;
        assert!(matches!(missing, Err(Error::NotFound(_))), "{missing:?}");
    }
}

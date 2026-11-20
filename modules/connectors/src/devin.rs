//! Devin, Cognition's coding agent, through an organization service user over the v3 API.
//! Agents hand Devin a task as a session, which works in Devin's own machine, opens pull requests
//! and is polled for when it finishes or needs an answer (Devin has no webhooks).
//! Docs: https://docs.devin.ai/api-reference/v3/overview.md (permissions),
//! https://docs.devin.ai/api-reference/authentication.md (`cog_` keys as bearer tokens),
//! https://docs.devin.ai/api-reference/getting-started/teams-quickstart.md (service users and roles),
//! and the sessions, messages, attachments and self endpoints in https://docs.devin.ai/v3-openapi.json.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Event, EventSpec, Field, FieldKind,
    Level, Manifest, Poll, Risk, Step, ToolSpec,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const API: &str = "https://api.devin.ai/v3";
/// Every session agents start carries this tag, so listing and polling find only theirs.
const TAG: &str = "cubebot";
/// The ACU limit of a session when the connection sets none.
const ACUS: u64 = 10;
/// Devin answers at most 200 messages or sessions a page.
const PAGE: u8 = 200;
/// Pages read before giving up on reaching the end of a long list.
const PAGES: usize = 5;
/// The latest messages a read shows, each cut to this many characters.
const MESSAGES: usize = 30;
const MESSAGE: usize = 4_000;
/// Sessions whose last state is remembered between polls.
const REMEMBERED: usize = 200;

pub struct Devin;

static MANIFEST: Manifest = Manifest {
    id: "devin",
    name: "Devin",
    description: "Hand coding tasks to Devin, follow its sessions and pull requests, and answer it.",
    category: Category::Dev,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "Service user API key",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "cog_…",
            help: "From Settings › Devin API › Service users. Shown once.",
        }],
    }],
    config: &[
        Field {
            key: "org_id",
            label: "Organization ID",
            kind: FieldKind::Text,
            required: true,
            placeholder: "org-…",
            help: "At the top of Settings › Devin API.",
        },
        Field {
            key: "max_acus",
            label: "Most ACUs per session",
            kind: FieldKind::Number,
            required: false,
            placeholder: "10",
            help: "Each session an agent starts stops at this many ACUs. Agents may ask for less, never more. 10 when empty.",
        },
    ],
    products: &[],
    tools: &[
        ToolSpec {
            name: "create_session",
            description: "Start a Devin session on a task. Devin works on its own, opens pull requests and spends your Devin usage up to the connection's limit. It answers at once; read the session later or wait for its event.",
            risk: Risk::Send,
            product: None,
            input: r#"{"type":"object","properties":{"prompt":{"type":"string","description":"The task, with everything Devin needs: repository, goal, constraints and how to verify."},"title":{"type":"string"},"repos":{"type":"array","items":{"type":"string"},"description":"Repositories to work in, as owner/name."},"playbook_id":{"type":"string","description":"A Devin playbook to follow."},"max_acus":{"type":"integer","minimum":1,"description":"A lower ACU limit for this session than the connection's."},"structured_output_schema":{"type":"object","description":"A JSON Schema (Draft 7) Devin must fill as its final answer, read back as structured_output."}},"required":["prompt"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_sessions",
            description: "Devin sessions agents started, most recently updated first, with their state, pull requests and ACUs used.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"everyone":{"type":"boolean","description":"Also sessions people started in Devin itself."},"limit":{"type":"integer","minimum":1,"maximum":50},"after":{"type":"string","description":"The next value of a previous list, for its next page."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_session",
            description: "A Devin session's state, pull requests, structured output, latest messages and attachments.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"session":{"type":"string","description":"The session's id (devin-…) or its app.devin.ai URL."}},"required":["session"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "message_session",
            description: "Answer or instruct a Devin session; a sleeping session wakes up and continues.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"session":{"type":"string","description":"The session's id (devin-…) or its app.devin.ai URL."},"message":{"type":"string"}},"required":["session","message"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "terminate_session",
            description: "Stop a Devin session for good; it cannot be resumed.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"session":{"type":"string","description":"The session's id (devin-…) or its app.devin.ai URL."}},"required":["session"],"additionalProperties":false}"#,
        },
    ],
    events: &[
        EventSpec {
            kind: "session.finished",
            description: "A session an agent started finished or ended.",
            product: None,
        },
        EventSpec {
            kind: "session.waiting",
            description: "A session an agent started waits for an answer or an approval.",
            product: None,
        },
        EventSpec {
            kind: "session.failed",
            description: "A session an agent started failed or stopped, such as out of usage.",
            product: None,
        },
    ],
    level: Level::Full,
    guide: &[
        Step {
            title: "Provision a service user",
            body: "In Devin open Settings › Devin API, choose the Service users tab and Provision service user. Name it Cubebot and give it the Member role, which creates, views and manages sessions without changing your settings. Pick the longest expiration offered, or Never where allowed: the default is 30 days, after which this connection stops working until you paste a new key.",
            link: Some("https://app.devin.ai/settings"),
            copy: None,
        },
        Step {
            title: "Paste the key and your organization ID",
            body: "Devin shows the key, starting with cog_, once. Paste it below with the organization ID from the top of the same page, which starts with org-.",
            link: None,
            copy: None,
        },
        Step {
            title: "Set a limit per session",
            body: "Every session an agent starts stops at the ACU limit you set here (10 when empty), so one task cannot use up your plan. Devin uses your organization's repositories, secrets and knowledge, and opens pull requests through its own GitHub connection.",
            link: None,
            copy: None,
        },
    ],
    warning: Some(
        "Sessions spend your Devin usage and run in Devin's cloud machines, outside Cubebot's sandbox, with your Devin organization's repository access. Agents start them on their own at the Full level.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    prompt: String,
    title: Option<String>,
    repos: Option<Vec<String>>,
    playbook_id: Option<String>,
    max_acus: Option<u64>,
    structured_output_schema: Option<Map<String, Value>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    #[serde(default)]
    everyone: bool,
    limit: Option<u8>,
    after: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct One {
    session: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    session: String,
    message: String,
}

impl Connector for Devin {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let me = cx
                .json(cx.get(&format!("{API}/self"))?.bearer_auth(cx.token()?))
                .await?;
            let name = me["service_user_name"]
                .as_str()
                .or(me["user_name"].as_str())
                .unwrap_or("Devin API key");
            let org = org(cx)?;
            // The organization and the right to view its sessions, which polling needs.
            let query = [("first", "1")];
            cx.json(sessions(cx, &org, None)?.query(&query)).await?;
            Ok(Account {
                name: format!("{name} in {org}"),
            })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "create_session" => create(cx, input).await,
                "list_sessions" => list(cx, input).await,
                "read_session" => read(cx, input).await,
                "message_session" => message(cx, input).await,
                "terminate_session" => terminate(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(poll(cx, cursor))
    }
}

/// The organization ID setting, such as `org-abc123`.
fn org(cx: &Context) -> Result<String, Error> {
    let org = cx.setting("org_id").unwrap_or_default().trim();
    let valid = org.len() > 4
        && org.len() <= 64
        && org.starts_with("org-")
        && org
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| org.to_owned()).ok_or_else(|| {
        Error::Invalid("the organization ID starts with org-, from Settings › Devin API".into())
    })
}

/// A request under the organization's sessions, or one session's `rest` when `session` is given.
fn sessions(
    cx: &Context,
    org: &str,
    session: Option<(&str, &str)>,
) -> Result<reqwest::RequestBuilder, Error> {
    let url = match session {
        Some((id, rest)) => format!("{API}/organizations/{org}/sessions/{id}{rest}"),
        None => format!("{API}/organizations/{org}/sessions"),
    };
    Ok(cx.get(&url)?.bearer_auth(cx.token()?))
}

fn with(
    cx: &Context,
    method: reqwest::Method,
    url: &str,
) -> Result<reqwest::RequestBuilder, Error> {
    Ok(cx.request(method, url)?.bearer_auth(cx.token()?))
}

/// A session's id, `devin-…`, from itself, its bare id or its URL.
fn session_id(text: &str) -> Result<String, Error> {
    let text = text.trim();
    let found = match text.split_once("/sessions/") {
        Some((_, rest)) => rest.split(['/', '?', '#']).next().unwrap_or_default(),
        None => text,
    };
    let bare = found.strip_prefix("devin-").unwrap_or(found);
    let valid = !bare.is_empty()
        && bare.len() <= 128
        && bare
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid
        .then(|| format!("devin-{bare}"))
        .ok_or_else(|| Error::Invalid(format!("{text:?} is not a Devin session id or URL")))
}

/// The ACU limit a new session gets: the agent's, never above the connection's.
fn limit(cx: &Context, asked: Option<u64>) -> u64 {
    let most = cx.config()["max_acus"]
        .as_u64()
        .filter(|most| *most > 0)
        .unwrap_or(ACUS);
    asked
        .filter(|asked| *asked > 0)
        .map_or(most, |asked| asked.min(most))
}

async fn create(cx: &Context, input: Value) -> Result<Value, Error> {
    let create: Create = crate::input(input)?;
    if create.prompt.trim().is_empty() {
        return Err(Error::Invalid("the prompt is empty".into()));
    }
    let org = org(cx)?;
    let mut body = json!({
        "prompt": create.prompt,
        "tags": [TAG],
        "max_acu_limit": limit(cx, create.max_acus),
    });
    if let Some(title) = create.title {
        body["title"] = json!(title);
    }
    if let Some(repos) = create.repos.filter(|repos| !repos.is_empty()) {
        body["repos"] = json!(repos);
    }
    if let Some(playbook) = create.playbook_id {
        body["playbook_id"] = json!(playbook);
    }
    if let Some(schema) = create.structured_output_schema {
        body["structured_output_schema"] = Value::Object(schema);
    }
    let url = format!("{API}/organizations/{org}/sessions");
    let made = cx
        .json(with(cx, reqwest::Method::POST, &url)?.json(&body))
        .await?;
    Ok(summary(&made))
}

async fn list(cx: &Context, input: Value) -> Result<Value, Error> {
    let list: List = crate::input(input)?;
    let org = org(cx)?;
    let first = list.limit.unwrap_or(20).clamp(1, 50).to_string();
    let mut query = vec![("first", first)];
    if !list.everyone {
        query.push(("tags", TAG.to_owned()));
    }
    if let Some(after) = list.after {
        query.push(("after", after));
    }
    let page = cx.json(sessions(cx, &org, None)?.query(&query)).await?;
    let mut found: Vec<&Value> = page["items"].as_array().into_iter().flatten().collect();
    found.sort_by_key(|session| std::cmp::Reverse(session["updated_at"].as_i64()));
    let next = match page["has_next_page"].as_bool() {
        Some(true) => page["end_cursor"].clone(),
        _ => Value::Null,
    };
    Ok(json!({"sessions": found.into_iter().map(summary).collect::<Vec<_>>(), "next": next}))
}

async fn read(cx: &Context, input: Value) -> Result<Value, Error> {
    let id = session_id(&crate::input::<One>(input)?.session)?;
    let org = org(cx)?;
    let session = cx.json(sessions(cx, &org, Some((&id, "")))?).await?;
    let mut shown = summary(&session);
    shown["structured_output"] = session["structured_output"].clone();
    shown["messages"] = json!(latest(cx, &org, &id).await?);
    let attached = cx
        .json(sessions(cx, &org, Some((&id, "/attachments")))?)
        .await?;
    shown["attachments"] = attached
        .as_array()
        .into_iter()
        .flatten()
        .map(|file| json!({"name": file["name"], "url": file["url"], "from": file["source"]}))
        .collect();
    Ok(shown)
}

/// The latest messages, oldest first. Devin pages them in order from the first.
async fn latest(cx: &Context, org: &str, id: &str) -> Result<Vec<Value>, Error> {
    let mut messages = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..PAGES {
        let mut query = vec![("first", PAGE.to_string())];
        if let Some(after) = after.take() {
            query.push(("after", after));
        }
        let page = cx
            .json(sessions(cx, org, Some((id, "/messages")))?.query(&query))
            .await?;
        messages.extend(
            page["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|message| {
                    json!({
                        "from": message["source"],
                        "at": message["created_at"],
                        "text": cut(message["message"].as_str().unwrap_or_default(), MESSAGE),
                    })
                }),
        );
        let excess = messages.len().saturating_sub(MESSAGES);
        messages.drain(..excess);
        match (page["has_next_page"].as_bool(), page["end_cursor"].as_str()) {
            (Some(true), Some(cursor)) => after = Some(cursor.to_owned()),
            _ => break,
        }
    }
    Ok(messages)
}

async fn message(cx: &Context, input: Value) -> Result<Value, Error> {
    let message: Message = crate::input(input)?;
    if message.message.trim().is_empty() {
        return Err(Error::Invalid("the message is empty".into()));
    }
    let id = session_id(&message.session)?;
    let org = org(cx)?;
    let url = format!("{API}/organizations/{org}/sessions/{id}/messages");
    let body = json!({"message": message.message});
    let session = cx
        .json(with(cx, reqwest::Method::POST, &url)?.json(&body))
        .await?;
    Ok(summary(&session))
}

async fn terminate(cx: &Context, input: Value) -> Result<Value, Error> {
    let id = session_id(&crate::input::<One>(input)?.session)?;
    let org = org(cx)?;
    let url = format!("{API}/organizations/{org}/sessions/{id}");
    let session = cx.json(with(cx, reqwest::Method::DELETE, &url)?).await?;
    Ok(summary(&session))
}

fn cut(text: &str, longest: usize) -> String {
    match text.char_indices().nth(longest) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

/// What a session is doing, in one word an agent can act on.
fn state(session: &Value) -> &'static str {
    let detail = session["status_detail"].as_str().unwrap_or_default();
    match (session["status"].as_str().unwrap_or_default(), detail) {
        ("new" | "claimed" | "resuming", _) => "starting",
        ("running", "waiting_for_user") => "waiting_for_reply",
        ("running", "waiting_for_approval") => "waiting_for_approval",
        ("running", "finished") => "finished",
        ("running", _) => "working",
        ("suspended", "inactivity" | "user_request") => "asleep",
        ("suspended", _) => "stopped",
        ("exit", _) => "ended",
        ("error", _) => "failed",
        _ => "unknown",
    }
}

fn summary(session: &Value) -> Value {
    let pulls: Vec<Value> = session["pull_requests"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|pull| json!({"url": pull["pr_url"], "state": pull["pr_state"]}))
        .collect();
    json!({
        "id": session["session_id"],
        "title": session["title"],
        "state": state(session),
        "status": session["status"],
        "detail": session["status_detail"],
        "url": session["url"],
        "pull_requests": pulls,
        "acus": session["acus_consumed"],
        "created": session["created_at"],
        "updated": session["updated_at"],
    })
}

/// The event a session's new state raises, if any.
fn kind(state: &str) -> Option<&'static str> {
    match state {
        "finished" | "ended" => Some("session.finished"),
        "waiting_for_reply" | "waiting_for_approval" => Some("session.waiting"),
        "failed" | "stopped" => Some("session.failed"),
        _ => None,
    }
}

#[derive(Default, Deserialize, serde::Serialize)]
struct Seen {
    /// The latest `updated_at` seen, in Devin's own unit, so only newer changes are listed.
    since: i64,
    /// Each remembered session's last state and `updated_at`.
    states: Map<String, Value>,
}

/// Agents' sessions that reached a state worth acting on since the last poll. The cursor keeps
/// each session's last state, since Devin updates a session for every step it takes.
async fn poll(cx: &Context, cursor: Option<&str>) -> Result<Poll, Error> {
    let mut seen: Seen = cursor
        .and_then(|cursor| serde_json::from_str(cursor).ok())
        .unwrap_or_default();
    let org = org(cx)?;
    let mut events = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..PAGES {
        let mut query = vec![("first", PAGE.to_string()), ("tags", TAG.to_owned())];
        if seen.since > 0 {
            query.push(("updated_after", (seen.since - 1).to_string()));
        }
        if let Some(after) = after.take() {
            query.push(("after", after));
        }
        let page = cx.json(sessions(cx, &org, None)?.query(&query)).await?;
        for session in page["items"].as_array().into_iter().flatten() {
            if let Some(event) = noticed(&mut seen, session) {
                events.push(event);
            }
        }
        match (page["has_next_page"].as_bool(), page["end_cursor"].as_str()) {
            (Some(true), Some(next)) => after = Some(next.to_owned()),
            _ => break,
        }
    }
    forget(&mut seen);
    Ok(Poll {
        events: if cursor.is_some() { events } else { Vec::new() },
        cursor: serde_json::to_string(&seen).ok(),
    })
}

/// Records a session's state and answers the event when it changed into one worth reporting.
fn noticed(seen: &mut Seen, session: &Value) -> Option<Event> {
    let id = session["session_id"].as_str()?;
    let updated = session["updated_at"].as_i64().unwrap_or_default();
    seen.since = seen.since.max(updated);
    let now = state(session);
    let before = seen.states.insert(id.to_owned(), json!([now, updated]));
    let changed = before.is_none_or(|before| before[0] != now);
    let kind = kind(now).filter(|_| changed)?;
    let title = session["title"].as_str().unwrap_or("A Devin session");
    let summary = match kind {
        "session.finished" => format!("Devin finished: {title}"),
        "session.waiting" => format!("Devin is waiting for an answer: {title}"),
        _ => format!("Devin stopped: {title}"),
    };
    Some(Event {
        kind,
        id: format!("{id}:{now}:{updated}"),
        summary,
        data: summary_of(session),
    })
}

fn summary_of(session: &Value) -> Value {
    let mut data = summary(session);
    data["structured_output"] = session["structured_output"].clone();
    data
}

/// Keeps only the most recently updated sessions.
fn forget(seen: &mut Seen) {
    if seen.states.len() <= REMEMBERED {
        return;
    }
    let mut order: Vec<(String, i64)> = seen
        .states
        .iter()
        .map(|(id, value)| (id.clone(), value[1].as_i64().unwrap_or_default()))
        .collect();
    order.sort_by_key(|(_, updated)| std::cmp::Reverse(*updated));
    for (id, _) in order.into_iter().skip(REMEMBERED) {
        seen.states.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    const SESSIONS: &str = "/api.devin.ai/v3/organizations/org-abc123def456/sessions";
    const ONE: &str = "/api.devin.ai/v3/organizations/org-abc123def456/sessions/devin-abc123def456";

    fn cx_with(fixture: &Fixture, config: Value) -> Context {
        let credential = Credential::Fields {
            fields: [("token".into(), "cog_secret".into())].into(),
        };
        Context::new(config, credential).with_origin(&fixture.origin())
    }

    fn cx(fixture: &Fixture) -> Context {
        cx_with(fixture, json!({"org_id": "org-abc123def456"}))
    }

    fn recorded(name: &str) -> Value {
        let path = format!("{}/fixtures/devin/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn with_state(status: &str, detail: Option<&str>, updated: i64) -> Value {
        let mut session = recorded("session");
        session["status"] = json!(status);
        session["status_detail"] = json!(detail);
        session["updated_at"] = json!(updated);
        session
    }

    fn page(items: Vec<Value>) -> Value {
        json!({"items": items, "has_next_page": false, "end_cursor": null})
    }

    #[tokio::test]
    async fn create_tags_the_session_and_holds_it_to_the_connections_acu_limit() {
        let mut started = recorded("session");
        started["status"] = json!("new");
        started["status_detail"] = Value::Null;
        let fixture = Fixture::start(vec![
            Exchange::new("POST", SESSIONS, 200, started.clone()),
            Exchange::new("POST", SESSIONS, 200, started),
        ])
        .await;
        let cx = cx_with(
            &fixture,
            json!({"org_id": "org-abc123def456", "max_acus": 8}),
        );
        let schema = json!({"type": "object", "properties": {"fixed": {"type": "boolean"}}});
        let input = json!({"prompt": "Fix the flaky login test", "title": "Flaky login", "repos": ["acme/web"], "max_acus": 50, "structured_output_schema": schema});
        let made = Devin.call(&cx, "create_session", input).await.unwrap();
        assert_eq!(made["id"], "devin-abc123def456");
        assert_eq!(made["state"], "starting");
        assert_eq!(made["url"], "https://app.devin.ai/sessions/abc123def456");
        let asked_less = json!({"prompt": "Small fix", "max_acus": 2});
        Devin.call(&cx, "create_session", asked_less).await.unwrap();
        let seen = fixture.finish().await;
        assert_eq!(seen[0].header("authorization"), Some("Bearer cog_secret"));
        assert_eq!(
            seen[0].json(),
            json!({"prompt": "Fix the flaky login test", "tags": ["cubebot"], "max_acu_limit": 8, "title": "Flaky login", "repos": ["acme/web"], "structured_output_schema": schema})
        );
        assert_eq!(seen[1].json()["max_acu_limit"], 2);
    }

    #[test]
    fn an_unset_or_zero_limit_falls_back_to_ten_acus() {
        let none = Context::new(
            json!({}),
            Credential::Token {
                token: String::new(),
            },
        );
        assert_eq!(limit(&none, None), ACUS);
        assert_eq!(limit(&none, Some(0)), ACUS);
        assert_eq!(limit(&none, Some(3)), 3);
        let zero = Context::new(
            json!({"max_acus": 0}),
            Credential::Token {
                token: String::new(),
            },
        );
        assert_eq!(limit(&zero, Some(40)), ACUS);
    }

    #[tokio::test]
    async fn list_shows_agents_sessions_newest_first_unless_asked_for_everyones() {
        let older = with_state("running", Some("working"), 100);
        let mut newer = with_state("suspended", Some("out_of_credits"), 200);
        newer["session_id"] = json!("devin-second");
        let mut more = page(vec![older, newer]);
        more["has_next_page"] = json!(true);
        more["end_cursor"] = json!("cursor-2");
        let fixture = Fixture::start(vec![
            Exchange::new("GET", SESSIONS, 200, more),
            Exchange::new("GET", SESSIONS, 200, page(Vec::new())),
        ])
        .await;
        let found = Devin
            .call(&cx(&fixture), "list_sessions", json!({"limit": 5}))
            .await
            .unwrap();
        assert_eq!(found["sessions"][0]["id"], "devin-second");
        assert_eq!(found["sessions"][0]["state"], "stopped");
        assert_eq!(found["sessions"][1]["state"], "working");
        assert_eq!(found["next"], "cursor-2");
        let input = json!({"everyone": true, "after": "cursor-2"});
        Devin
            .call(&cx(&fixture), "list_sessions", input)
            .await
            .unwrap();
        let seen = fixture.finish().await;
        assert!(
            seen[0].target.ends_with("sessions?first=5&tags=cubebot"),
            "{}",
            seen[0].target
        );
        assert!(
            seen[1].target.ends_with("sessions?first=20&after=cursor-2"),
            "{}",
            seen[1].target
        );
    }

    #[tokio::test]
    async fn read_takes_a_url_and_shows_the_result_messages_and_attachments() {
        let attachments = json!([{"attachment_id": "a1", "name": "report.md", "url": "https://files.devin.ai/a1", "source": "devin", "content_type": "text/markdown"}]);
        let fixture = Fixture::start(vec![
            Exchange::new("GET", ONE, 200, recorded("session")),
            Exchange::new("GET", &format!("{ONE}/messages"), 200, recorded("messages")),
            Exchange::new("GET", &format!("{ONE}/attachments"), 200, attachments),
        ])
        .await;
        let input = json!({"session": "https://app.devin.ai/sessions/abc123def456"});
        let read = Devin
            .call(&cx(&fixture), "read_session", input)
            .await
            .unwrap();
        assert_eq!(read["state"], "finished");
        assert_eq!(
            read["pull_requests"][0],
            json!({"url": "https://github.com/acme/web/pull/42", "state": "open"})
        );
        assert_eq!(read["structured_output"]["fixed"], true);
        assert_eq!(read["acus"], 3.5);
        assert_eq!(read["messages"][1]["from"], "devin");
        assert!(
            read["messages"][1]["text"]
                .as_str()
                .unwrap()
                .starts_with("Found a race")
        );
        assert_eq!(
            read["attachments"][0],
            json!({"name": "report.md", "url": "https://files.devin.ai/a1", "from": "devin"})
        );
        let seen = fixture.finish().await;
        assert!(
            seen[1].target.ends_with("/messages?first=200"),
            "{}",
            seen[1].target
        );
    }

    #[tokio::test]
    async fn a_long_conversation_keeps_its_latest_messages() {
        let message = |n: usize| json!({"event_id": format!("ev-{n}"), "source": "devin", "message": format!("step {n}"), "created_at": n});
        let mut first = page((0..200).map(message).collect());
        first["has_next_page"] = json!(true);
        first["end_cursor"] = json!("m-2");
        let second = page((200..210).map(message).collect());
        let fixture = Fixture::start(vec![
            Exchange::new("GET", &format!("{ONE}/messages"), 200, first),
            Exchange::new(
                "GET",
                &format!("{ONE}/messages?first=200&after=m-2"),
                200,
                second,
            ),
        ])
        .await;
        let messages = latest(&cx(&fixture), "org-abc123def456", "devin-abc123def456")
            .await
            .unwrap();
        assert_eq!(messages.len(), MESSAGES);
        assert_eq!(messages[0]["text"], "step 180");
        assert_eq!(messages[MESSAGES - 1]["text"], "step 209");
        fixture.finish().await;
    }

    #[tokio::test]
    async fn message_and_terminate_reach_the_session_by_its_bare_id() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "POST",
                &format!("{ONE}/messages"),
                200,
                with_state("resuming", None, 300),
            ),
            Exchange::new("DELETE", ONE, 200, with_state("exit", None, 301)),
        ])
        .await;
        let input = json!({"session": "abc123def456", "message": "Also update the changelog."});
        let answered = Devin
            .call(&cx(&fixture), "message_session", input)
            .await
            .unwrap();
        assert_eq!(answered["state"], "starting");
        let ended = Devin
            .call(
                &cx(&fixture),
                "terminate_session",
                json!({"session": "devin-abc123def456"}),
            )
            .await
            .unwrap();
        assert_eq!(ended["state"], "ended");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].json(),
            json!({"message": "Also update the changelog."})
        );
        assert_eq!(seen[1].method, "DELETE");
    }

    #[tokio::test]
    async fn refusals_map_to_errors_and_bad_input_never_reaches_devin() {
        let problem = |status: u16, detail: &str| json!({"type": "about:blank", "title": "Problem", "status": status, "detail": detail});
        let cases = [
            (
                Exchange::new("GET", ONE, 401, problem(401, "Invalid API key")),
                Error::Unauthorized,
            ),
            (
                Exchange::new("GET", ONE, 403, problem(403, "Missing ViewOrgSessions")),
                Error::Forbidden("Missing ViewOrgSessions".into()),
            ),
            (
                Exchange::new("GET", ONE, 404, problem(404, "Session not found")),
                Error::NotFound("Session not found".into()),
            ),
        ];
        for (exchange, expected) in cases {
            let fixture = Fixture::start(vec![exchange]).await;
            let read = Devin
                .call(
                    &cx(&fixture),
                    "read_session",
                    json!({"session": "devin-abc123def456"}),
                )
                .await;
            assert_eq!(read, Err(expected));
            fixture.finish().await;
        }
        let fixture = Fixture::start(vec![]).await;
        let bad = [
            (
                "read_session",
                json!({"session": "devin-../../self"}),
                json!({"org_id": "org-abc123def456"}),
            ),
            (
                "read_session",
                json!({"session": "devin-abc", "x": 1}),
                json!({"org_id": "org-abc123def456"}),
            ),
            (
                "message_session",
                json!({"session": "devin-abc", "message": "  "}),
                json!({"org_id": "org-abc123def456"}),
            ),
            (
                "create_session",
                json!({"prompt": "Go"}),
                json!({"org_id": "acme/../x"}),
            ),
            (
                "create_session",
                json!({"prompt": ""}),
                json!({"org_id": "org-abc123def456"}),
            ),
        ];
        for (tool, input, config) in bad {
            let refused = Devin.call(&cx_with(&fixture, config), tool, input).await;
            assert!(
                matches!(refused, Err(Error::Invalid(_))),
                "{tool}: {refused:?}"
            );
        }
        assert!(fixture.finish().await.is_empty());
    }

    #[tokio::test]
    async fn poll_records_a_baseline_then_reports_each_change_of_state_once() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                SESSIONS,
                200,
                page(vec![with_state("running", Some("working"), 100)]),
            ),
            Exchange::new(
                "GET",
                SESSIONS,
                200,
                page(vec![with_state("running", Some("waiting_for_user"), 110)]),
            ),
            Exchange::new(
                "GET",
                SESSIONS,
                200,
                page(vec![with_state("running", Some("waiting_for_user"), 115)]),
            ),
            Exchange::new(
                "GET",
                SESSIONS,
                200,
                page(vec![with_state("running", Some("finished"), 130)]),
            ),
            Exchange::new(
                "GET",
                SESSIONS,
                200,
                page(vec![with_state("suspended", Some("inactivity"), 200)]),
            ),
        ])
        .await;
        let cx = cx(&fixture);
        let baseline = Devin.poll(&cx, None).await.unwrap();
        assert!(baseline.events.is_empty());
        let waiting = Devin.poll(&cx, baseline.cursor.as_deref()).await.unwrap();
        assert_eq!(waiting.events.len(), 1);
        assert_eq!(waiting.events[0].kind, "session.waiting");
        assert_eq!(
            waiting.events[0].summary,
            "Devin is waiting for an answer: Fix the flaky login test"
        );
        let still = Devin.poll(&cx, waiting.cursor.as_deref()).await.unwrap();
        assert!(
            still.events.is_empty(),
            "another step in the same state is not news"
        );
        let finished = Devin.poll(&cx, still.cursor.as_deref()).await.unwrap();
        let event = &finished.events[0];
        assert_eq!(event.kind, "session.finished");
        assert_eq!(event.id, "devin-abc123def456:finished:130");
        assert_eq!(
            event.data["pull_requests"][0]["url"],
            "https://github.com/acme/web/pull/42"
        );
        assert_eq!(event.data["structured_output"]["fixed"], true);
        let asleep = Devin.poll(&cx, finished.cursor.as_deref()).await.unwrap();
        assert!(
            asleep.events.is_empty(),
            "falling asleep after finishing is not news"
        );
        let seen = fixture.finish().await;
        assert!(
            seen[0].target.ends_with("sessions?first=200&tags=cubebot"),
            "{}",
            seen[0].target
        );
        assert!(
            seen[1].target.ends_with("&tags=cubebot&updated_after=99"),
            "{}",
            seen[1].target
        );
    }

    #[tokio::test]
    async fn a_session_that_finished_between_polls_is_still_reported() {
        let mut quick = with_state("exit", None, 150);
        quick["session_id"] = json!("devin-quick");
        let failed = {
            let mut failed = with_state("error", None, 160);
            failed["session_id"] = json!("devin-broken");
            failed
        };
        let fixture = Fixture::start(vec![
            Exchange::new("GET", SESSIONS, 200, page(Vec::new())),
            Exchange::new("GET", SESSIONS, 200, page(vec![quick, failed])),
        ])
        .await;
        let cx = cx(&fixture);
        let baseline = Devin.poll(&cx, None).await.unwrap();
        let polled = Devin.poll(&cx, baseline.cursor.as_deref()).await.unwrap();
        let kinds: Vec<&str> = polled.events.iter().map(|event| event.kind).collect();
        assert_eq!(kinds, ["session.finished", "session.failed"]);
        fixture.finish().await;
    }

    #[test]
    fn the_cursor_remembers_only_the_latest_sessions() {
        let mut seen = Seen::default();
        for n in 0..(REMEMBERED + 50) {
            let mut session = with_state("running", Some("working"), n as i64);
            session["session_id"] = json!(format!("devin-{n}"));
            noticed(&mut seen, &session);
        }
        forget(&mut seen);
        assert_eq!(seen.states.len(), REMEMBERED);
        assert!(
            seen.states
                .contains_key(&format!("devin-{}", REMEMBERED + 49))
        );
        assert!(!seen.states.contains_key("devin-0"));
    }

    /// Against a real organization: `CUBEBOT_LIVE_DEVIN_TOKEN` (a `cog_` service user key) and
    /// `CUBEBOT_LIVE_DEVIN_ORG_ID`. Starts one session limited to 1 ACU and terminates it.
    #[tokio::test]
    #[ignore = "needs a Devin organization and spends up to 1 ACU"]
    async fn live_devin() {
        let env = |key: &str| std::env::var(format!("CUBEBOT_LIVE_DEVIN_{key}")).unwrap();
        let credential = Credential::Fields {
            fields: [("token".into(), env("TOKEN"))].into(),
        };
        let cx = Context::new(json!({"org_id": env("ORG_ID"), "max_acus": 1}), credential);
        println!("check: {:?}", Devin.check(&cx).await.unwrap());
        let baseline = Devin.poll(&cx, None).await.unwrap();
        let schema = json!({"type": "object", "properties": {"word": {"type": "string"}}, "required": ["word"]});
        let prompt = "Cubebot connector live test. Do not open, clone or change any repository. Answer with the word ready as your structured output, then finish.";
        let input = json!({"prompt": prompt, "title": "Cubebot live test", "structured_output_schema": schema});
        let made = Devin.call(&cx, "create_session", input).await.unwrap();
        println!("created: {made}");
        let id = made["id"].as_str().unwrap().to_owned();
        let mut cursor = baseline.cursor;
        let mut events = Vec::new();
        for _ in 0..60 {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            let polled = Devin.poll(&cx, cursor.as_deref()).await.unwrap();
            cursor = polled.cursor;
            events.extend(polled.events);
            if events.iter().any(|event| event.kind != "session.waiting") {
                break;
            }
        }
        println!("events: {events:?}");
        let read = Devin
            .call(&cx, "read_session", json!({"session": id}))
            .await
            .unwrap();
        println!("read: {read}");
        let listed = Devin
            .call(&cx, "list_sessions", json!({"limit": 5}))
            .await
            .unwrap();
        assert!(
            listed["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|session| session["id"] == id.as_str())
        );
        let ended = Devin
            .call(&cx, "terminate_session", json!({"session": id}))
            .await;
        println!("terminated: {ended:?}");
        assert!(
            !events.is_empty(),
            "the session raised no event in ten minutes"
        );
    }

    #[test]
    fn session_ids_come_from_ids_bare_ids_and_urls() {
        assert_eq!(session_id("devin-abc123").unwrap(), "devin-abc123");
        assert_eq!(session_id(" abc123 ").unwrap(), "devin-abc123");
        assert_eq!(
            session_id("https://app.devin.ai/sessions/abc123?tab=pr").unwrap(),
            "devin-abc123"
        );
        for bad in [
            "",
            "devin-",
            "abc/123",
            "https://app.devin.ai/sessions/",
            "a b",
        ] {
            assert!(session_id(bad).is_err(), "{bad}");
        }
    }
}

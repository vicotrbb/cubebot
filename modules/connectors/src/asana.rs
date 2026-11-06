//! Asana through a personal access token sent as a bearer token.
//! Docs: https://developers.asana.com/docs/personal-access-token,
//! https://developers.asana.com/reference/gettasks, https://developers.asana.com/reference/typeaheadforworkspace,
//! https://developers.asana.com/reference/getstoriesfortask, https://developers.asana.com/docs/pagination
//! and https://developers.asana.com/docs/errors.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Event, EventSpec, Field, FieldKind,
    Level, Manifest, Poll, Risk, Step, ToolSpec,
};
use reqwest::{Method, RequestBuilder};
use serde::Deserialize;
use serde_json::{Value, json};

const API: &str = "https://app.asana.com/api/1.0";
const TASK: &str = "name,completed,due_on,assignee.name,projects.name,permalink_url,modified_at";
/// Asana's page size limit.
const MOST: u8 = 100;
/// Notes and comments are cut to this many characters.
const TEXT: usize = 20_000;
const COMMENT: usize = 2_000;
const COMMENTS: usize = 20;
/// Task ids already reported, so a repeat is recognised.
const REMEMBERED: usize = 300;

pub struct Asana;

static MANIFEST: Manifest = Manifest {
    id: "asana",
    name: "Asana",
    description: "Find, read, create and complete Asana tasks and comment on them.",
    category: Category::Work,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "Personal access token",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "2/1204…",
            help: "From the Asana developer console.",
        }],
    }],
    config: &[Field {
        key: "workspace",
        label: "Workspace id",
        kind: FieldKind::Text,
        required: false,
        placeholder: "1204567890123456",
        help: "Only if you belong to several workspaces: the number after /0/home/ or /1/ in Asana's address. Empty means your first workspace.",
    }],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_my_tasks",
            description: "Your open tasks in the workspace, with their projects and due dates.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"limit":{"type":"integer","minimum":1,"maximum":100},"next":{"type":"string","description":"The next value of a previous list, for its next page."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "search_tasks",
            description: "Find tasks in the workspace whose names match some words.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["query"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_task",
            description: "A task's details, notes and latest comments.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"task":{"type":"string","description":"The task's id or URL."}},"required":["task"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "create_task",
            description: "Create a task, in a project or in My Tasks, optionally assigned and due.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"name":{"type":"string"},"notes":{"type":"string"},"project":{"type":"string","description":"A project id; without one the task is created in the workspace."},"assignee":{"type":"string","description":"me, a user's email address, or a user id."},"due_on":{"type":"string","description":"YYYY-MM-DD."}},"required":["name"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "update_task",
            description: "Change a task's name, notes, assignee or due date, or mark it complete.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"task":{"type":"string","description":"The task's id or URL."},"name":{"type":"string"},"notes":{"type":"string","description":"Replaces the notes."},"completed":{"type":"boolean"},"assignee":{"type":"string","description":"me, a user's email address, a user id, or none to unassign."},"due_on":{"type":"string","description":"YYYY-MM-DD, or empty to clear it."}},"required":["task"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "comment_on_task",
            description: "Add a comment to a task; its followers are notified.",
            risk: Risk::Send,
            product: None,
            input: r#"{"type":"object","properties":{"task":{"type":"string","description":"The task's id or URL."},"text":{"type":"string"}},"required":["task","text"],"additionalProperties":false}"#,
        },
    ],
    events: &[EventSpec {
        kind: "task.assigned",
        description: "An open task was newly assigned to you.",
        product: None,
    }],
    level: Level::Write,
    guide: &[
        Step {
            title: "Create a personal access token",
            body: "Open Asana's developer console, choose Create new token (under Personal access tokens), name it Cubebot, accept the API terms and create it.",
            link: Some("https://app.asana.com/0/my-apps"),
            copy: None,
        },
        Step {
            title: "Paste it below",
            body: "Asana shows the token once. Copy it and paste it here. It acts as you: tasks and comments agents make carry your name.",
            link: None,
            copy: None,
        },
    ],
    warning: Some(
        "The token acts as you in every workspace you belong to. On Enterprise plans an administrator can restrict or expire personal access tokens; sign in again with a new one if that happens.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    limit: Option<u8>,
    next: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: String,
    limit: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Task {
    task: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    name: String,
    notes: Option<String>,
    project: Option<String>,
    assignee: Option<String>,
    due_on: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    task: String,
    name: Option<String>,
    notes: Option<String>,
    completed: Option<bool>,
    assignee: Option<String>,
    due_on: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Comment {
    task: String,
    text: String,
}

impl Connector for Asana {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let me = cx.json(me(cx)?).await?;
            let user = &me["data"];
            let name = user["email"]
                .as_str()
                .or(user["name"].as_str())
                .unwrap_or("Asana account");
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "list_my_tasks" => list(cx, input).await,
                "search_tasks" => search(cx, input).await,
                "read_task" => read(cx, input).await,
                "create_task" => create(cx, input).await,
                "update_task" => update(cx, input).await,
                "comment_on_task" => comment(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(poll(cx, cursor))
    }
}

fn request(cx: &Context, method: Method, path: &str) -> Result<RequestBuilder, Error> {
    Ok(cx
        .request(method, &format!("{API}{path}"))?
        .bearer_auth(cx.token()?))
}

fn me(cx: &Context) -> Result<RequestBuilder, Error> {
    Ok(request(cx, Method::GET, "/users/me")?.query(&[("opt_fields", "name,email,workspaces")]))
}

/// The workspace setting, or the person's first workspace.
async fn workspace(cx: &Context) -> Result<String, Error> {
    if let Some(chosen) = cx.setting("workspace") {
        return gid(chosen, "workspace");
    }
    let me = cx.json(me(cx)?).await?;
    me["data"]["workspaces"][0]["gid"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::NotFound("the account belongs to no workspace".into()))
}

/// An Asana id: digits only, so it can sit in a path.
fn gid(text: &str, what: &str) -> Result<String, Error> {
    let text = text.trim();
    let valid = !text.is_empty() && text.len() <= 32 && text.bytes().all(|b| b.is_ascii_digit());
    valid
        .then(|| text.to_owned())
        .ok_or_else(|| Error::Invalid(format!("{text:?} is not an Asana {what} id")))
}

/// A task's id from itself or its URL: the segment after `/task/`, else the last numeric one.
fn task(text: &str) -> Result<String, Error> {
    let path = text.trim().split(['?', '#']).next().unwrap_or_default();
    let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let after_task = segments
        .windows(2)
        .find(|pair| pair[0] == "task")
        .map(|pair| pair[1]);
    let last_number = segments
        .iter()
        .rev()
        .find(|part| part.bytes().all(|b| b.is_ascii_digit()))
        .copied();
    let found = after_task.or(last_number).unwrap_or(path);
    gid(found, "task")
}

fn shown(task: &Value) -> Value {
    let projects: Vec<Value> = task["projects"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|project| project["name"].clone())
        .collect();
    json!({
        "id": task["gid"],
        "name": task["name"],
        "completed": task["completed"],
        "due_on": task["due_on"],
        "assignee": task["assignee"]["name"],
        "projects": projects,
        "url": task["permalink_url"],
        "modified": task["modified_at"],
    })
}

fn tasks(found: &Value) -> Vec<Value> {
    found["data"]
        .as_array()
        .into_iter()
        .flatten()
        .map(shown)
        .collect()
}

async fn list(cx: &Context, input: Value) -> Result<Value, Error> {
    let list: List = crate::input(input)?;
    let workspace = workspace(cx).await?;
    let limit = list.limit.unwrap_or(50).clamp(1, MOST).to_string();
    let mut query = vec![
        ("assignee", "me"),
        ("workspace", workspace.as_str()),
        ("completed_since", "now"),
        ("limit", limit.as_str()),
        ("opt_fields", TASK),
    ];
    if let Some(next) = list.next.as_deref() {
        query.push(("offset", next));
    }
    let found = cx
        .json(request(cx, Method::GET, "/tasks")?.query(&query))
        .await?;
    Ok(json!({"tasks": tasks(&found), "next": found["next_page"]["offset"]}))
}

async fn search(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: Search = crate::input(input)?;
    let path = format!("/workspaces/{}/typeahead", workspace(cx).await?);
    let count = search.limit.unwrap_or(20).clamp(1, MOST).to_string();
    let query = [
        ("resource_type", "task"),
        ("query", search.query.as_str()),
        ("count", count.as_str()),
        ("opt_fields", TASK),
    ];
    let found = cx
        .json(request(cx, Method::GET, &path)?.query(&query))
        .await?;
    Ok(json!({"tasks": tasks(&found)}))
}

async fn read(cx: &Context, input: Value) -> Result<Value, Error> {
    let id = task(&crate::input::<Task>(input)?.task)?;
    let fields = format!("{TASK},notes,created_at");
    let request_task =
        request(cx, Method::GET, &format!("/tasks/{id}"))?.query(&[("opt_fields", fields)]);
    let found = cx.json(request_task).await?;
    let stories = request(cx, Method::GET, &format!("/tasks/{id}/stories"))?.query(&[
        (
            "opt_fields",
            "text,created_at,created_by.name,resource_subtype",
        ),
        ("limit", "100"),
    ]);
    let stories = cx.json(stories).await?;
    let comments: Vec<Value> = stories["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|story| story["resource_subtype"] == "comment_added")
        .map(|story| {
            json!({
                "author": story["created_by"]["name"],
                "created": story["created_at"],
                "text": cut(story["text"].as_str().unwrap_or_default(), COMMENT),
            })
        })
        .collect();
    let task = &found["data"];
    let mut shown = shown(task);
    shown["created"] = task["created_at"].clone();
    shown["notes"] = json!(cut(task["notes"].as_str().unwrap_or_default(), TEXT));
    shown["comments"] = json!(comments[comments.len().saturating_sub(COMMENTS)..]);
    Ok(shown)
}

fn cut(text: &str, longest: usize) -> String {
    match text.char_indices().nth(longest) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

/// An assignee as Asana takes it: `me`, an email address or a user id; `none` for nobody.
fn assignee(who: &str) -> Value {
    match who.trim() {
        "none" => Value::Null,
        who => json!(who),
    }
}

/// `YYYY-MM-DD`, or empty to clear a due date.
fn due(text: &str) -> Result<Value, Error> {
    let bytes = text.as_bytes();
    let shaped = bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, b)| match index {
            4 | 7 => *b == b'-',
            _ => b.is_ascii_digit(),
        });
    match (text.is_empty(), shaped) {
        (true, _) => Ok(Value::Null),
        (false, true) => Ok(json!(text)),
        (false, false) => Err(Error::Invalid(format!("{text:?} is not a YYYY-MM-DD date"))),
    }
}

async fn create(cx: &Context, input: Value) -> Result<Value, Error> {
    let create: Create = crate::input(input)?;
    let mut data = json!({"name": create.name});
    match create.project {
        Some(project) => data["projects"] = json!([gid(&project, "project")?]),
        None => data["workspace"] = json!(workspace(cx).await?),
    }
    if let Some(notes) = create.notes {
        data["notes"] = json!(notes);
    }
    if let Some(who) = create.assignee {
        data["assignee"] = assignee(&who);
    }
    if let Some(due_on) = create.due_on {
        data["due_on"] = due(&due_on)?;
    }
    let body = json!({"data": data});
    let request = request(cx, Method::POST, "/tasks")?
        .query(&[("opt_fields", TASK)])
        .json(&body);
    let made = cx.json(request).await?;
    Ok(shown(&made["data"]))
}

async fn update(cx: &Context, input: Value) -> Result<Value, Error> {
    let update: Update = crate::input(input)?;
    let id = task(&update.task)?;
    let mut data = json!({});
    if let Some(name) = update.name {
        data["name"] = json!(name);
    }
    if let Some(notes) = update.notes {
        data["notes"] = json!(notes);
    }
    if let Some(completed) = update.completed {
        data["completed"] = json!(completed);
    }
    if let Some(who) = update.assignee {
        data["assignee"] = assignee(&who);
    }
    if let Some(due_on) = update.due_on {
        data["due_on"] = due(&due_on)?;
    }
    let request = request(cx, Method::PUT, &format!("/tasks/{id}"))?
        .query(&[("opt_fields", TASK)])
        .json(&json!({"data": data}));
    let changed = cx.json(request).await?;
    Ok(shown(&changed["data"]))
}

async fn comment(cx: &Context, input: Value) -> Result<Value, Error> {
    let comment: Comment = crate::input(input)?;
    let path = format!("/tasks/{}/stories", task(&comment.task)?);
    let body = json!({"data": {"text": comment.text}});
    let made = cx
        .json(request(cx, Method::POST, &path)?.json(&body))
        .await?;
    Ok(json!({"id": made["data"]["gid"], "created": made["data"]["created_at"]}))
}

/// Open tasks newly assigned to the person. The cursor is the JSON list of task ids already seen.
async fn poll(cx: &Context, cursor: Option<&str>) -> Result<Poll, Error> {
    let mut seen: Vec<String> = cursor
        .and_then(|cursor| serde_json::from_str(cursor).ok())
        .unwrap_or_default();
    let workspace = workspace(cx).await?;
    // shortcut: the first page of 100 open tasks; a task further down that was never seen reads as new, page if that bites.
    let query = [
        ("assignee", "me"),
        ("workspace", workspace.as_str()),
        ("completed_since", "now"),
        ("limit", "100"),
        ("opt_fields", TASK),
    ];
    let found = cx
        .json(request(cx, Method::GET, "/tasks")?.query(&query))
        .await?;
    let mut events = Vec::new();
    for task in tasks(&found) {
        let Some(id) = task["id"].as_str().map(str::to_owned) else {
            continue;
        };
        if seen.contains(&id) {
            continue;
        }
        seen.push(id.clone());
        events.push(Event {
            kind: "task.assigned",
            summary: format!(
                "Assigned to you: {}",
                task["name"].as_str().unwrap_or_default()
            ),
            id,
            data: task,
        });
    }
    let excess = seen.len().saturating_sub(REMEMBERED);
    seen.drain(..excess);
    Ok(Poll {
        events: if cursor.is_some() { events } else { Vec::new() },
        cursor: Some(json!(seen).to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    fn cx(fixture: &Fixture, config: Value) -> Context {
        let credential = Credential::Fields {
            fields: [("token".into(), "2/1201/1209:secret".into())].into(),
        };
        Context::new(config, credential).with_origin(&fixture.origin())
    }

    fn chosen(fixture: &Fixture) -> Context {
        cx(fixture, json!({"workspace": "1204000000000001"}))
    }

    fn recorded(name: &str) -> Value {
        let path = format!("{}/fixtures/asana/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn at(method: &str, path: &str, status: u16, body: Value) -> Exchange {
        Exchange::new(
            method,
            &format!("/app.asana.com/api/1.0{path}"),
            status,
            body,
        )
    }

    #[tokio::test]
    async fn list_my_tasks_finds_the_first_workspace_and_pages() {
        let me = recorded("check")["exchanges"][0]["body"].clone();
        let fixture = Fixture::start(vec![
            at("GET", "/users/me", 200, me),
            at("GET", "/tasks", 200, recorded("tasks")),
        ])
        .await;
        let listed = Asana
            .call(
                &cx(&fixture, json!({})),
                "list_my_tasks",
                json!({"limit": 2, "next": "eyJ0eXAiOJiKV1"}),
            )
            .await
            .unwrap();
        assert_eq!(listed["tasks"][0]["name"], "Draft the launch post");
        assert_eq!(listed["tasks"][0]["projects"], json!(["Launch"]));
        assert_eq!(listed["next"], "eyJ0eXAiOJiKV1iQLCJhbGciOiJIUzI1NiJ9");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[1].header("authorization"),
            Some("Bearer 2/1201/1209:secret")
        );
        assert_eq!(
            seen[1].target,
            "/app.asana.com/api/1.0/tasks?assignee=me&workspace=1204000000000001&completed_since=now&limit=2&opt_fields=name%2Ccompleted%2Cdue_on%2Cassignee.name%2Cprojects.name%2Cpermalink_url%2Cmodified_at&offset=eyJ0eXAiOJiKV1"
        );
    }

    #[tokio::test]
    async fn search_tasks_uses_typeahead_in_the_chosen_workspace() {
        let fixture = Fixture::start(vec![at(
            "GET",
            "/workspaces/1204000000000001/typeahead",
            200,
            recorded("tasks"),
        )])
        .await;
        let found = Asana
            .call(
                &chosen(&fixture),
                "search_tasks",
                json!({"query": "launch"}),
            )
            .await
            .unwrap();
        assert_eq!(found["tasks"].as_array().unwrap().len(), 2);
        let target = &fixture.finish().await[0].target;
        assert!(
            target.contains("resource_type=task&query=launch&count=20"),
            "{target}"
        );
    }

    #[tokio::test]
    async fn read_task_takes_a_url_and_keeps_only_comments() {
        let fixture = Fixture::start(vec![
            at("GET", "/tasks/1209000000000011", 200, recorded("task")),
            at(
                "GET",
                "/tasks/1209000000000011/stories",
                200,
                recorded("stories"),
            ),
        ])
        .await;
        let url = "https://app.asana.com/1/1204000000000001/project/1205000000000002/task/1209000000000011?focus=true";
        let read = Asana
            .call(&chosen(&fixture), "read_task", json!({"task": url}))
            .await
            .unwrap();
        assert_eq!(read["notes"], "Outline in the doc.");
        assert_eq!(
            read["comments"],
            json!([{"author": "Ada Lovelace", "created": "2026-11-04T12:00:00.000Z", "text": "First draft is up."}])
        );
        fixture.finish().await;
        assert_eq!(
            task("https://app.asana.com/0/1205000000000002/1209000000000011/f"),
            Ok("1209000000000011".into())
        );
    }

    #[tokio::test]
    async fn create_task_in_a_project_assigned_to_me() {
        let fixture = Fixture::start(vec![at("POST", "/tasks", 201, recorded("task"))]).await;
        let input = json!({"name": "Draft the launch post", "project": "1205000000000002", "assignee": "me", "due_on": "2026-11-20"});
        let made = Asana
            .call(&chosen(&fixture), "create_task", input)
            .await
            .unwrap();
        assert_eq!(
            made["url"],
            "https://app.asana.com/1/1204000000000001/project/1205000000000002/task/1209000000000011"
        );
        let sent = fixture.finish().await[0].json();
        assert_eq!(
            sent,
            json!({"data": {"name": "Draft the launch post", "projects": ["1205000000000002"], "assignee": "me", "due_on": "2026-11-20"}})
        );
    }

    #[tokio::test]
    async fn update_task_completes_it_and_clears_the_due_date() {
        let fixture = Fixture::start(vec![at(
            "PUT",
            "/tasks/1209000000000011",
            200,
            recorded("task"),
        )])
        .await;
        let input = json!({"task": "1209000000000011", "completed": true, "due_on": "", "assignee": "none"});
        Asana
            .call(&chosen(&fixture), "update_task", input)
            .await
            .unwrap();
        let sent = fixture.finish().await[0].json();
        assert_eq!(
            sent,
            json!({"data": {"completed": true, "assignee": null, "due_on": null}})
        );
    }

    #[tokio::test]
    async fn comment_on_task_adds_a_story() {
        let fixture = Fixture::start(vec![at("POST", "/tasks/1209000000000011/stories", 201, json!({"data": {"gid": "1209000000000099", "resource_type": "story", "created_at": "2026-11-05T10:00:00.000Z", "text": "Done."}}))]).await;
        let made = Asana
            .call(
                &chosen(&fixture),
                "comment_on_task",
                json!({"task": "1209000000000011", "text": "Done."}),
            )
            .await
            .unwrap();
        assert_eq!(made["id"], "1209000000000099");
        assert_eq!(
            fixture.finish().await[0].json(),
            json!({"data": {"text": "Done."}})
        );
    }

    #[tokio::test]
    async fn refusals_map_to_errors_and_bad_input_never_reaches_asana() {
        let help = "For more information on API status codes and how to handle them, read the docs on errors: https://developers.asana.com/docs/errors";
        let cases = [
            (401, "Not Authorized", Error::Unauthorized),
            (
                403,
                "You do not have access to this task.",
                Error::Forbidden("You do not have access to this task.".into()),
            ),
            (
                404,
                "task: Unknown object: 1209000000000011",
                Error::NotFound("task: Unknown object: 1209000000000011".into()),
            ),
            (
                429,
                "You have made too many requests recently.",
                Error::Unavailable(
                    "rate limited: You have made too many requests recently.".into(),
                ),
            ),
            (
                400,
                "due_on: Invalid date",
                Error::Rejected("due_on: Invalid date".into()),
            ),
        ];
        for (status, message, expected) in cases {
            let body = json!({"errors": [{"message": message, "help": help}]});
            let exchange =
                at("PUT", "/tasks/1209000000000011", status, body).header("retry-after", "3600");
            let fixture = Fixture::start(vec![exchange]).await;
            let changed = Asana
                .call(
                    &chosen(&fixture),
                    "update_task",
                    json!({"task": "1209000000000011", "name": "x"}),
                )
                .await;
            assert_eq!(changed, Err(expected));
            fixture.finish().await;
        }
        let fixture = Fixture::start(vec![]).await;
        for input in [
            json!({"task": "../users/me"}),
            json!({"task": "1", "x": 1}),
            json!({"task": "1", "due_on": "next week"}),
        ] {
            let refused = Asana.call(&chosen(&fixture), "update_task", input).await;
            assert!(matches!(refused, Err(Error::Invalid(_))), "{refused:?}");
        }
        let bad = Asana
            .call(
                &cx(&fixture, json!({"workspace": "../x"})),
                "list_my_tasks",
                json!({}),
            )
            .await;
        assert!(matches!(bad, Err(Error::Invalid(_))));
        assert!(fixture.finish().await.is_empty());
    }

    #[tokio::test]
    async fn poll_records_a_baseline_then_reports_newly_assigned_tasks_once() {
        let all = recorded("tasks");
        let older = json!({"data": [all["data"][1]], "next_page": null});
        let fixture = Fixture::start(vec![
            at("GET", "/tasks", 200, older),
            at("GET", "/tasks", 200, all.clone()),
            at("GET", "/tasks", 200, all),
        ])
        .await;
        let cx = chosen(&fixture);
        let baseline = Asana.poll(&cx, None).await.unwrap();
        assert!(baseline.events.is_empty());
        let polled = Asana.poll(&cx, baseline.cursor.as_deref()).await.unwrap();
        assert_eq!(polled.events.len(), 1);
        assert_eq!(polled.events[0].id, "1209000000000011");
        assert_eq!(
            polled.events[0].summary,
            "Assigned to you: Draft the launch post"
        );
        let again = Asana.poll(&cx, polled.cursor.as_deref()).await.unwrap();
        assert!(again.events.is_empty());
        assert!(fixture.finish().await[0].target.contains("assignee=me"));
    }
}

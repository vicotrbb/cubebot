//! Todoist through a personal API token, on the unified API v1
//! (https://developer.todoist.com/api/v1/); REST v2 and Sync v9 are shut down and answer 410.
//! Shapes follow Doist's own client (https://github.com/Doist/todoist-api-python).
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Event, EventSpec, Field, FieldKind,
    Level, Manifest, Poll, Risk, Step, ToolSpec,
};
use reqwest::{Method, RequestBuilder};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const API: &str = "https://api.todoist.com/api/v1";
const TASK_URL: &str = "https://app.todoist.com/app/task";
/// Todoist pages hold at most 200 items.
const LARGEST_PAGE: u8 = 200;
const LONGEST: usize = 4_000;
/// What a poll reads: everything due today or overdue, in the person's own time zone.
const DUE: &str = "today | overdue";

pub struct Todoist;

static MANIFEST: Manifest = Manifest {
    id: "todoist",
    name: "Todoist",
    description: "Read, add, change and complete Todoist tasks, and hear when tasks fall due.",
    category: Category::Work,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "API token",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "0123456789abcdef…",
            help: "From Settings › Integrations › Developer.",
        }],
    }],
    config: &[],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_tasks",
            description: "Open tasks, all or those of a project, or matching a Todoist filter such as \"today | overdue\" or \"#Work & p1\".",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"filter":{"type":"string","description":"A Todoist filter query; not together with project."},"project":{"type":"string","description":"A project id."},"limit":{"type":"integer","minimum":1,"maximum":200},"cursor":{"type":"string","description":"The next value of a previous answer."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_projects",
            description: "The projects in the account, with their ids.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"limit":{"type":"integer","minimum":1,"maximum":200},"cursor":{"type":"string"}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "add_task",
            description: "Add a task, with a due date in plain words such as \"tomorrow at 9\" or \"every friday\".",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"content":{"type":"string","description":"The task's title."},"description":{"type":"string"},"project":{"type":"string","description":"A project id; the Inbox when absent."},"due":{"type":"string","description":"When it is due, in plain words."},"priority":{"type":"integer","minimum":1,"maximum":4,"description":"4 is the most urgent (p1 in the app)."},"labels":{"type":"array","items":{"type":"string"}}},"required":["content"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "update_task",
            description: "Change a task's title, description, due date, priority or labels.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"task":{"type":"string","description":"The task's id or URL."},"content":{"type":"string"},"description":{"type":"string"},"due":{"type":"string","description":"When it is due, in plain words; \"no date\" clears it."},"priority":{"type":"integer","minimum":1,"maximum":4},"labels":{"type":"array","items":{"type":"string"}}},"required":["task"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "complete_task",
            description: "Complete a task; a recurring one moves to its next date instead.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"task":{"type":"string","description":"The task's id or URL."}},"required":["task"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "comment_task",
            description: "Comment on a task. In a shared project every collaborator sees it.",
            risk: Risk::Send,
            product: None,
            input: r#"{"type":"object","properties":{"task":{"type":"string","description":"The task's id or URL."},"text":{"type":"string","description":"Markdown."}},"required":["task","text"],"additionalProperties":false}"#,
        },
    ],
    events: &[EventSpec {
        kind: "task.due",
        description: "A task is due today or overdue; reported once per due date.",
        product: None,
    }],
    level: Level::Write,
    guide: &[Step {
        title: "Copy your API token",
        body: "In Todoist open Settings › Integrations › Developer and copy the API token, then paste it below. It acts as you on every project you can see.",
        link: Some("https://app.todoist.com/app/settings/integrations"),
        copy: None,
    }],
    warning: Some(
        "The token is your whole account and never expires; issuing a new one in Settings › Integrations › Developer revokes this one.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    filter: Option<String>,
    project: Option<String>,
    limit: Option<u8>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    limit: Option<u8>,
    cursor: Option<String>,
}

/// Both a new task and a change to one; `task` names the one to change.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    task: Option<String>,
    content: Option<String>,
    description: Option<String>,
    project: Option<String>,
    due: Option<String>,
    priority: Option<u8>,
    labels: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Task {
    task: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Comment {
    task: String,
    text: String,
}

impl Connector for Todoist {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let me = cx.json(request(cx, Method::GET, "/user")?).await?;
            let name = me["email"]
                .as_str()
                .or(me["full_name"].as_str())
                .unwrap_or("Todoist account");
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "list_tasks" => list_tasks(cx, input).await,
                "list_projects" => list_projects(cx, input).await,
                "add_task" => add_task(cx, input).await,
                "update_task" => update_task(cx, input).await,
                "complete_task" => complete_task(cx, input).await,
                "comment_task" => comment_task(cx, input).await,
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

/// A task's id, alone or at the end of its URL (`…/app/task/buy-milk-6X7rM8997g3RQmvh`).
fn id(text: &str) -> Result<String, Error> {
    let last = match text.strip_prefix(TASK_URL) {
        Some(path) => path.trim_end_matches('/').rsplit(['/', '-']).next(),
        None => Some(text),
    };
    last.filter(|id| (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric()))
        .map(str::to_owned)
        .ok_or_else(|| Error::Invalid(format!("{text:?} is not a Todoist task id or URL")))
}

fn paged(limit: Option<u8>, cursor: Option<String>) -> Vec<(&'static str, String)> {
    let mut query = vec![("limit", limit.unwrap_or(50).min(LARGEST_PAGE).to_string())];
    query.extend(cursor.map(|cursor| ("cursor", cursor)));
    query
}

async fn list_tasks(cx: &Context, input: Value) -> Result<Value, Error> {
    let list: List = crate::input(input)?;
    let mut query = paged(list.limit, list.cursor);
    let path = match (list.filter, list.project) {
        (Some(_), Some(_)) => {
            return Err(Error::Invalid(
                "give a filter or a project, not both; a filter can name a project as #Name".into(),
            ));
        }
        (Some(filter), None) => {
            query.push(("query", filter));
            "/tasks/filter"
        }
        (None, project) => {
            query.extend(project.map(|project| ("project_id", project)));
            "/tasks"
        }
    };
    let found = cx
        .json(request(cx, Method::GET, path)?.query(&query))
        .await?;
    Ok(json!({"tasks": shown(&found, task), "next": found["next_cursor"]}))
}

async fn list_projects(cx: &Context, input: Value) -> Result<Value, Error> {
    let page: Page = crate::input(input)?;
    let query = paged(page.limit, page.cursor);
    let found = cx
        .json(request(cx, Method::GET, "/projects")?.query(&query))
        .await?;
    let project = |project: &Value| {
        json!({
            "id": project["id"],
            "name": project["name"],
            "inbox": project["inbox_project"],
            "shared": project["shared"],
            "parent": project["parent_id"],
        })
    };
    Ok(json!({"projects": shown(&found, project), "next": found["next_cursor"]}))
}

fn body(change: Change) -> Map<String, Value> {
    let fields = [
        ("content", change.content.map(Value::from)),
        ("description", change.description.map(Value::from)),
        ("project_id", change.project.map(Value::from)),
        ("due_string", change.due.map(Value::from)),
        ("priority", change.priority.map(Value::from)),
        ("labels", change.labels.map(Value::from)),
    ];
    fields
        .into_iter()
        .filter_map(|(key, value)| Some((key.to_owned(), value?)))
        .collect()
}

async fn add_task(cx: &Context, input: Value) -> Result<Value, Error> {
    let change: Change = crate::input(input)?;
    if change.task.is_some() || change.content.as_deref().is_none_or(str::is_empty) {
        return Err(Error::Invalid(
            "a new task needs content and no task id".into(),
        ));
    }
    let made = cx
        .json(request(cx, Method::POST, "/tasks")?.json(&body(change)))
        .await?;
    Ok(task(&made))
}

async fn update_task(cx: &Context, input: Value) -> Result<Value, Error> {
    let change: Change = crate::input(input)?;
    let Some(task_id) = change.task.as_deref().map(id).transpose()? else {
        return Err(Error::Invalid("name the task to change".into()));
    };
    if change.project.is_some() {
        return Err(Error::Invalid("a task cannot change project here".into()));
    }
    let path = format!("/tasks/{task_id}");
    let changed = cx
        .json(request(cx, Method::POST, &path)?.json(&body(change)))
        .await?;
    Ok(task(&changed))
}

async fn complete_task(cx: &Context, input: Value) -> Result<Value, Error> {
    let task_id = id(&crate::input::<Task>(input)?.task)?;
    let path = format!("/tasks/{task_id}/close");
    cx.send(request(cx, Method::POST, &path)?).await?;
    Ok(json!({"completed": task_id}))
}

async fn comment_task(cx: &Context, input: Value) -> Result<Value, Error> {
    let comment: Comment = crate::input(input)?;
    let body = json!({"task_id": id(&comment.task)?, "content": comment.text});
    let made = cx
        .json(request(cx, Method::POST, "/comments")?.json(&body))
        .await?;
    Ok(json!({"id": made["id"], "task": made["item_id"], "posted": made["posted_at"]}))
}

/// Tasks due today or overdue. The cursor is the JSON list of `<id>@<due date>` already reported
/// that are still due, so each is reported once per date and a rescheduled one again.
async fn poll(cx: &Context, cursor: Option<&str>) -> Result<Poll, Error> {
    let reported: Vec<String> = cursor
        .and_then(|cursor| serde_json::from_str(cursor).ok())
        .unwrap_or_default();
    // shortcut: one page of 200; a backlog beyond that is not reported until it shrinks.
    let query = [
        ("query", DUE.to_owned()),
        ("limit", LARGEST_PAGE.to_string()),
    ];
    let found = cx
        .json(request(cx, Method::GET, "/tasks/filter")?.query(&query))
        .await?;
    let mut due = Vec::new();
    let mut events = Vec::new();
    for shown in shown(&found, task) {
        let key = format!("{}@{}", text(&shown["id"]), text(&shown["due"]));
        if !reported.contains(&key) {
            let title: String = text(&shown["content"]).chars().take(200).collect();
            events.push(Event {
                kind: "task.due",
                id: key.clone(),
                summary: format!("{title} is due {}", text(&shown["due"])),
                data: shown,
            });
        }
        due.push(key);
    }
    Ok(Poll {
        events: if cursor.is_some() { events } else { Vec::new() },
        cursor: Some(json!(due).to_string()),
    })
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn shown(list: &Value, each: fn(&Value) -> Value) -> Vec<Value> {
    list["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(each)
        .collect()
}

fn task(task: &Value) -> Value {
    let description: String = text(&task["description"]).chars().take(LONGEST).collect();
    json!({
        "id": task["id"],
        "content": task["content"],
        "description": description,
        "project": task["project_id"],
        "due": task["due"]["date"],
        "due_words": task["due"]["string"],
        "recurring": task["due"]["is_recurring"],
        "priority": task["priority"],
        "labels": task["labels"],
        "done": task["checked"],
        "url": format!("{TASK_URL}/{}", text(&task["id"])),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    fn cx(fixture: &Fixture) -> Context {
        let credential = Credential::Fields {
            fields: [("token".into(), "todoist-secret".into())].into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    fn recorded(name: &str) -> Value {
        let path = format!(
            "{}/fixtures/todoist/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn list_tasks_filters_or_lists_a_project_and_returns_the_next_cursor() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api.todoist.com/api/v1/tasks/filter?limit=10&query=today+%7C+p1",
                200,
                recorded("tasks"),
            ),
            Exchange::new(
                "GET",
                "/api.todoist.com/api/v1/tasks?limit=50&cursor=abc&project_id=6Jf8VQXxpwv56VQ7",
                200,
                json!({"results": [], "next_cursor": null}),
            ),
        ])
        .await;
        let cx = cx(&fixture);
        let found = Todoist
            .call(
                &cx,
                "list_tasks",
                json!({"filter": "today | p1", "limit": 10}),
            )
            .await
            .unwrap();
        assert_eq!(found["next"], "eyJsYXN0X2lkIjoiNlg3ck04OTk3ZzNSUW12aCJ9");
        assert_eq!(
            found["tasks"][0],
            json!({
                "id": "6X7rM8997g3RQmvh",
                "content": "Buy milk",
                "description": "Whole milk",
                "project": "6Jf8VQXxpwv56VQ7",
                "due": "2026-11-05",
                "due_words": "every day",
                "recurring": true,
                "priority": 4,
                "labels": ["errands"],
                "done": false,
                "url": "https://app.todoist.com/app/task/6X7rM8997g3RQmvh",
            })
        );
        let project = json!({"project": "6Jf8VQXxpwv56VQ7", "cursor": "abc"});
        let empty = Todoist.call(&cx, "list_tasks", project).await.unwrap();
        assert_eq!(empty["tasks"], json!([]));
        let both = json!({"filter": "today", "project": "6Jf8VQXxpwv56VQ7"});
        let refused = Todoist.call(&cx, "list_tasks", both).await;
        assert!(matches!(refused, Err(Error::Invalid(_))));
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].header("authorization"),
            Some("Bearer todoist-secret")
        );
    }

    #[tokio::test]
    async fn tasks_are_added_changed_completed_and_commented_on() {
        let task = recorded("tasks")["results"][0].clone();
        let fixture = Fixture::start(vec![
            Exchange::new("POST", "/api.todoist.com/api/v1/tasks", 200, task.clone()),
            Exchange::new(
                "POST",
                "/api.todoist.com/api/v1/tasks/6X7rM8997g3RQmvh",
                200,
                task,
            ),
            Exchange::new(
                "POST",
                "/api.todoist.com/api/v1/tasks/6X7rM8997g3RQmvh/close",
                204,
                Value::Null,
            ),
            Exchange::new(
                "POST",
                "/api.todoist.com/api/v1/comments",
                200,
                recorded("comment"),
            ),
        ])
        .await;
        let cx = cx(&fixture);
        let add = json!({"content": "Buy milk", "due": "tomorrow at 9", "priority": 4, "labels": ["errands"]});
        let made = Todoist.call(&cx, "add_task", add).await.unwrap();
        assert_eq!(made["id"], "6X7rM8997g3RQmvh");
        let url = "https://app.todoist.com/app/task/buy-milk-6X7rM8997g3RQmvh";
        let change = json!({"task": url, "due": "no date"});
        Todoist.call(&cx, "update_task", change).await.unwrap();
        let done = Todoist
            .call(&cx, "complete_task", json!({"task": "6X7rM8997g3RQmvh"}))
            .await
            .unwrap();
        assert_eq!(done, json!({"completed": "6X7rM8997g3RQmvh"}));
        let comment = json!({"task": "6X7rM8997g3RQmvh", "text": "Bought."});
        let posted = Todoist.call(&cx, "comment_task", comment).await.unwrap();
        assert_eq!(posted["id"], "6X7rM8xq3xRmwwhP");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].json(),
            json!({"content": "Buy milk", "due_string": "tomorrow at 9", "priority": 4, "labels": ["errands"]})
        );
        assert_eq!(seen[1].json(), json!({"due_string": "no date"}));
        assert_eq!(
            seen[3].json(),
            json!({"task_id": "6X7rM8997g3RQmvh", "content": "Bought."})
        );
    }

    #[tokio::test]
    async fn bad_input_is_refused_before_any_request() {
        let fixture = Fixture::start(Vec::new()).await;
        let cx = cx(&fixture);
        for (tool, input) in [
            ("complete_task", json!({"task": "../projects"})),
            ("complete_task", json!({"task": "6X7r", "force": true})),
            ("add_task", json!({"content": ""})),
            ("update_task", json!({"content": "No task named"})),
        ] {
            let refused = Todoist.call(&cx, tool, input.clone()).await;
            assert!(matches!(refused, Err(Error::Invalid(_))), "{input}");
        }
        assert!(fixture.finish().await.is_empty());
    }

    #[tokio::test]
    async fn refusals_map_to_errors() {
        let path = "/api.todoist.com/api/v1/projects";
        let cases = [
            (401, json!({"error": "Unauthorized"}), Error::Unauthorized),
            (
                403,
                json!({"error": "Forbidden", "error_tag": "FORBIDDEN"}),
                Error::Forbidden("Forbidden".into()),
            ),
            (
                404,
                json!({"error": "Project not found"}),
                Error::NotFound("Project not found".into()),
            ),
            (
                400,
                json!({"error": "Invalid argument value", "error_code": 20, "error_tag": "INVALID_ARGUMENT_VALUE", "http_code": 400}),
                Error::Rejected("Invalid argument value".into()),
            ),
        ];
        for (status, body, expected) in cases {
            let fixture = Fixture::start(vec![Exchange::new("GET", path, status, body)]).await;
            let refused = Todoist
                .call(&cx(&fixture), "list_projects", json!({}))
                .await;
            assert_eq!(refused, Err(expected));
            fixture.finish().await;
        }
        let limited = Exchange::new("GET", path, 429, json!({"error": "Too many requests"}))
            .header("retry-after", "900");
        let fixture = Fixture::start(vec![limited]).await;
        let refused = Todoist
            .call(&cx(&fixture), "list_projects", json!({}))
            .await;
        assert!(matches!(refused, Err(Error::Unavailable(_))));
        fixture.finish().await;
    }

    #[tokio::test]
    async fn poll_records_what_is_due_then_reports_each_task_once_per_due_date() {
        let filter = "/api.todoist.com/api/v1/tasks/filter?query=today+%7C+overdue&limit=200";
        let tasks = recorded("tasks");
        let mut later = tasks.clone();
        later["results"][1]["due"]["date"] = json!("2026-11-05");
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                filter,
                200,
                json!({"results": [tasks["results"][0]]}),
            ),
            Exchange::new("GET", filter, 200, tasks.clone()),
            Exchange::new("GET", filter, 200, tasks),
            Exchange::new("GET", filter, 200, later),
        ])
        .await;
        let cx = cx(&fixture);
        let baseline = Todoist.poll(&cx, None).await.unwrap();
        assert!(baseline.events.is_empty());
        let cursor = baseline.cursor.unwrap();
        assert_eq!(cursor, json!(["6X7rM8997g3RQmvh@2026-11-05"]).to_string());
        let next = Todoist.poll(&cx, Some(&cursor)).await.unwrap();
        assert_eq!(next.events.len(), 1);
        assert_eq!(next.events[0].id, "6X7rPPJ6pmxWHmvr@2026-11-03");
        assert_eq!(next.events[0].summary, "Send the invoice is due 2026-11-03");
        let cursor = next.cursor.unwrap();
        let again = Todoist.poll(&cx, Some(&cursor)).await.unwrap();
        assert!(again.events.is_empty(), "repeats are not reported");
        let moved = Todoist.poll(&cx, Some(&cursor)).await.unwrap();
        assert_eq!(moved.events[0].id, "6X7rPPJ6pmxWHmvr@2026-11-05");
        fixture.finish().await;
    }
}

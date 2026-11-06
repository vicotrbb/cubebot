//! Linear through a personal API key, over its GraphQL API.
//! Docs: https://linear.app/developers/graphql (a personal key goes in `Authorization` without
//! `Bearer`), https://linear.app/developers/filtering, https://linear.app/developers/pagination,
//! https://linear.app/developers/rate-limiting, and notification types in
//! https://github.com/linear/linear/blob/master/packages/codegen-doc/src/constants.ts.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Event, EventSpec, Field, FieldKind,
    Level, Manifest, Poll, Risk, Step, ToolSpec,
};
use serde::Deserialize;
use serde_json::{Value, json};

const API: &str = "https://api.linear.app/graphql";
/// The most issues one search answers; Linear's connections default to 50.
const PAGE: u8 = 50;
/// A description or comment is cut to this many characters.
const TEXT: usize = 20_000;
const COMMENT: usize = 2_000;
const COMMENTS: u8 = 50;
/// Notification ids already reported, so a repeat is recognised.
const REMEMBERED: usize = 200;

pub struct Linear;

static MANIFEST: Manifest = Manifest {
    id: "linear",
    name: "Linear",
    description: "Find, read, create and update Linear issues and comment on them.",
    category: Category::Work,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "Personal API key",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "lin_api_…",
            help: "From Settings › Account › Security & access.",
        }],
    }],
    config: &[],
    products: &[],
    tools: &[
        ToolSpec {
            name: "search_issues",
            description: "Find issues by text, or list them filtered by assignee, team and state, most recently updated first.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"query":{"type":"string","description":"Words to search titles and descriptions for."},"assigned_to_me":{"type":"boolean"},"team":{"type":"string","description":"A team key, such as ENG."},"state":{"type":"string","description":"A workflow state name, such as In Progress."},"limit":{"type":"integer","minimum":1,"maximum":50},"after":{"type":"string","description":"The next value of a previous search, for its next page."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_issue",
            description: "An issue's details, description and comments.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"issue":{"type":"string","description":"The issue's identifier such as ENG-123, or its URL."}},"required":["issue"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "create_issue",
            description: "Create an issue in a team, optionally assigned and prioritised.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"team":{"type":"string","description":"The team's key, such as ENG."},"title":{"type":"string"},"description":{"type":"string","description":"Markdown."},"assignee":{"type":"string","description":"me, or a user's email address."},"priority":{"type":"integer","minimum":0,"maximum":4,"description":"0 none, 1 urgent, 2 high, 3 medium, 4 low."}},"required":["team","title"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "update_issue",
            description: "Change an issue's title, description, state, assignee or priority.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"issue":{"type":"string","description":"The issue's identifier or URL."},"title":{"type":"string"},"description":{"type":"string","description":"Markdown; replaces the description."},"state":{"type":"string","description":"A workflow state name of the issue's team, such as Done."},"assignee":{"type":"string","description":"me, a user's email address, or none to unassign."},"priority":{"type":"integer","minimum":0,"maximum":4}},"required":["issue"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "comment_on_issue",
            description: "Post a comment on an issue; its subscribers are notified.",
            risk: Risk::Send,
            product: None,
            input: r#"{"type":"object","properties":{"issue":{"type":"string","description":"The issue's identifier or URL."},"body":{"type":"string","description":"Markdown."}},"required":["issue","body"],"additionalProperties":false}"#,
        },
    ],
    events: &[
        EventSpec {
            kind: "issue.assigned",
            description: "An issue was assigned to the key's owner.",
            product: None,
        },
        EventSpec {
            kind: "comment.mentioned",
            description: "A comment mentioned the key's owner.",
            product: None,
        },
    ],
    level: Level::Full,
    guide: &[
        Step {
            title: "Create a personal API key",
            body: "In Linear open Settings › Account › Security & access, and under Personal API keys choose New API key. Name it Cubebot. If asked for permissions, allow reading and writing (create issues and comments); limit it to teams only if agents should see no others.",
            link: Some("https://linear.app/settings/account/security"),
            copy: None,
        },
        Step {
            title: "Paste it below",
            body: "Linear shows the key once. Copy it and paste it here. It acts as you: issues and comments agents create carry your name.",
            link: None,
            copy: None,
        },
    ],
    warning: Some(
        "The key acts as you in every team it can see. Workspace admins can turn off personal API keys for members. Linear allows 2,500 requests an hour per user, shared by all of that user's keys.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: Option<String>,
    #[serde(default)]
    assigned_to_me: bool,
    team: Option<String>,
    state: Option<String>,
    limit: Option<u8>,
    after: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    issue: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    team: String,
    title: String,
    description: Option<String>,
    assignee: Option<String>,
    priority: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    issue: String,
    title: Option<String>,
    description: Option<String>,
    state: Option<String>,
    assignee: Option<String>,
    priority: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Comment {
    issue: String,
    body: String,
}

impl Connector for Linear {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let data = graphql(cx, "query { viewer { name email } }", json!({})).await?;
            let viewer = &data["viewer"];
            let name = viewer["email"]
                .as_str()
                .or(viewer["name"].as_str())
                .unwrap_or("Linear account");
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "search_issues" => search(cx, input).await,
                "read_issue" => read(cx, input).await,
                "create_issue" => create(cx, input).await,
                "update_issue" => update(cx, input).await,
                "comment_on_issue" => comment(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(poll(cx, cursor))
    }
}

/// One GraphQL request. Linear answers 200 with `errors` for many refusals, so those map here.
async fn graphql(cx: &Context, query: &str, variables: Value) -> Result<Value, Error> {
    let request = cx
        .post(API)?
        .header(reqwest::header::AUTHORIZATION, cx.token()?)
        .json(&json!({"query": query, "variables": variables}));
    let mut answer = cx.json(request).await?;
    if let Some(error) = answer["errors"]
        .as_array()
        .and_then(|errors| errors.first())
    {
        return Err(refusal(error));
    }
    Ok(answer["data"].take())
}

fn refusal(error: &Value) -> Error {
    let extensions = &error["extensions"];
    let message = extensions["userPresentableMessage"]
        .as_str()
        .or(error["message"].as_str())
        .unwrap_or("no reason given");
    let message: String = message.chars().take(300).collect();
    match extensions["code"].as_str().unwrap_or_default() {
        "AUTHENTICATION_ERROR" => Error::Unauthorized,
        "FORBIDDEN" => Error::Forbidden(message),
        "RATELIMITED" => Error::Unavailable(format!("rate limited: {message}")),
        _ if error["message"]
            .as_str()
            .is_some_and(|text| text.starts_with("Entity not found")) =>
        {
            Error::NotFound(message)
        }
        _ => Error::Rejected(message),
    }
}

/// An issue's identifier, such as `ENG-123`, from itself or its URL.
fn issue(text: &str) -> Result<String, Error> {
    let text = text.trim();
    let found = match text.split_once("/issue/") {
        Some((_, rest)) => rest.split(['/', '?', '#']).next().unwrap_or_default(),
        None => text,
    };
    let valid = !found.is_empty()
        && found.len() <= 64
        && found.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    valid
        .then(|| found.to_owned())
        .ok_or_else(|| Error::Invalid(format!("{text:?} is not a Linear issue identifier or URL")))
}

const SUMMARY: &str =
    "identifier title url priorityLabel updatedAt state { name } assignee { name } team { key }";

async fn search(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: Search = crate::input(input)?;
    let mut filter = json!({});
    if search.assigned_to_me {
        filter["assignee"] = json!({"isMe": {"eq": true}});
    }
    if let Some(team) = search.team {
        filter["team"] = json!({"key": {"eqIgnoreCase": team}});
    }
    if let Some(state) = search.state {
        filter["state"] = json!({"name": {"eqIgnoreCase": state}});
    }
    let first = search.limit.unwrap_or(25).clamp(1, PAGE);
    let variables =
        json!({"filter": filter, "first": first, "after": search.after, "term": search.query});
    let (field, query) = match search.query {
        Some(_) => (
            "searchIssues",
            format!(
                "query($term: String!, $filter: IssueFilter, $first: Int, $after: String) {{ searchIssues(term: $term, filter: $filter, first: $first, after: $after) {{ nodes {{ {SUMMARY} }} pageInfo {{ hasNextPage endCursor }} }} }}"
            ),
        ),
        None => (
            "issues",
            format!(
                "query($filter: IssueFilter, $first: Int, $after: String) {{ issues(filter: $filter, first: $first, after: $after, orderBy: updatedAt) {{ nodes {{ {SUMMARY} }} pageInfo {{ hasNextPage endCursor }} }} }}"
            ),
        ),
    };
    let data = graphql(cx, &query, variables).await?;
    let found = &data[field];
    let issues: Vec<Value> = found["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(summary)
        .collect();
    let next = match found["pageInfo"]["hasNextPage"].as_bool() {
        Some(true) => found["pageInfo"]["endCursor"].clone(),
        _ => Value::Null,
    };
    Ok(json!({"issues": issues, "next": next}))
}

fn summary(issue: &Value) -> Value {
    json!({
        "id": issue["identifier"],
        "title": issue["title"],
        "state": issue["state"]["name"],
        "assignee": issue["assignee"]["name"],
        "priority": issue["priorityLabel"],
        "team": issue["team"]["key"],
        "url": issue["url"],
        "updated": issue["updatedAt"],
    })
}

async fn read(cx: &Context, input: Value) -> Result<Value, Error> {
    let id = issue(&crate::input::<Read>(input)?.issue)?;
    let query = format!(
        "query($id: String!, $comments: Int) {{ issue(id: $id) {{ {SUMMARY} description createdAt comments(first: $comments) {{ nodes {{ body createdAt user {{ name }} }} }} }} }}"
    );
    let data = graphql(cx, &query, json!({"id": id, "comments": COMMENTS})).await?;
    let found = &data["issue"];
    let mut shown = summary(found);
    shown["created"] = found["createdAt"].clone();
    shown["description"] = json!(cut(found["description"].as_str().unwrap_or_default(), TEXT));
    shown["comments"] = found["comments"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|comment| {
            json!({
                "author": comment["user"]["name"],
                "created": comment["createdAt"],
                "body": cut(comment["body"].as_str().unwrap_or_default(), COMMENT),
            })
        })
        .collect();
    Ok(shown)
}

fn cut(text: &str, longest: usize) -> String {
    match text.char_indices().nth(longest) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

async fn create(cx: &Context, input: Value) -> Result<Value, Error> {
    let create: Create = crate::input(input)?;
    let teams = graphql(
        cx,
        "query($key: String!) { teams(filter: {key: {eqIgnoreCase: $key}}) { nodes { id } } }",
        json!({"key": create.team}),
    )
    .await?;
    let team = teams["teams"]["nodes"][0]["id"]
        .as_str()
        .ok_or_else(|| Error::NotFound(format!("no team with the key {}", create.team)))?;
    let mut fields = json!({"teamId": team, "title": create.title});
    if let Some(description) = create.description {
        fields["description"] = json!(description);
    }
    if let Some(priority) = create.priority {
        fields["priority"] = json!(priority);
    }
    if let Some(assignee) = create.assignee {
        fields["assigneeId"] = user(cx, &assignee).await?;
    }
    let query = format!(
        "mutation($input: IssueCreateInput!) {{ issueCreate(input: $input) {{ success issue {{ {SUMMARY} }} }} }}"
    );
    let data = graphql(cx, &query, json!({"input": fields})).await?;
    Ok(summary(&data["issueCreate"]["issue"]))
}

/// A user's id from `me`, an email address, or `none` for nobody.
async fn user(cx: &Context, who: &str) -> Result<Value, Error> {
    match who.trim() {
        "none" => Ok(Value::Null),
        "me" => {
            let data = graphql(cx, "query { viewer { id } }", json!({})).await?;
            Ok(data["viewer"]["id"].clone())
        }
        email => {
            let data = graphql(
                cx,
                "query($email: String!) { users(filter: {email: {eqIgnoreCase: $email}}) { nodes { id } } }",
                json!({"email": email}),
            )
            .await?;
            let id = &data["users"]["nodes"][0]["id"];
            if id.is_null() {
                return Err(Error::NotFound(format!("no user with the email {email}")));
            }
            Ok(id.clone())
        }
    }
}

async fn update(cx: &Context, input: Value) -> Result<Value, Error> {
    let update: Update = crate::input(input)?;
    let id = issue(&update.issue)?;
    let mut fields = json!({});
    if let Some(title) = update.title {
        fields["title"] = json!(title);
    }
    if let Some(description) = update.description {
        fields["description"] = json!(description);
    }
    if let Some(priority) = update.priority {
        fields["priority"] = json!(priority);
    }
    if let Some(assignee) = update.assignee {
        fields["assigneeId"] = user(cx, &assignee).await?;
    }
    if let Some(state) = update.state {
        fields["stateId"] = json!(state_id(cx, &id, &state).await?);
    }
    let query = format!(
        "mutation($id: String!, $input: IssueUpdateInput!) {{ issueUpdate(id: $id, input: $input) {{ success issue {{ {SUMMARY} }} }} }}"
    );
    let data = graphql(cx, &query, json!({"id": id, "input": fields})).await?;
    Ok(summary(&data["issueUpdate"]["issue"]))
}

/// The id of the issue's team's workflow state with this name.
async fn state_id(cx: &Context, issue: &str, name: &str) -> Result<String, Error> {
    let data = graphql(
        cx,
        "query($id: String!, $name: String!) { issue(id: $id) { team { states(filter: {name: {eqIgnoreCase: $name}}) { nodes { id } } } } }",
        json!({"id": issue, "name": name}),
    )
    .await?;
    data["issue"]["team"]["states"]["nodes"][0]["id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::NotFound(format!("the issue's team has no state named {name}")))
}

async fn comment(cx: &Context, input: Value) -> Result<Value, Error> {
    let comment: Comment = crate::input(input)?;
    let id = issue(&comment.issue)?;
    let data = graphql(
        cx,
        "mutation($input: CommentCreateInput!) { commentCreate(input: $input) { success comment { id url createdAt } } }",
        json!({"input": {"issueId": id, "body": comment.body}}),
    )
    .await?;
    let made = &data["commentCreate"]["comment"];
    Ok(json!({"id": made["id"], "url": made["url"], "created": made["createdAt"]}))
}

/// Assignments and comment mentions from the inbox of the last day. Linear does not document the
/// order of a connection, so the cursor is the JSON list of ids already seen rather than a time.
async fn poll(cx: &Context, cursor: Option<&str>) -> Result<Poll, Error> {
    let mut seen: Vec<String> = cursor
        .and_then(|cursor| serde_json::from_str(cursor).ok())
        .unwrap_or_default();
    // shortcut: one page of 50 from the last day; more in one day could be missed, page if that bites.
    let data = graphql(
        cx,
        "query { notifications(first: 50, filter: {type: {in: [\"issueAssignedToYou\", \"issueCommentMention\"]}, createdAt: {gt: \"-P1D\"}}) { nodes { id type createdAt actor { name } ... on IssueNotification { issue { identifier title url } comment { body url } } } } }",
        json!({}),
    )
    .await?;
    let mut events = Vec::new();
    for found in data["notifications"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let Some(event) = event(found) else { continue };
        if seen.contains(&event.id) {
            continue;
        }
        seen.push(event.id.clone());
        events.push(event);
    }
    let excess = seen.len().saturating_sub(REMEMBERED);
    seen.drain(..excess);
    Ok(Poll {
        events: if cursor.is_some() { events } else { Vec::new() },
        cursor: Some(json!(seen).to_string()),
    })
}

fn event(notification: &Value) -> Option<Event> {
    let issue = &notification["issue"];
    let actor = notification["actor"]["name"].as_str().unwrap_or("Someone");
    let name = issue["identifier"].as_str().unwrap_or("an issue");
    let title = issue["title"].as_str().unwrap_or_default();
    let (kind, summary, data) = match notification["type"].as_str()? {
        "issueAssignedToYou" => (
            "issue.assigned",
            format!("{actor} assigned you {name}: {title}"),
            summary_of(issue, None),
        ),
        "issueCommentMention" => (
            "comment.mentioned",
            format!("{actor} mentioned you on {name}: {title}"),
            summary_of(issue, Some(&notification["comment"])),
        ),
        _ => return None,
    };
    Some(Event {
        kind,
        id: notification["id"].as_str()?.to_owned(),
        summary,
        data,
    })
}

fn summary_of(issue: &Value, comment: Option<&Value>) -> Value {
    let mut data =
        json!({"issue": issue["identifier"], "title": issue["title"], "url": issue["url"]});
    if let Some(comment) = comment {
        data["comment"] = json!(cut(comment["body"].as_str().unwrap_or_default(), COMMENT));
        data["comment_url"] = comment["url"].clone();
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    const PATH: &str = "/api.linear.app/graphql";

    fn cx(fixture: &Fixture) -> Context {
        let credential = Credential::Fields {
            fields: [("token".into(), "lin_api_secret".into())].into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    fn recorded(name: &str) -> Value {
        let path = format!("{}/fixtures/linear/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn answer(body: Value) -> Exchange {
        Exchange::new("POST", PATH, 200, body)
    }

    #[tokio::test]
    async fn search_filters_for_me_by_team_and_state_and_pages() {
        let fixture = Fixture::start(vec![answer(recorded("issues"))]).await;
        let input =
            json!({"assigned_to_me": true, "team": "ENG", "state": "In Progress", "limit": 2});
        let found = Linear
            .call(&cx(&fixture), "search_issues", input)
            .await
            .unwrap();
        assert_eq!(found["issues"][0]["id"], "ENG-123");
        assert_eq!(found["issues"][0]["state"], "In Progress");
        assert_eq!(found["issues"][0]["assignee"], "Victor Bona");
        assert_eq!(found["next"], "c2f1a7e0-cursor");
        let seen = fixture.finish().await;
        assert_eq!(seen[0].header("authorization"), Some("lin_api_secret"));
        let sent = seen[0].json();
        assert!(sent["query"].as_str().unwrap().contains("issues(filter:"));
        assert_eq!(
            sent["variables"]["filter"],
            json!({"assignee": {"isMe": {"eq": true}}, "team": {"key": {"eqIgnoreCase": "ENG"}}, "state": {"name": {"eqIgnoreCase": "In Progress"}}})
        );
        assert_eq!(sent["variables"]["first"], 2);
    }

    #[tokio::test]
    async fn a_text_search_uses_linear_search_and_answers_no_next_on_the_last_page() {
        let mut body = recorded("issues");
        let found = body["data"]["issues"].take();
        body["data"] = json!({"searchIssues": found});
        body["data"]["searchIssues"]["pageInfo"]["hasNextPage"] = json!(false);
        let fixture = Fixture::start(vec![answer(body)]).await;
        let found = Linear
            .call(&cx(&fixture), "search_issues", json!({"query": "login"}))
            .await
            .unwrap();
        assert_eq!(found["issues"].as_array().unwrap().len(), 2);
        assert_eq!(found["next"], Value::Null);
        let sent = fixture.finish().await[0].json();
        assert!(
            sent["query"]
                .as_str()
                .unwrap()
                .contains("searchIssues(term:")
        );
        assert_eq!(sent["variables"]["term"], "login");
    }

    #[tokio::test]
    async fn read_issue_takes_a_url_and_shows_description_and_comments() {
        let fixture = Fixture::start(vec![answer(recorded("issue"))]).await;
        let url = "https://linear.app/acme/issue/ENG-123/login-fails-on-safari";
        let read = Linear
            .call(&cx(&fixture), "read_issue", json!({"issue": url}))
            .await
            .unwrap();
        assert_eq!(read["id"], "ENG-123");
        assert_eq!(read["description"], "Steps:\n1. Open Safari\n2. Sign in");
        assert_eq!(read["comments"][0]["author"], "Ada Lovelace");
        assert_eq!(read["comments"][0]["body"], "Reproduced on 17.4.");
        let sent = fixture.finish().await[0].json();
        assert_eq!(sent["variables"]["id"], "ENG-123");
    }

    #[tokio::test]
    async fn create_issue_resolves_the_team_and_assignee() {
        let fixture = Fixture::start(vec![
            answer(json!({"data": {"teams": {"nodes": [{"id": "team-uuid"}]}}})),
            answer(json!({"data": {"viewer": {"id": "me-uuid"}}})),
            answer(json!({"data": {"issueCreate": {"success": true, "issue": recorded("issues")["data"]["issues"]["nodes"][0]}}})),
        ])
        .await;
        let input = json!({"team": "eng", "title": "Login fails on Safari", "assignee": "me", "priority": 2});
        let made = Linear
            .call(&cx(&fixture), "create_issue", input)
            .await
            .unwrap();
        assert_eq!(
            made["url"],
            "https://linear.app/acme/issue/ENG-123/login-fails-on-safari"
        );
        let seen = fixture.finish().await;
        assert_eq!(seen[0].json()["variables"]["key"], "eng");
        assert_eq!(
            seen[2].json()["variables"]["input"],
            json!({"teamId": "team-uuid", "title": "Login fails on Safari", "priority": 2, "assigneeId": "me-uuid"})
        );
    }

    #[tokio::test]
    async fn update_issue_moves_it_to_a_named_state_and_unassigns() {
        let fixture = Fixture::start(vec![
            answer(json!({"data": {"issue": {"team": {"states": {"nodes": [{"id": "done-uuid"}]}}}}})),
            answer(json!({"data": {"issueUpdate": {"success": true, "issue": recorded("issues")["data"]["issues"]["nodes"][0]}}})),
        ])
        .await;
        let input = json!({"issue": "ENG-123", "state": "Done", "assignee": "none"});
        let changed = Linear
            .call(&cx(&fixture), "update_issue", input)
            .await
            .unwrap();
        assert_eq!(changed["id"], "ENG-123");
        let seen = fixture.finish().await;
        assert_eq!(seen[0].json()["variables"]["name"], "Done");
        assert_eq!(
            seen[1].json()["variables"],
            json!({"id": "ENG-123", "input": {"assigneeId": null, "stateId": "done-uuid"}})
        );
    }

    #[tokio::test]
    async fn comment_on_issue_posts_the_body() {
        let fixture = Fixture::start(vec![answer(json!({"data": {"commentCreate": {"success": true, "comment": {"id": "c1", "url": "https://linear.app/acme/issue/ENG-123#comment-c1", "createdAt": "2026-11-05T10:00:00.000Z"}}}}))]).await;
        let input = json!({"issue": "ENG-123", "body": "Fixed in #42."});
        let made = Linear
            .call(&cx(&fixture), "comment_on_issue", input)
            .await
            .unwrap();
        assert_eq!(made["id"], "c1");
        let sent = fixture.finish().await[0].json();
        assert_eq!(
            sent["variables"]["input"],
            json!({"issueId": "ENG-123", "body": "Fixed in #42."})
        );
    }

    #[tokio::test]
    async fn refusals_map_to_errors_and_bad_input_never_reaches_linear() {
        let auth = json!({"errors": [{"message": "Authentication required, not authenticated", "extensions": {"type": "authentication error", "code": "AUTHENTICATION_ERROR", "statusCode": 401, "userPresentableMessage": "You need to authenticate to access this operation."}}]});
        let missing = json!({"data": null, "errors": [{"message": "Entity not found: Issue", "extensions": {"type": "invalid input", "code": "INVALID_INPUT", "userPresentableMessage": "Could not find referenced Issue."}}]});
        let cases = [
            (Exchange::new("POST", PATH, 401, auth), Error::Unauthorized),
            (
                Exchange::new("POST", PATH, 403, json!({"errors": [{"message": "Forbidden"}]})),
                Error::Forbidden("Forbidden".into()),
            ),
            (
                Exchange::new("POST", PATH, 404, json!({})),
                Error::NotFound("{}".into()),
            ),
            (
                Exchange::new("POST", PATH, 429, json!({"errors": [{"message": "Rate limit exceeded", "extensions": {"code": "RATELIMITED"}}]}))
                    .header("retry-after", "3600"),
                Error::Unavailable("rate limited: Rate limit exceeded".into()),
            ),
            (
                answer(missing),
                Error::NotFound("Could not find referenced Issue.".into()),
            ),
        ];
        for (exchange, expected) in cases {
            let fixture = Fixture::start(vec![exchange]).await;
            let read = Linear
                .call(&cx(&fixture), "read_issue", json!({"issue": "ENG-9"}))
                .await;
            assert_eq!(read, Err(expected));
            fixture.finish().await;
        }
        let fixture = Fixture::start(vec![]).await;
        for input in [
            json!({"issue": "ENG 1; drop"}),
            json!({"issue": "ENG-1", "x": 1}),
        ] {
            let refused = Linear.call(&cx(&fixture), "read_issue", input).await;
            assert!(matches!(refused, Err(Error::Invalid(_))), "{refused:?}");
        }
        assert!(fixture.finish().await.is_empty());
    }

    #[tokio::test]
    async fn poll_records_a_baseline_then_reports_new_assignments_and_mentions_once() {
        let mut later = recorded("notifications");
        let older = later["data"]["notifications"]["nodes"][1].clone();
        let fixture = Fixture::start(vec![
            answer(json!({"data": {"notifications": {"nodes": [older]}}})),
            answer(later.clone()),
            answer(later.take()),
        ])
        .await;
        let cx = cx(&fixture);
        let baseline = Linear.poll(&cx, None).await.unwrap();
        assert!(baseline.events.is_empty());
        let cursor = baseline.cursor.unwrap();
        let polled = Linear.poll(&cx, Some(&cursor)).await.unwrap();
        assert_eq!(polled.events.len(), 1);
        let event = &polled.events[0];
        assert_eq!(event.kind, "comment.mentioned");
        assert_eq!(
            event.summary,
            "Ada Lovelace mentioned you on ENG-123: Login fails on Safari"
        );
        assert_eq!(event.data["comment"], "@victor can you take a look?");
        let again = Linear.poll(&cx, polled.cursor.as_deref()).await.unwrap();
        assert!(again.events.is_empty());
        let sent = fixture.finish().await[0].json();
        assert!(
            sent["query"]
                .as_str()
                .unwrap()
                .contains("issueAssignedToYou")
        );
    }
}

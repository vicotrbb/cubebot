//! Jira and Confluence on one Atlassian Cloud site, with the person's email and an API token
//! sent as Basic authentication to the site itself.
//! Docs: https://support.atlassian.com/atlassian-account/docs/manage-api-tokens-for-your-atlassian-account/
//! (unscoped tokens use Basic against the site URL; scoped ones only work through api.atlassian.com),
//! https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/ (the old
//! `/rest/api/3/search` is removed in favour of `/rest/api/3/search/jql`),
//! https://developer.atlassian.com/cloud/jira/platform/basic-auth-for-rest-apis/,
//! https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/ and
//! https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-search/ (CQL search).
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Event, EventSpec, Field, FieldKind,
    Level, Manifest, Poll, Product, Risk, Step, ToolSpec,
};
use reqwest::{Method, RequestBuilder, Url};
use serde::Deserialize;
use serde_json::{Value, json};

const FIELDS: &str = "summary,status,assignee,priority,issuetype,updated";
/// A description, page or comment is cut to this many characters.
const TEXT: usize = 20_000;
const COMMENT: usize = 2_000;
const COMMENTS: usize = 20;
const MOST: u8 = 50;
/// Open issues assigned to the person, most recently changed first: a new assignment is a change.
const ASSIGNED: &str = "assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC";
/// Issue keys already reported, so a repeat is recognised.
const REMEMBERED: usize = 200;

pub struct Atlassian;

static MANIFEST: Manifest = Manifest {
    id: "atlassian",
    name: "Jira and Confluence",
    description: "Search, read and update Jira issues and Confluence pages on an Atlassian Cloud site.",
    category: Category::Work,
    auth: &[Auth::Password {
        fields: &[
            Field {
                key: "email",
                label: "Atlassian account email",
                kind: FieldKind::Text,
                required: true,
                placeholder: "you@company.com",
                help: "The email you sign in to Atlassian with.",
            },
            Field {
                key: "token",
                label: "API token",
                kind: FieldKind::Secret,
                required: true,
                placeholder: "ATATT…",
                help: "An API token without scopes, from your Atlassian account's security settings.",
            },
        ],
    }],
    config: &[Field {
        key: "site",
        label: "Site address",
        kind: FieldKind::Url,
        required: true,
        placeholder: "https://your-company.atlassian.net",
        help: "The address you open Jira or Confluence at.",
    }],
    products: &[
        Product {
            id: "jira",
            name: "Jira",
        },
        Product {
            id: "confluence",
            name: "Confluence",
        },
    ],
    tools: &[
        ToolSpec {
            name: "search_issues",
            description: "Find Jira issues with a JQL query, such as assignee = currentUser() AND statusCategory != Done.",
            risk: Risk::Read,
            product: Some("jira"),
            input: r#"{"type":"object","properties":{"jql":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":50},"next":{"type":"string","description":"The next value of a previous search, for its next page."}},"required":["jql"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_issue",
            description: "A Jira issue's fields, description as text, and its latest comments.",
            risk: Risk::Read,
            product: Some("jira"),
            input: r#"{"type":"object","properties":{"issue":{"type":"string","description":"The issue key such as PROJ-123, or its URL."}},"required":["issue"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "create_issue",
            description: "Create a Jira issue in a project.",
            risk: Risk::Write,
            product: Some("jira"),
            input: r#"{"type":"object","properties":{"project":{"type":"string","description":"The project key, such as PROJ."},"summary":{"type":"string"},"type":{"type":"string","description":"The issue type's name; Task when absent."},"description":{"type":"string"},"assignee":{"type":"string","description":"me, or an Atlassian account id."}},"required":["project","summary"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "update_issue",
            description: "Change a Jira issue's summary, description or assignee, or move it to another status.",
            risk: Risk::Write,
            product: Some("jira"),
            input: r#"{"type":"object","properties":{"issue":{"type":"string","description":"The issue key or URL."},"summary":{"type":"string"},"description":{"type":"string","description":"Replaces the description."},"assignee":{"type":"string","description":"me, an Atlassian account id, or none to unassign."},"status":{"type":"string","description":"The status or transition name to move it to, such as Done."}},"required":["issue"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "comment_on_issue",
            description: "Add a comment to a Jira issue; its watchers are notified.",
            risk: Risk::Send,
            product: Some("jira"),
            input: r#"{"type":"object","properties":{"issue":{"type":"string","description":"The issue key or URL."},"text":{"type":"string"}},"required":["issue","text"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "search_pages",
            description: "Find Confluence content with a CQL query, such as type = page AND text ~ \"roadmap\".",
            risk: Risk::Read,
            product: Some("confluence"),
            input: r#"{"type":"object","properties":{"cql":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":50},"next":{"type":"string","description":"The next value of a previous search, for its next page."}},"required":["cql"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_page",
            description: "A Confluence page's title and its body as text.",
            risk: Risk::Read,
            product: Some("confluence"),
            input: r#"{"type":"object","properties":{"page":{"type":"string","description":"The page's id or URL."}},"required":["page"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "create_page",
            description: "Create a Confluence page in a space, optionally under a parent page.",
            risk: Risk::Write,
            product: Some("confluence"),
            input: r#"{"type":"object","properties":{"space":{"type":"string","description":"The space key, such as ENG."},"title":{"type":"string"},"text":{"type":"string","description":"Body text; each blank-line-separated block becomes a paragraph."},"parent":{"type":"string","description":"The parent page's id."}},"required":["space","title"],"additionalProperties":false}"#,
        },
    ],
    events: &[EventSpec {
        kind: "issue.assigned",
        description: "An open Jira issue was assigned to the person.",
        product: Some("jira"),
    }],
    level: Level::Write,
    guide: &[
        Step {
            title: "Create an API token",
            body: "Open your Atlassian account's API tokens page and choose Create API token — not the one with scopes, which only works through Atlassian's gateway. Name it Cubebot and pick an expiry date (at most a year).",
            link: Some("https://id.atlassian.com/manage-profile/security/api-tokens"),
            copy: None,
        },
        Step {
            title: "Copy it",
            body: "Atlassian shows the token once. Paste it below with the email address of the same account.",
            link: None,
            copy: None,
        },
        Step {
            title: "Enter your site",
            body: "The site address is the start of any Jira or Confluence page you open, such as https://your-company.atlassian.net. Switch off Jira or Confluence below if the site has only one of them.",
            link: None,
            copy: None,
        },
    ],
    warning: Some(
        "API tokens expire, after a year at most, and the connection stops working then: create a new token and sign in again. The token acts as you, with all your access on the site. Tokens with scopes do not work here.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    jql: String,
    limit: Option<u8>,
    next: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Issue {
    issue: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    project: String,
    summary: String,
    #[serde(rename = "type")]
    kind: Option<String>,
    description: Option<String>,
    assignee: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    issue: String,
    summary: Option<String>,
    description: Option<String>,
    assignee: Option<String>,
    status: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Comment {
    issue: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Find {
    cql: String,
    limit: Option<u8>,
    next: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    page: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewPage {
    space: String,
    title: String,
    #[serde(default)]
    text: String,
    parent: Option<String>,
}

impl Connector for Atlassian {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let jira = MANIFEST.enabled(cx.config()).contains(&"jira");
            let path = match jira {
                true => "/rest/api/3/myself",
                false => "/wiki/rest/api/user/current",
            };
            let me = cx.json(request(cx, Method::GET, path)?).await?;
            let name = me["emailAddress"]
                .as_str()
                .or(me["email"].as_str())
                .or(me["displayName"].as_str())
                .unwrap_or("Atlassian account");
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "search_issues" => search_issues(cx, input).await,
                "read_issue" => read_issue(cx, input).await,
                "create_issue" => create_issue(cx, input).await,
                "update_issue" => update_issue(cx, input).await,
                "comment_on_issue" => comment(cx, input).await,
                "search_pages" => search_pages(cx, input).await,
                "read_page" => read_page(cx, input).await,
                "create_page" => create_page(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(poll(cx, cursor))
    }
}

/// The site's origin. Only an https `*.atlassian.net` address is accepted, so the credential
/// never goes anywhere else.
fn site(cx: &Context) -> Result<String, Error> {
    let typed = cx
        .setting("site")
        .ok_or_else(|| Error::Invalid("the connection has no site setting".into()))?;
    let url = Url::parse(typed.trim()).ok();
    let host = url
        .as_ref()
        .filter(|url| url.scheme() == "https" && url.port().is_none())
        .and_then(Url::host_str)
        .filter(|host| {
            host.strip_suffix(".atlassian.net").is_some_and(|name| {
                !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
        })
        .ok_or_else(|| {
            Error::Invalid("the site must be an https://<name>.atlassian.net address".into())
        })?;
    Ok(format!("https://{host}"))
}

fn request(cx: &Context, method: Method, path: &str) -> Result<RequestBuilder, Error> {
    let email = cx.setting("email").ok_or(Error::Unauthorized)?;
    Ok(cx
        .request(method, &format!("{}{path}", site(cx)?))?
        .basic_auth(email, Some(cx.secret("token")?)))
}

/// An issue key such as `PROJ-123`, or a numeric id, from itself or its `/browse/` URL.
fn key(text: &str) -> Result<String, Error> {
    let text = text.trim();
    let found = match text.split_once("/browse/") {
        Some((_, rest)) => rest.split(['/', '?', '#']).next().unwrap_or_default(),
        None => text,
    };
    let valid = !found.is_empty()
        && found.len() <= 64
        && found
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid
        .then(|| found.to_ascii_uppercase())
        .ok_or_else(|| Error::Invalid(format!("{text:?} is not a Jira issue key or URL")))
}

/// A page's numeric id, from itself or a URL holding `/pages/<id>`.
fn page_id(text: &str) -> Result<String, Error> {
    let text = text.trim();
    let found = match text.split_once("/pages/") {
        Some((_, rest)) => rest.split(['/', '?', '#']).next().unwrap_or_default(),
        None => text,
    };
    let valid = !found.is_empty() && found.len() <= 32 && found.bytes().all(|b| b.is_ascii_digit());
    valid
        .then(|| found.to_owned())
        .ok_or_else(|| Error::Invalid(format!("{text:?} is not a Confluence page id or URL")))
}

async fn search_issues(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: Search = crate::input(input)?;
    let limit = search.limit.unwrap_or(25).clamp(1, MOST).to_string();
    let mut query = vec![
        ("jql", search.jql.as_str()),
        ("maxResults", limit.as_str()),
        ("fields", FIELDS),
    ];
    if let Some(next) = search.next.as_deref() {
        query.push(("nextPageToken", next));
    }
    let request = request(cx, Method::GET, "/rest/api/3/search/jql")?.query(&query);
    let found = cx.json(request).await?;
    let site = site(cx)?;
    let issues: Vec<Value> = found["issues"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|issue| summary(&site, issue))
        .collect();
    // Atlassian documents isLast, but answers without a token on the last page too.
    let next = match found["isLast"].as_bool() {
        Some(true) => Value::Null,
        _ => found["nextPageToken"].clone(),
    };
    Ok(json!({"issues": issues, "next": next}))
}

fn summary(site: &str, issue: &Value) -> Value {
    let fields = &issue["fields"];
    let key = issue["key"].as_str().unwrap_or_default();
    json!({
        "key": key,
        "summary": fields["summary"],
        "status": fields["status"]["name"],
        "type": fields["issuetype"]["name"],
        "priority": fields["priority"]["name"],
        "assignee": fields["assignee"]["displayName"],
        "updated": fields["updated"],
        "url": format!("{site}/browse/{key}"),
    })
}

async fn read_issue(cx: &Context, input: Value) -> Result<Value, Error> {
    let key = key(&crate::input::<Issue>(input)?.issue)?;
    let fields = format!("{FIELDS},reporter,created,description,comment");
    let request = request(cx, Method::GET, &format!("/rest/api/3/issue/{key}"))?
        .query(&[("fields", fields.as_str())]);
    let issue = cx.json(request).await?;
    let fields = &issue["fields"];
    let mut shown = summary(&site(cx)?, &issue);
    shown["reporter"] = fields["reporter"]["displayName"].clone();
    shown["created"] = fields["created"].clone();
    shown["description"] = json!(cut(&text(&fields["description"]), TEXT));
    let comments = fields["comment"]["comments"].as_array().map(Vec::as_slice);
    let comments = comments.unwrap_or_default();
    shown["comments"] = comments[comments.len().saturating_sub(COMMENTS)..]
        .iter()
        .map(|comment| {
            json!({
                "author": comment["author"]["displayName"],
                "created": comment["created"],
                "text": cut(&text(&comment["body"]), COMMENT),
            })
        })
        .collect();
    Ok(shown)
}

async fn create_issue(cx: &Context, input: Value) -> Result<Value, Error> {
    let create: Create = crate::input(input)?;
    let kind = create.kind.unwrap_or_else(|| "Task".into());
    let mut fields = json!({
        "project": {"key": create.project},
        "summary": create.summary,
        "issuetype": {"name": kind},
    });
    if let Some(description) = create.description {
        fields["description"] = document(&description);
    }
    if let Some(assignee) = create.assignee {
        fields["assignee"] = person(cx, &assignee).await?;
    }
    let request = request(cx, Method::POST, "/rest/api/3/issue")?.json(&json!({"fields": fields}));
    let made = cx.json(request).await?;
    let key = made["key"].as_str().unwrap_or_default();
    Ok(json!({"key": key, "url": format!("{}/browse/{key}", site(cx)?)}))
}

/// An assignee field: `me`, an account id, or `none` for nobody.
async fn person(cx: &Context, who: &str) -> Result<Value, Error> {
    match who.trim() {
        "none" => Ok(Value::Null),
        "me" => {
            let me = cx
                .json(request(cx, Method::GET, "/rest/api/3/myself")?)
                .await?;
            Ok(json!({"accountId": me["accountId"]}))
        }
        id => Ok(json!({"accountId": id})),
    }
}

async fn update_issue(cx: &Context, input: Value) -> Result<Value, Error> {
    let update: Update = crate::input(input)?;
    let key = key(&update.issue)?;
    let mut fields = serde_json::Map::new();
    if let Some(summary) = update.summary {
        fields.insert("summary".into(), json!(summary));
    }
    if let Some(description) = update.description {
        fields.insert("description".into(), document(&description));
    }
    if let Some(assignee) = update.assignee {
        fields.insert("assignee".into(), person(cx, &assignee).await?);
    }
    let path = format!("/rest/api/3/issue/{key}");
    let mut changed: Vec<String> = fields.keys().cloned().collect();
    if !fields.is_empty() {
        let body = json!({"fields": fields});
        cx.send(request(cx, Method::PUT, &path)?.json(&body))
            .await?;
    }
    if let Some(status) = update.status {
        transition(cx, &path, &status).await?;
        changed.push("status".into());
    }
    let url = format!("{}/browse/{key}", site(cx)?);
    Ok(json!({"key": key, "url": url, "changed": changed}))
}

/// Moves an issue through the transition named `status`, or the one leading to that status.
async fn transition(cx: &Context, issue: &str, status: &str) -> Result<(), Error> {
    let path = format!("{issue}/transitions");
    let offered = cx.json(request(cx, Method::GET, &path)?).await?;
    let named = |value: &Value| {
        value
            .as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(status))
    };
    let list = offered["transitions"].as_array().map(Vec::as_slice);
    let found = list
        .unwrap_or_default()
        .iter()
        .find(|transition| named(&transition["name"]) || named(&transition["to"]["name"]))
        .ok_or_else(|| {
            Error::Invalid(format!(
                "the issue cannot move to {status:?} from its current status"
            ))
        })?;
    let body = json!({"transition": {"id": found["id"]}});
    cx.send(request(cx, Method::POST, &path)?.json(&body))
        .await?;
    Ok(())
}

async fn comment(cx: &Context, input: Value) -> Result<Value, Error> {
    let comment: Comment = crate::input(input)?;
    let path = format!("/rest/api/3/issue/{}/comment", key(&comment.issue)?);
    let body = json!({"body": document(&comment.text)});
    let made = cx
        .json(request(cx, Method::POST, &path)?.json(&body))
        .await?;
    Ok(json!({"id": made["id"], "created": made["created"]}))
}

async fn search_pages(cx: &Context, input: Value) -> Result<Value, Error> {
    let find: Find = crate::input(input)?;
    let limit = find.limit.unwrap_or(25).clamp(1, MOST).to_string();
    let mut query = vec![("cql", find.cql.as_str()), ("limit", limit.as_str())];
    if let Some(next) = find.next.as_deref() {
        query.push(("cursor", next));
    }
    let request = request(cx, Method::GET, "/wiki/rest/api/search")?.query(&query);
    let found = cx.json(request).await?;
    let site = site(cx)?;
    let results: Vec<Value> = found["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|result| {
            json!({
                "id": result["content"]["id"],
                "type": result["content"]["type"],
                "title": plain(result["title"].as_str().unwrap_or_default()),
                "space": result["resultGlobalContainer"]["title"],
                "excerpt": cut(&plain(result["excerpt"].as_str().unwrap_or_default()), 300),
                "modified": result["lastModified"],
                "url": format!("{site}/wiki{}", result["url"].as_str().unwrap_or_default()),
            })
        })
        .collect();
    Ok(json!({"results": results, "next": next_cursor(&found)}))
}

/// The cursor in the `next` link Confluence answers when there are more results.
fn next_cursor(found: &Value) -> Value {
    let Some(next) = found["_links"]["next"].as_str() else {
        return Value::Null;
    };
    let url = Url::parse("https://site.invalid").and_then(|base| base.join(next));
    url.ok()
        .and_then(|url| {
            url.query_pairs()
                .find(|(name, _)| name == "cursor")
                .map(|(_, value)| value.into_owned())
        })
        .map_or(Value::Null, Value::String)
}

/// Search answers mark matches with `@@@hl@@@` and `@@@endhl@@@`.
fn plain(text: &str) -> String {
    text.replace("@@@hl@@@", "").replace("@@@endhl@@@", "")
}

async fn read_page(cx: &Context, input: Value) -> Result<Value, Error> {
    let id = page_id(&crate::input::<Page>(input)?.page)?;
    let request = request(cx, Method::GET, &format!("/wiki/api/v2/pages/{id}"))?
        .query(&[("body-format", "atlas_doc_format")]);
    let page = cx.json(request).await?;
    let body = page["body"]["atlas_doc_format"]["value"]
        .as_str()
        .unwrap_or("null");
    let body: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    Ok(json!({
        "id": page["id"],
        "title": page["title"],
        "status": page["status"],
        "version": page["version"]["number"],
        "updated": page["version"]["createdAt"],
        "url": format!("{}/wiki{}", site(cx)?, page["_links"]["webui"].as_str().unwrap_or_default()),
        "text": cut(&text(&body), TEXT),
    }))
}

async fn create_page(cx: &Context, input: Value) -> Result<Value, Error> {
    let create: NewPage = crate::input(input)?;
    let parent = create.parent.as_deref().map(page_id).transpose()?;
    let spaces = request(cx, Method::GET, "/wiki/api/v2/spaces")?.query(&[("keys", &create.space)]);
    let spaces = cx.json(spaces).await?;
    let space = spaces["results"][0]["id"]
        .as_str()
        .ok_or_else(|| Error::NotFound(format!("no space with the key {}", create.space)))?;
    let mut body = json!({
        "spaceId": space,
        "status": "current",
        "title": create.title,
        "body": {"representation": "atlas_doc_format", "value": document(&create.text).to_string()},
    });
    if let Some(parent) = parent {
        body["parentId"] = json!(parent);
    }
    let made = cx
        .json(request(cx, Method::POST, "/wiki/api/v2/pages")?.json(&body))
        .await?;
    let url = format!(
        "{}/wiki{}",
        site(cx)?,
        made["_links"]["webui"].as_str().unwrap_or_default()
    );
    Ok(json!({"id": made["id"], "title": made["title"], "url": url}))
}

/// Plain text as an Atlassian document: blank-line-separated paragraphs, lines kept.
fn document(text: &str) -> Value {
    let paragraphs: Vec<Value> = text
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(|block| {
            let mut inline = Vec::new();
            for (index, line) in block.lines().enumerate() {
                if index > 0 {
                    inline.push(json!({"type": "hardBreak"}));
                }
                if !line.is_empty() {
                    inline.push(json!({"type": "text", "text": line}));
                }
            }
            json!({"type": "paragraph", "content": inline})
        })
        .collect();
    json!({"type": "doc", "version": 1, "content": paragraphs})
}

/// An Atlassian document's text, one block per line.
fn text(document: &Value) -> String {
    let mut out = String::new();
    write(document, &mut out);
    let lines: Vec<&str> = out.lines().map(str::trim_end).collect();
    lines.join("\n").trim().to_owned()
}

fn write(node: &Value, out: &mut String) {
    let attrs = &node["attrs"];
    match node["type"].as_str().unwrap_or_default() {
        "text" => out.push_str(node["text"].as_str().unwrap_or_default()),
        "hardBreak" => out.push('\n'),
        "mention" | "emoji" | "status" => out.push_str(attrs["text"].as_str().unwrap_or_default()),
        "inlineCard" | "blockCard" => out.push_str(attrs["url"].as_str().unwrap_or_default()),
        kind => {
            if kind == "listItem" {
                out.push_str("- ");
            }
            for child in node["content"].as_array().into_iter().flatten() {
                write(child, out);
            }
            if !matches!(kind, "listItem" | "bulletList" | "orderedList" | "doc") {
                out.push('\n');
            }
        }
    }
}

fn cut(text: &str, longest: usize) -> String {
    match text.char_indices().nth(longest) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

/// Open issues newly assigned to the person. The cursor is the JSON list of keys already seen.
async fn poll(cx: &Context, cursor: Option<&str>) -> Result<Poll, Error> {
    if !MANIFEST.enabled(cx.config()).contains(&"jira") {
        return Ok(Poll {
            events: Vec::new(),
            cursor: cursor.map(str::to_owned),
        });
    }
    let mut seen: Vec<String> = cursor
        .and_then(|cursor| serde_json::from_str(cursor).ok())
        .unwrap_or_default();
    // shortcut: the 50 most recently changed; more assignments in one interval are missed, page if that bites.
    let query = [("jql", ASSIGNED), ("maxResults", "50"), ("fields", FIELDS)];
    let request = request(cx, Method::GET, "/rest/api/3/search/jql")?.query(&query);
    let found = cx.json(request).await?;
    let site = site(cx)?;
    let mut events = Vec::new();
    for issue in found["issues"].as_array().into_iter().flatten() {
        let shown = summary(&site, issue);
        let key = shown["key"].as_str().unwrap_or_default().to_owned();
        if key.is_empty() || seen.contains(&key) {
            continue;
        }
        seen.push(key.clone());
        events.push(Event {
            kind: "issue.assigned",
            summary: format!(
                "{key} assigned to you: {}",
                shown["summary"].as_str().unwrap_or_default()
            ),
            id: key,
            data: shown,
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

    const SITE: &str = "/acme.atlassian.net";

    fn cx(fixture: &Fixture, config: Value) -> Context {
        let credential = Credential::Fields {
            fields: [("token".into(), "ATATT3xFfGF0secret".into())].into(),
        };
        Context::new(config, credential).with_origin(&fixture.origin())
    }

    fn jira(fixture: &Fixture) -> Context {
        cx(
            fixture,
            json!({"site": "https://acme.atlassian.net/jira/your-work", "email": "victor@acme.dev"}),
        )
    }

    fn recorded(name: &str) -> Value {
        let path = format!(
            "{}/fixtures/atlassian/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn at(method: &str, path: &str, status: u16, body: Value) -> Exchange {
        Exchange::new(method, &format!("{SITE}{path}"), status, body)
    }

    #[tokio::test]
    async fn search_issues_uses_the_jql_endpoint_with_basic_auth_and_pages() {
        let fixture = Fixture::start(vec![at(
            "GET",
            "/rest/api/3/search/jql",
            200,
            recorded("search"),
        )])
        .await;
        let input = json!({"jql": "assignee = currentUser()", "limit": 2, "next": "page-2"});
        let found = Atlassian
            .call(&jira(&fixture), "search_issues", input)
            .await
            .unwrap();
        assert_eq!(found["issues"][0]["key"], "PROJ-7");
        assert_eq!(found["issues"][0]["status"], "In Progress");
        assert_eq!(
            found["issues"][0]["url"],
            "https://acme.atlassian.net/browse/PROJ-7"
        );
        assert_eq!(found["next"], "CAEaAggD");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].target,
            "/acme.atlassian.net/rest/api/3/search/jql?jql=assignee+%3D+currentUser%28%29&maxResults=2&fields=summary%2Cstatus%2Cassignee%2Cpriority%2Cissuetype%2Cupdated&nextPageToken=page-2"
        );
        // base64 of victor@acme.dev:ATATT3xFfGF0secret
        assert_eq!(
            seen[0].header("authorization"),
            Some("Basic dmljdG9yQGFjbWUuZGV2OkFUQVRUM3hGZkdGMHNlY3JldA==")
        );
    }

    #[tokio::test]
    async fn read_issue_turns_the_description_and_comments_into_text() {
        let fixture = Fixture::start(vec![at(
            "GET",
            "/rest/api/3/issue/PROJ-7",
            200,
            recorded("issue"),
        )])
        .await;
        let url = "https://acme.atlassian.net/browse/proj-7";
        let read = Atlassian
            .call(&jira(&fixture), "read_issue", json!({"issue": url}))
            .await
            .unwrap();
        assert_eq!(
            read["description"],
            "Checkout fails for @Ada Lovelace\n- on Safari\n- on Firefox"
        );
        assert_eq!(read["comments"][0]["text"], "Seen on staging too.");
        assert_eq!(read["reporter"], "Ada Lovelace");
        assert!(fixture.finish().await[0].target.contains("fields=summary"));
    }

    #[tokio::test]
    async fn create_issue_sends_a_document_and_assigns_to_me() {
        let fixture = Fixture::start(vec![
            at("GET", "/rest/api/3/myself", 200, recorded("myself")),
            at("POST", "/rest/api/3/issue", 201, json!({"id": "10007", "key": "PROJ-8", "self": "https://acme.atlassian.net/rest/api/3/issue/10007"})),
        ])
        .await;
        let input = json!({"project": "PROJ", "summary": "Fix checkout", "description": "One\nTwo", "assignee": "me"});
        let made = Atlassian
            .call(&jira(&fixture), "create_issue", input)
            .await
            .unwrap();
        assert_eq!(
            made,
            json!({"key": "PROJ-8", "url": "https://acme.atlassian.net/browse/PROJ-8"})
        );
        let sent = fixture.finish().await[1].json();
        assert_eq!(
            sent["fields"],
            json!({
                "project": {"key": "PROJ"},
                "summary": "Fix checkout",
                "issuetype": {"name": "Task"},
                "assignee": {"accountId": "5b10ac8d82e05b22cc7d4ef5"},
                "description": {"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [
                    {"type": "text", "text": "One"}, {"type": "hardBreak"}, {"type": "text", "text": "Two"}
                ]}]},
            })
        );
    }

    #[tokio::test]
    async fn update_issue_edits_fields_then_moves_it_through_the_matching_transition() {
        let fixture = Fixture::start(vec![
            at("PUT", "/rest/api/3/issue/PROJ-7", 204, Value::Null),
            at(
                "GET",
                "/rest/api/3/issue/PROJ-7/transitions",
                200,
                recorded("transitions"),
            ),
            at(
                "POST",
                "/rest/api/3/issue/PROJ-7/transitions",
                204,
                Value::Null,
            ),
        ])
        .await;
        let input = json!({"issue": "PROJ-7", "summary": "Fix checkout", "assignee": "none", "status": "done"});
        let changed = Atlassian
            .call(&jira(&fixture), "update_issue", input)
            .await
            .unwrap();
        let mut names: Vec<&str> = changed["changed"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        names.sort_unstable();
        assert_eq!(names, ["assignee", "status", "summary"]);
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].json(),
            json!({"fields": {"summary": "Fix checkout", "assignee": null}})
        );
        assert_eq!(seen[2].json(), json!({"transition": {"id": "31"}}));
    }

    #[tokio::test]
    async fn comment_on_issue_posts_a_document() {
        let fixture = Fixture::start(vec![at(
            "POST",
            "/rest/api/3/issue/PROJ-7/comment",
            201,
            json!({"id": "10100", "created": "2026-11-05T10:00:00.000+0000"}),
        )])
        .await;
        let input = json!({"issue": "PROJ-7", "text": "Deployed."});
        let made = Atlassian
            .call(&jira(&fixture), "comment_on_issue", input)
            .await
            .unwrap();
        assert_eq!(made["id"], "10100");
        let sent = fixture.finish().await[0].json();
        assert_eq!(
            sent["body"]["content"][0]["content"][0]["text"],
            "Deployed."
        );
    }

    #[tokio::test]
    async fn search_pages_strips_highlights_and_answers_the_next_cursor() {
        let fixture = Fixture::start(vec![at(
            "GET",
            "/wiki/rest/api/search",
            200,
            recorded("cql"),
        )])
        .await;
        let input = json!({"cql": "type = page AND text ~ \"roadmap\"", "limit": 1});
        let found = Atlassian
            .call(&jira(&fixture), "search_pages", input)
            .await
            .unwrap();
        assert_eq!(found["results"][0]["title"], "2027 roadmap");
        assert_eq!(
            found["results"][0]["url"],
            "https://acme.atlassian.net/wiki/spaces/ENG/pages/98306/2027+roadmap"
        );
        assert_eq!(found["next"], "_sa_WyJcdDk4MzA2");
        assert!(fixture.finish().await[0].target.ends_with("&limit=1"));
    }

    #[tokio::test]
    async fn read_page_reads_its_document_as_text() {
        let fixture = Fixture::start(vec![at(
            "GET",
            "/wiki/api/v2/pages/98306?body-format=atlas_doc_format",
            200,
            recorded("page"),
        )])
        .await;
        let url = "https://acme.atlassian.net/wiki/spaces/ENG/pages/98306/2027+roadmap";
        let read = Atlassian
            .call(&jira(&fixture), "read_page", json!({"page": url}))
            .await
            .unwrap();
        assert_eq!(read["title"], "2027 roadmap");
        assert_eq!(
            read["text"],
            "Goals\nShip the sandbox.\nhttps://acme.atlassian.net/browse/PROJ-7"
        );
        fixture.finish().await;
    }

    #[tokio::test]
    async fn create_page_finds_the_space_by_key() {
        let fixture = Fixture::start(vec![
            at("GET", "/wiki/api/v2/spaces?keys=ENG", 200, json!({"results": [{"id": "65538", "key": "ENG", "name": "Engineering"}], "_links": {}})),
            at("POST", "/wiki/api/v2/pages", 200, recorded("page")),
        ])
        .await;
        let input =
            json!({"space": "ENG", "title": "2027 roadmap", "text": "Goals", "parent": "98000"});
        let made = Atlassian
            .call(&jira(&fixture), "create_page", input)
            .await
            .unwrap();
        assert_eq!(
            made["url"],
            "https://acme.atlassian.net/wiki/spaces/ENG/pages/98306/2027+roadmap"
        );
        let sent = fixture.finish().await[1].json();
        assert_eq!(sent["spaceId"], "65538");
        assert_eq!(sent["parentId"], "98000");
        assert_eq!(sent["body"]["representation"], "atlas_doc_format");
        let value: Value = serde_json::from_str(sent["body"]["value"].as_str().unwrap()).unwrap();
        assert_eq!(value["content"][0]["content"][0]["text"], "Goals");
    }

    #[tokio::test]
    async fn refusals_map_to_errors_and_bad_input_or_a_foreign_site_never_reach_the_network() {
        let cases = [
            (401, json!({}), Error::Unauthorized),
            (403, json!({"errorMessages": ["You do not have permission"], "errors": {}}), Error::Forbidden("You do not have permission".into())),
            (404, json!({"errorMessages": ["Issue does not exist or you do not have permission to see it."], "errors": {}}), Error::NotFound("Issue does not exist or you do not have permission to see it.".into())),
            (429, json!({"message": "Rate limit exceeded"}), Error::Unavailable("rate limited: Rate limit exceeded".into())),
            (400, json!({"errorMessages": [], "errors": {"summary": "You must specify a summary of the issue."}}), Error::Rejected("{\"errorMessages\":[],\"errors\":{\"summary\":\"You must specify a summary of the issue.\"}}".into())),
        ];
        for (status, body, expected) in cases {
            let exchange =
                at("GET", "/rest/api/3/issue/PROJ-1", status, body).header("retry-after", "3600");
            let fixture = Fixture::start(vec![exchange]).await;
            let read = Atlassian
                .call(&jira(&fixture), "read_issue", json!({"issue": "PROJ-1"}))
                .await;
            assert_eq!(read, Err(expected));
            fixture.finish().await;
        }
        let fixture = Fixture::start(vec![]).await;
        for input in [
            json!({"issue": "PROJ 1/../x"}),
            json!({"issue": "PROJ-1", "x": 1}),
        ] {
            let refused = Atlassian.call(&jira(&fixture), "read_issue", input).await;
            assert!(matches!(refused, Err(Error::Invalid(_))), "{refused:?}");
        }
        for site in [
            "http://acme.atlassian.net",
            "https://acme.atlassian.net.evil.com",
            "https://atlassian.net",
            "https://evil.com/acme.atlassian.net",
        ] {
            let cx = cx(&fixture, json!({"site": site, "email": "victor@acme.dev"}));
            let refused = Atlassian
                .call(&cx, "read_issue", json!({"issue": "PROJ-1"}))
                .await;
            assert!(matches!(refused, Err(Error::Invalid(_))), "{site}");
        }
        assert!(fixture.finish().await.is_empty());
    }

    #[tokio::test]
    async fn poll_records_a_baseline_then_reports_newly_assigned_issues_once() {
        let search = recorded("search");
        let older = json!({"issues": [search["issues"][1]], "isLast": true});
        let fixture = Fixture::start(vec![
            at("GET", "/rest/api/3/search/jql", 200, older),
            at("GET", "/rest/api/3/search/jql", 200, search.clone()),
            at("GET", "/rest/api/3/search/jql", 200, search),
        ])
        .await;
        let cx = jira(&fixture);
        let baseline = Atlassian.poll(&cx, None).await.unwrap();
        assert!(baseline.events.is_empty());
        let polled = Atlassian
            .poll(&cx, baseline.cursor.as_deref())
            .await
            .unwrap();
        assert_eq!(polled.events.len(), 1);
        assert_eq!(polled.events[0].id, "PROJ-7");
        assert_eq!(
            polled.events[0].summary,
            "PROJ-7 assigned to you: Checkout fails on Safari"
        );
        let again = Atlassian.poll(&cx, polled.cursor.as_deref()).await.unwrap();
        assert!(again.events.is_empty());
        assert!(fixture.finish().await[0].target.contains("currentUser"));
        let confluence = cx_only_confluence();
        let skipped = Atlassian.poll(&confluence, Some("[]")).await.unwrap();
        assert_eq!(skipped.cursor.as_deref(), Some("[]"));
    }

    fn cx_only_confluence() -> Context {
        let credential = Credential::Fields {
            fields: [("token".into(), "ATATT3xFfGF0secret".into())].into(),
        };
        let config = json!({"site": "https://acme.atlassian.net", "email": "victor@acme.dev", "products": ["confluence"]});
        Context::new(config, credential).with_origin("http://127.0.0.1:9/")
    }
}

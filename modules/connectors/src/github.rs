//! GitHub through an app the person creates from a manifest, or a fine-grained personal access
//! token (https://docs.github.com/rest). The orchestrator runs the app's sign-in with the
//! functions below; the agent always holds a bearer token: an installation token or the PAT.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Credential, Error, Event, EventSpec,
    Field, FieldKind, Level, Manifest, Poll, Risk, Step, ToolSpec,
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use reqwest::{Method, RequestBuilder, Url, header::ACCEPT};
use rsa::{
    RsaPrivateKey,
    pkcs1::DecodeRsaPrivateKey,
    pkcs1v15::SigningKey,
    pkcs8::DecodePrivateKey,
    signature::{SignatureEncoding, Signer},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;

const API: &str = "https://api.github.com";
/// Pinned so a GitHub release cannot change the answers under us.
const VERSION: &str = "2022-11-28";
const JSON: &str = "application/vnd.github+json";
/// The most characters of a body, file or diff the model gets.
const LONGEST: usize = 60_000;
/// A notification cursor older than any notification.
const EPOCH: &str = "1970-01-01T00:00:00Z";

pub struct Github;

static MANIFEST: Manifest = Manifest {
    id: "github",
    name: "GitHub",
    description: "Search, read and work on issues, pull requests and files in GitHub repositories.",
    category: Category::Dev,
    auth: &[
        Auth::GithubApp {
            fields: &[Field {
                key: "organization",
                label: "Organization",
                kind: FieldKind::Text,
                required: false,
                placeholder: "my-org",
                help: "Optional: create the app under this organization instead of your account.",
            }],
        },
        Auth::ApiKey {
            fields: &[Field {
                key: "token",
                label: "Fine-grained personal access token",
                kind: FieldKind::Secret,
                required: true,
                placeholder: "github_pat_…",
                help: "Limited to the repositories and permissions you choose.",
            }],
        },
    ],
    config: &[],
    products: &[],
    tools: &[
        ToolSpec {
            name: "search_repos",
            description: "Find repositories with GitHub's search syntax, such as `org:acme topic:api`.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["query"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "search_issues",
            description: "Find issues and pull requests with GitHub's search syntax, such as `repo:acme/api is:open is:pr author:me`.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["query"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "get_issue",
            description: "An issue or pull request with its body and its most recent comments.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"repo":{"type":"string","description":"owner/name"},"number":{"type":"integer","minimum":1}},"required":["repo","number"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_issues",
            description: "A repository's issues and pull requests, newest first, by state, labels or assignee.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"repo":{"type":"string","description":"owner/name"},"state":{"type":"string","enum":["open","closed","all"]},"labels":{"type":"string","description":"Comma-separated label names."},"assignee":{"type":"string","description":"A login, `none` or `*`."},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["repo"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "get_file",
            description: "A file's text at a branch, tag or commit; a directory's entries; a binary file is saved as a file.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"repo":{"type":"string","description":"owner/name"},"path":{"type":"string","description":"Such as src/main.rs; empty for the top directory."},"ref":{"type":"string","description":"A branch, tag or commit; the default branch when absent."}},"required":["repo","path"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_pull_requests",
            description: "A repository's pull requests, newest first, with their branches.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"repo":{"type":"string","description":"owner/name"},"state":{"type":"string","enum":["open","closed","all"]},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["repo"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "get_pull_request_diff",
            description: "A pull request's changes as a unified diff, cut short when very large.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"repo":{"type":"string","description":"owner/name"},"number":{"type":"integer","minimum":1}},"required":["repo","number"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "create_issue",
            description: "Open an issue with a title, a Markdown body, labels and assignees.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"repo":{"type":"string","description":"owner/name"},"title":{"type":"string"},"body":{"type":"string"},"labels":{"type":"array","items":{"type":"string"}},"assignees":{"type":"array","items":{"type":"string"}}},"required":["repo","title"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "update_issue",
            description: "Change an issue's or pull request's title, body, state, labels or assignees. Lists replace the current ones.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"repo":{"type":"string","description":"owner/name"},"number":{"type":"integer","minimum":1},"title":{"type":"string"},"body":{"type":"string"},"state":{"type":"string","enum":["open","closed"]},"state_reason":{"type":"string","enum":["completed","not_planned","reopened"]},"labels":{"type":"array","items":{"type":"string"}},"assignees":{"type":"array","items":{"type":"string"}}},"required":["repo","number"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "comment",
            description: "Comment on an issue or pull request; participants and watchers are notified.",
            risk: Risk::Send,
            product: None,
            input: r#"{"type":"object","properties":{"repo":{"type":"string","description":"owner/name"},"number":{"type":"integer","minimum":1},"body":{"type":"string","description":"Markdown."}},"required":["repo","number","body"],"additionalProperties":false}"#,
        },
        // Send, unlike create_issue: opening a pull request requests reviews (code owners) and
        // starts the repository's workflows, which can run with its secrets and deploy.
        ToolSpec {
            name: "create_pull_request",
            description: "Open a pull request from an existing branch; reviewers are asked and checks start.",
            risk: Risk::Send,
            product: None,
            input: r#"{"type":"object","properties":{"repo":{"type":"string","description":"owner/name"},"title":{"type":"string"},"head":{"type":"string","description":"The branch with the changes, or owner:branch from a fork."},"base":{"type":"string","description":"The branch to merge into."},"body":{"type":"string"},"draft":{"type":"boolean"}},"required":["repo","title","head","base"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_notifications",
            description: "The account's unread notifications, newest first. Needs a classic token.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"all":{"type":"boolean","description":"Include read ones."},"participating":{"type":"boolean","description":"Only where the account takes part or is mentioned."},"limit":{"type":"integer","minimum":1,"maximum":50}},"additionalProperties":false}"#,
        },
    ],
    events: &[EventSpec {
        kind: "notification.received",
        description: "A new or updated GitHub notification. Only a classic token can read them.",
        product: None,
    }],
    level: Level::Write,
    guide: &[
        Step {
            title: "Create the GitHub App",
            body: "Choose Create GitHub App. GitHub opens with the app already filled in, on your account or on the organization named above; confirm with Create GitHub App.",
            link: None,
            copy: None,
        },
        Step {
            title: "Install it",
            body: "GitHub then asks where to install the app: choose the repositories it may use. You can change them later in the app's settings.",
            link: None,
            copy: None,
        },
        Step {
            title: "Or paste a personal access token",
            body: "Instead of an app, generate a fine-grained token: choose its repositories and grant the repository permissions below. Notifications need a classic token with the notifications and repo scopes; GitHub refuses them to fine-grained and app tokens.",
            link: Some("https://github.com/settings/personal-access-tokens/new"),
            copy: Some(
                "Contents: Read-only\nIssues: Read and write\nPull requests: Read and write\nMetadata: Read-only",
            ),
        },
    ],
    warning: None,
};

/// The manifest GitHub creates the app from: private, with no webhook, and the permissions the
/// tools need.
pub fn app_manifest(name: &str, url: &str, redirect_url: &str) -> Value {
    json!({
        "name": name,
        "url": url,
        "redirect_url": redirect_url,
        "public": false,
        "hook_attributes": {"url": url, "active": false},
        "default_permissions": {
            "contents": "read",
            "issues": "write",
            "pull_requests": "write",
            "metadata": "read",
        },
        "default_events": [],
    })
}

/// Where the browser POSTs the manifest form: the person's account, or an organization's.
pub fn app_creation_url(organization: Option<&str>, state: &str) -> String {
    let mut url = Url::parse("https://github.com/").expect("a valid address");
    let owner = organization
        .map(str::trim)
        .filter(|organization| !organization.is_empty())
        .map(|organization| vec!["organizations", organization])
        .unwrap_or_default();
    url.path_segments_mut()
        .expect("an address with a path")
        .pop_if_empty()
        .extend(owner)
        .extend(["settings", "apps", "new"]);
    url.query_pairs_mut().append_pair("state", state);
    url.into()
}

/// The app GitHub created. No `Debug`, so its key never reaches a log.
pub struct App {
    pub id: u64,
    pub slug: String,
    pub pem: String,
    pub html_url: String,
}

/// Exchanges the redirect's code for the new app. The call is unauthenticated, so `cx` holds no
/// token (an empty `Credential::Fields`); a bearer token here would be refused.
pub async fn convert(cx: &Context, code: &str) -> Result<App, Error> {
    let plain = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
    if code.is_empty() || !code.bytes().all(plain) {
        return Err(Error::Invalid("not a GitHub app manifest code".into()));
    }
    let path = format!("{API}/app-manifests/{code}/conversions");
    let made = cx
        .json(
            cx.post(&path)?
                .header(ACCEPT, JSON)
                .header("x-github-api-version", VERSION),
        )
        .await?;
    let missing = || Error::Unavailable("GitHub's answer lacks the new app".into());
    let text = |key: &str| made[key].as_str().map(str::to_owned).ok_or_else(missing);
    Ok(App {
        id: made["id"].as_u64().ok_or_else(missing)?,
        slug: text("slug")?,
        pem: text("pem")?,
        html_url: text("html_url")?,
    })
}

/// An RS256 JWT for the app, valid for nine minutes from a minute ago (GitHub allows ten and
/// clocks drift). `pem` is PKCS#1, as GitHub issues it, or PKCS#8.
pub fn jwt(app_id: u64, pem: &str, now: i64) -> Result<String, Error> {
    let key = RsaPrivateKey::from_pkcs1_pem(pem)
        .or_else(|_| RsaPrivateKey::from_pkcs8_pem(pem))
        .map_err(|_| Error::Invalid("the app's private key is not an RSA key in PEM".into()))?;
    let claims = json!({"iat": now - 60, "exp": now + 540, "iss": app_id});
    let signed = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let signature = SigningKey::<Sha256>::new(key)
        .try_sign(signed.as_bytes())
        .map_err(|_| Error::Invalid("the app's private key cannot sign".into()))?;
    Ok(format!(
        "{signed}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    ))
}

/// An hour-long installation token, minted with `cx`'s credential being the app's JWT. Without
/// an `installation` it uses the app's first one. Answers the token, when it expires in Unix
/// seconds, and the installation.
pub async fn installation_token(
    cx: &Context,
    installation: Option<u64>,
) -> Result<(String, i64, u64), Error> {
    let id = match installation {
        Some(id) => id,
        None => first_installation(cx).await?,
    };
    let path = format!("/app/installations/{id}/access_tokens");
    let minted = cx.json(api(cx, Method::POST, &path)?).await?;
    let missing = || Error::Unavailable("GitHub's answer lacks the token".into());
    let token = minted["token"].as_str().ok_or_else(missing)?;
    let expires = minted["expires_at"]
        .as_str()
        .and_then(unix)
        .ok_or_else(missing)?;
    Ok((token.into(), expires, id))
}

async fn first_installation(cx: &Context) -> Result<u64, Error> {
    let request = api(cx, Method::GET, "/app/installations")?.query(&[("per_page", "1")]);
    cx.json(request).await?[0]["id"].as_u64().ok_or_else(|| {
        Error::NotFound(
            "the app is not installed yet; install it on the repositories it may use".into(),
        )
    })
}

/// The page where the person installs the app on their repositories.
pub fn install_url(slug: &str) -> String {
    format!("https://github.com/apps/{slug}/installations/new")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: String,
    limit: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Numbered {
    repo: String,
    number: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Issues {
    repo: String,
    state: Option<String>,
    labels: Option<String>,
    assignee: Option<String>,
    limit: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    repo: String,
    path: String,
    #[serde(rename = "ref")]
    reference: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pulls {
    repo: String,
    state: Option<String>,
    limit: Option<u8>,
}

/// Sent to GitHub as it is, without the repository and number, which are in the path.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Change {
    #[serde(skip_serializing)]
    repo: String,
    #[serde(skip_serializing)]
    number: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    labels: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    assignees: Option<Vec<String>>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NewIssue {
    #[serde(skip_serializing)]
    repo: String,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    labels: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    assignees: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Comment {
    repo: String,
    number: u64,
    body: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Pull {
    #[serde(skip_serializing)]
    repo: String,
    title: String,
    head: String,
    base: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    draft: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Notifications {
    #[serde(default)]
    all: bool,
    #[serde(default)]
    participating: bool,
    limit: Option<u8>,
}

impl Connector for Github {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    /// A PAT names its user; an installation token has none, so its first repository's owner.
    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let name = match cx.json(api(cx, Method::GET, "/user")?).await {
                Ok(me) => me["login"].as_str().unwrap_or("GitHub account").to_owned(),
                Err(Error::Forbidden(_)) => installation_owner(cx).await?,
                Err(error) => return Err(error),
            };
            Ok(Account { name })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "search_repos" => search_repos(cx, input).await,
                "search_issues" => search_issues(cx, input).await,
                "get_issue" => get_issue(cx, input).await,
                "list_issues" => list_issues(cx, input).await,
                "get_file" => get_file(cx, input).await,
                "list_pull_requests" => list_pulls(cx, input).await,
                "get_pull_request_diff" => diff(cx, input).await,
                "create_issue" => create_issue(cx, input).await,
                "update_issue" => update_issue(cx, input).await,
                "comment" => comment(cx, input).await,
                "create_pull_request" => create_pull(cx, input).await,
                "list_notifications" => list_notifications(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    /// New notifications since the newest one seen. Only a classic token can read them: for any
    /// other GitHub answers 403, and the connection simply has no events.
    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(async move {
            let Some(since) = cursor else {
                return baseline(cx).await;
            };
            // shortcut: one page of 50 per poll; more in one interval are skipped, page if that bites.
            let request = api(cx, Method::GET, "/notifications")?
                .query(&[("since", since), ("per_page", "50")]);
            let found = match cx.json(request).await {
                Err(Error::Forbidden(_)) => Value::Null,
                other => other?,
            };
            let list = found.as_array().map(Vec::as_slice).unwrap_or_default();
            let newest = list
                .iter()
                .filter_map(|thread| thread["updated_at"].as_str())
                .max()
                .unwrap_or(since);
            Ok(Poll {
                events: list.iter().map(event).collect(),
                cursor: Some(newest.into()),
            })
        })
    }
}

/// The newest notification, read or not, is where polling starts; with none ever, any is new.
async fn baseline(cx: &Context) -> Result<Poll, Error> {
    let request =
        api(cx, Method::GET, "/notifications")?.query(&[("all", "true"), ("per_page", "1")]);
    let newest = match cx.json(request).await {
        Err(Error::Forbidden(_)) => Value::Null,
        other => other?,
    };
    let cursor = newest[0]["updated_at"].as_str().unwrap_or(EPOCH);
    Ok(Poll {
        events: Vec::new(),
        cursor: Some(cursor.into()),
    })
}

fn event(thread: &Value) -> Event {
    let shown = notification(thread);
    Event {
        kind: "notification.received",
        // A thread updated again is a new event.
        id: format!(
            "{}@{}",
            thread["id"].as_str().unwrap_or_default(),
            thread["updated_at"].as_str().unwrap_or_default()
        ),
        summary: format!(
            "{} in {}: {}",
            thread["reason"].as_str().unwrap_or("notification"),
            thread["repository"]["full_name"]
                .as_str()
                .unwrap_or("GitHub"),
            thread["subject"]["title"].as_str().unwrap_or_default()
        ),
        data: shown,
    }
}

async fn installation_owner(cx: &Context) -> Result<String, Error> {
    let request = api(cx, Method::GET, "/installation/repositories")?.query(&[("per_page", "1")]);
    let found = cx.json(request).await?;
    let owner = found["repositories"][0]["owner"]["login"].as_str();
    Ok(owner.unwrap_or("GitHub App installation").into())
}

fn request(
    cx: &Context,
    method: Method,
    path: &str,
    accept: &str,
) -> Result<RequestBuilder, Error> {
    let request = cx
        .request(method, &format!("{API}{path}"))?
        .header(ACCEPT, accept)
        .header("x-github-api-version", VERSION);
    // Context already sends a token credential as a bearer.
    Ok(match cx.credential() {
        Credential::Fields { .. } => request.bearer_auth(cx.token()?),
        Credential::Token { .. } => request,
    })
}

fn api(cx: &Context, method: Method, path: &str) -> Result<RequestBuilder, Error> {
    request(cx, method, path, JSON)
}

/// `owner/name`, refused when it could reach outside `/repos/`.
fn repo(text: &str) -> Result<&str, Error> {
    let plain = |part: &str| {
        !matches!(part, "" | "." | "..")
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    let valid = text
        .split_once('/')
        .is_some_and(|(owner, name)| plain(owner) && plain(name));
    valid
        .then_some(text)
        .ok_or_else(|| Error::Invalid(format!("{text:?} is not owner/name")))
}

fn limit(limit: Option<u8>, default: u8) -> String {
    limit.unwrap_or(default).clamp(1, 100).to_string()
}

fn clip(text: &str, longest: usize) -> String {
    match text.char_indices().nth(longest) {
        Some((end, _)) => format!("{}\n…[cut short]", &text[..end]),
        None => text.into(),
    }
}

fn names(list: &Value, key: &str) -> Vec<Value> {
    let list = list.as_array().map(Vec::as_slice).unwrap_or_default();
    list.iter().map(|item| item[key].clone()).collect()
}

fn list(found: &Value, shown: fn(&Value) -> Value) -> Vec<Value> {
    let found = found.as_array().map(Vec::as_slice).unwrap_or_default();
    found.iter().map(shown).collect()
}

/// An issue or pull request as the model needs it.
fn issue(issue: &Value) -> Value {
    let repository = issue["repository_url"].as_str().unwrap_or_default();
    json!({
        "repo": repository.rsplit_once("/repos/").map(|(_, repo)| repo),
        "number": issue["number"],
        "title": issue["title"],
        "state": issue["state"],
        "pull_request": issue["pull_request"].is_object(),
        "author": issue["user"]["login"],
        "labels": names(&issue["labels"], "name"),
        "assignees": names(&issue["assignees"], "login"),
        "comments": issue["comments"],
        "updated_at": issue["updated_at"],
        "url": issue["html_url"],
    })
}

fn repository(repo: &Value) -> Value {
    json!({
        "repo": repo["full_name"],
        "description": repo["description"],
        "private": repo["private"],
        "stars": repo["stargazers_count"],
        "default_branch": repo["default_branch"],
        "updated_at": repo["updated_at"],
        "url": repo["html_url"],
    })
}

fn pull(pull: &Value) -> Value {
    json!({
        "number": pull["number"],
        "title": pull["title"],
        "state": pull["state"],
        "draft": pull["draft"],
        "author": pull["user"]["login"],
        "head": pull["head"]["label"],
        "base": pull["base"]["ref"],
        "updated_at": pull["updated_at"],
        "url": pull["html_url"],
    })
}

fn notification(thread: &Value) -> Value {
    json!({
        "id": thread["id"],
        "reason": thread["reason"],
        "unread": thread["unread"],
        "updated_at": thread["updated_at"],
        "title": thread["subject"]["title"],
        "type": thread["subject"]["type"],
        "repo": thread["repository"]["full_name"],
    })
}

async fn search_repos(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: Search = crate::input(input)?;
    let query = [("q", search.query), ("per_page", limit(search.limit, 20))];
    let found = cx
        .json(api(cx, Method::GET, "/search/repositories")?.query(&query))
        .await?;
    Ok(json!({"total": found["total_count"], "repos": list(&found["items"], repository)}))
}

async fn search_issues(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: Search = crate::input(input)?;
    let query = [("q", search.query), ("per_page", limit(search.limit, 20))];
    let found = cx
        .json(api(cx, Method::GET, "/search/issues")?.query(&query))
        .await?;
    Ok(json!({"total": found["total_count"], "issues": list(&found["items"], issue)}))
}

async fn get_issue(cx: &Context, input: Value) -> Result<Value, Error> {
    let numbered: Numbered = crate::input(input)?;
    let path = format!(
        "/repos/{}/issues/{}",
        repo(&numbered.repo)?,
        numbered.number
    );
    let found = cx.json(api(cx, Method::GET, &path)?).await?;
    // shortcut: the last 20 of the first 100 comments; page to the end if long threads matter.
    let request = api(cx, Method::GET, &format!("{path}/comments"))?.query(&[("per_page", "100")]);
    let comments = cx.json(request).await?;
    let comments = comments.as_array().map(Vec::as_slice).unwrap_or_default();
    let recent: Vec<Value> = comments[comments.len().saturating_sub(20)..]
        .iter()
        .map(|comment| {
            json!({
                "author": comment["user"]["login"],
                "created_at": comment["created_at"],
                "body": clip(comment["body"].as_str().unwrap_or_default(), 4000),
            })
        })
        .collect();
    let mut shown = issue(&found);
    shown["body"] = json!(clip(found["body"].as_str().unwrap_or_default(), LONGEST));
    shown["recent_comments"] = json!(recent);
    Ok(shown)
}

async fn list_issues(cx: &Context, input: Value) -> Result<Value, Error> {
    let issues: Issues = crate::input(input)?;
    let path = format!("/repos/{}/issues", repo(&issues.repo)?);
    let mut query = vec![("per_page", limit(issues.limit, 30))];
    let optional = [
        ("state", issues.state),
        ("labels", issues.labels),
        ("assignee", issues.assignee),
    ];
    query.extend(
        optional
            .into_iter()
            .filter_map(|(key, value)| Some((key, value?))),
    );
    let found = cx.json(api(cx, Method::GET, &path)?.query(&query)).await?;
    Ok(json!({"issues": list(&found, issue)}))
}

async fn get_file(cx: &Context, input: Value) -> Result<Value, Error> {
    let file: File = crate::input(input)?;
    let (owner, name) = repo(&file.repo)?.split_once('/').unwrap_or_default();
    let path = file.path.trim_matches('/');
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments
        .iter()
        .any(|segment| matches!(*segment, "." | ".."))
    {
        return Err(Error::Invalid(
            "a path inside the repository, without . or ..".into(),
        ));
    }
    // Segments are percent-encoded, so a name with `?` or `#` stays a name.
    let mut url = Url::parse(API).expect("a valid address");
    url.path_segments_mut()
        .expect("an address with a path")
        .pop_if_empty()
        .extend(["repos", owner, name, "contents"])
        .extend(segments);
    let mut request = api(cx, Method::GET, url.path())?;
    if let Some(reference) = &file.reference {
        request = request.query(&[("ref", reference)]);
    }
    contents(&cx.json(request).await?)
}

fn contents(found: &Value) -> Result<Value, Error> {
    if let Some(entries) = found.as_array() {
        let entry = |entry: &Value| json!({"name": entry["name"], "type": entry["type"], "path": entry["path"], "size": entry["size"]});
        return Ok(json!({"directory": entries.iter().map(entry).collect::<Vec<_>>()}));
    }
    // shortcut: GitHub inlines files up to 1 MB; fetch the raw media type if larger ones matter.
    if found["encoding"] != "base64" {
        return Err(Error::Rejected(
            "the file is over 1 MB or is not a regular file".into(),
        ));
    }
    let packed: String = found["content"]
        .as_str()
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    let bytes = STANDARD
        .decode(packed)
        .map_err(|_| Error::Unavailable("GitHub sent content that is not base64".into()))?;
    Ok(match String::from_utf8(bytes) {
        Ok(text) => {
            json!({"path": found["path"], "sha": found["sha"], "text": clip(&text, LONGEST)})
        }
        Err(binary) => crate::file(
            found["name"].as_str().unwrap_or("file"),
            None,
            binary.as_bytes(),
        ),
    })
}

async fn list_pulls(cx: &Context, input: Value) -> Result<Value, Error> {
    let pulls: Pulls = crate::input(input)?;
    let path = format!("/repos/{}/pulls", repo(&pulls.repo)?);
    let mut query = vec![("per_page", limit(pulls.limit, 30))];
    query.extend(pulls.state.map(|state| ("state", state)));
    let found = cx.json(api(cx, Method::GET, &path)?.query(&query)).await?;
    Ok(json!({"pull_requests": list(&found, pull)}))
}

async fn diff(cx: &Context, input: Value) -> Result<Value, Error> {
    let numbered: Numbered = crate::input(input)?;
    let path = format!("/repos/{}/pulls/{}", repo(&numbered.repo)?, numbered.number);
    let accept = "application/vnd.github.diff";
    let (body, _) = cx.bytes(request(cx, Method::GET, &path, accept)?).await?;
    Ok(json!({"diff": clip(&String::from_utf8_lossy(&body), LONGEST)}))
}

async fn create_issue(cx: &Context, input: Value) -> Result<Value, Error> {
    let new: NewIssue = crate::input(input)?;
    let path = format!("/repos/{}/issues", repo(&new.repo)?);
    let made = cx.json(api(cx, Method::POST, &path)?.json(&new)).await?;
    Ok(json!({"number": made["number"], "url": made["html_url"]}))
}

async fn update_issue(cx: &Context, input: Value) -> Result<Value, Error> {
    let change: Change = crate::input(input)?;
    let path = format!("/repos/{}/issues/{}", repo(&change.repo)?, change.number);
    let changed = cx
        .json(api(cx, Method::PATCH, &path)?.json(&change))
        .await?;
    Ok(issue(&changed))
}

async fn comment(cx: &Context, input: Value) -> Result<Value, Error> {
    let comment: Comment = crate::input(input)?;
    let path = format!(
        "/repos/{}/issues/{}/comments",
        repo(&comment.repo)?,
        comment.number
    );
    let body = json!({"body": comment.body});
    let made = cx.json(api(cx, Method::POST, &path)?.json(&body)).await?;
    Ok(json!({"id": made["id"], "url": made["html_url"]}))
}

async fn create_pull(cx: &Context, input: Value) -> Result<Value, Error> {
    let new: Pull = crate::input(input)?;
    let path = format!("/repos/{}/pulls", repo(&new.repo)?);
    let made = cx.json(api(cx, Method::POST, &path)?.json(&new)).await?;
    Ok(json!({"number": made["number"], "url": made["html_url"]}))
}

async fn list_notifications(cx: &Context, input: Value) -> Result<Value, Error> {
    let wanted: Notifications = crate::input(input)?;
    let query = [
        ("all", wanted.all.to_string()),
        ("participating", wanted.participating.to_string()),
        ("per_page", limit(wanted.limit, 30)),
    ];
    let found = cx
        .json(api(cx, Method::GET, "/notifications")?.query(&query))
        .await?;
    Ok(json!({"notifications": list(&found, notification)}))
}

/// Unix seconds of GitHub's `2016-07-11T22:14:10Z` timestamps.
fn unix(text: &str) -> Option<i64> {
    let number = |at: usize| text.get(at..at + 2)?.parse::<i64>().ok();
    let year = text.get(..4)?.parse::<i64>().ok()?;
    let (month, day) = (number(5)?, number(8)?);
    let seconds = number(11)? * 3600 + number(14)? * 60 + number(17)?;
    // Days from the civil date (Howard Hinnant's algorithm), years starting in March.
    let (year, month) = if month <= 2 {
        (year - 1, month + 9)
    } else {
        (year, month - 3)
    };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * month + 2) / 5 + day - 1;
    let days = era * 146_097 + of_era * 365 + of_era / 4 - of_era / 100 + of_year - 719_468;
    Some(days * 86_400 + seconds)
}

#[cfg(test)]
mod tests {
    //! Answers are trimmed from the examples in GitHub's REST reference.
    use super::*;
    use crate::testing::{Exchange, Fixture};
    use rsa::{
        pkcs1v15::{Signature, VerifyingKey},
        pkcs8::{EncodePrivateKey, LineEnding},
        signature::Verifier,
    };

    fn pat(fixture: &Fixture) -> Context {
        let credential = Credential::Fields {
            fields: [("token".into(), "github_pat_secret".into())].into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    fn bearer(fixture: &Fixture, token: &str) -> Context {
        let credential = Credential::Token {
            token: token.into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    fn recorded(name: &str) -> Value {
        let path = format!("{}/fixtures/github/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// A key generated once for these tests; it signs nothing real.
    fn key() -> String {
        let path = format!("{}/fixtures/github/app-key.pem", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(path).unwrap()
    }

    fn part(token: &str, index: usize) -> Vec<u8> {
        let part = token.split('.').nth(index).unwrap();
        URL_SAFE_NO_PAD.decode(part).unwrap()
    }

    #[test]
    fn jwt_is_signed_with_the_apps_key_and_claims_the_app_for_nine_minutes() {
        let private = RsaPrivateKey::from_pkcs1_pem(&key()).unwrap();
        let token = jwt(42, &key(), 1_700_000_000).unwrap();
        let header: Value = serde_json::from_slice(&part(&token, 0)).unwrap();
        assert_eq!(header, json!({"alg": "RS256", "typ": "JWT"}));
        let claims: Value = serde_json::from_slice(&part(&token, 1)).unwrap();
        assert_eq!(
            claims,
            json!({"iat": 1_699_999_940, "exp": 1_700_000_540, "iss": 42})
        );
        let (signed, _) = token.rsplit_once('.').unwrap();
        let signature = Signature::try_from(part(&token, 2).as_slice()).unwrap();
        let verifier = VerifyingKey::<Sha256>::new(private.to_public_key());
        assert!(verifier.verify(signed.as_bytes(), &signature).is_ok());
        let pkcs8 = private.to_pkcs8_pem(LineEnding::LF).unwrap();
        assert_eq!(jwt(42, &pkcs8, 1_700_000_000).unwrap(), token);
        let refused = jwt(42, "-----BEGIN RSA PRIVATE KEY-----\nnope", 0);
        assert!(matches!(refused, Err(Error::Invalid(message)) if !message.contains("nope")));
    }

    #[test]
    fn the_app_is_created_private_without_a_webhook_on_the_chosen_account() {
        let manifest = app_manifest("Cubebot", "https://cube.example", "https://cube.example/cb");
        assert_eq!(manifest["public"], false);
        assert_eq!(manifest["hook_attributes"]["active"], false);
        assert_eq!(manifest["redirect_url"], "https://cube.example/cb");
        assert_eq!(manifest["default_permissions"]["issues"], "write");
        assert_eq!(
            app_creation_url(None, "a b"),
            "https://github.com/settings/apps/new?state=a+b"
        );
        assert_eq!(
            app_creation_url(Some(" acme "), "s"),
            "https://github.com/organizations/acme/settings/apps/new?state=s"
        );
        assert_eq!(
            install_url("octoapp"),
            "https://github.com/apps/octoapp/installations/new"
        );
    }

    #[tokio::test]
    async fn convert_trades_the_code_for_the_new_app_without_credentials() {
        let fixture = Fixture::start(vec![Exchange::new(
            "POST",
            "/api.github.com/app-manifests/a1b2c3/conversions",
            201,
            recorded("conversion"),
        )])
        .await;
        let cx = Context::new(
            json!({}),
            Credential::Fields {
                fields: Default::default(),
            },
        )
        .with_origin(&fixture.origin());
        let app = convert(&cx, "a1b2c3").await.unwrap();
        assert_eq!((app.id, app.slug.as_str()), (1, "octoapp"));
        assert_eq!(app.html_url, "https://github.com/apps/octoapp");
        assert!(app.pem.starts_with("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(matches!(
            convert(&cx, "../user").await,
            Err(Error::Invalid(_))
        ));
        let seen = fixture.finish().await;
        assert_eq!(seen[0].header("authorization"), None);
    }

    #[tokio::test]
    async fn installation_token_uses_the_first_installation_and_reads_its_expiry() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api.github.com/app/installations",
                200,
                recorded("installations"),
            ),
            Exchange::new(
                "POST",
                "/api.github.com/app/installations/1/access_tokens",
                201,
                recorded("access_token"),
            ),
            Exchange::new("GET", "/api.github.com/app/installations", 200, json!([])),
        ])
        .await;
        let cx = bearer(&fixture, "app.jwt.token");
        let (token, expires, installation) = installation_token(&cx, None).await.unwrap();
        assert_eq!(token, "ghs_16C7e42F292c6912E7710c838347Ae178B4a");
        assert_eq!((expires, installation), (1_468_275_250, 1));
        assert!(matches!(
            installation_token(&cx, None).await,
            Err(Error::NotFound(_))
        ));
        let seen = fixture.finish().await;
        assert_eq!(
            seen[1].header("authorization"),
            Some("Bearer app.jwt.token")
        );
        assert_eq!(unix("2000-02-29T00:00:00Z"), Some(951_782_400));
    }

    #[tokio::test]
    async fn check_names_an_installation_by_its_repositories_owner() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api.github.com/user",
                403,
                json!({"message": "Resource not accessible by integration"}),
            ),
            Exchange::new(
                "GET",
                "/api.github.com/installation/repositories?per_page=1",
                200,
                recorded("installation_repositories"),
            ),
        ])
        .await;
        let account = Github
            .check(&bearer(&fixture, "ghs_installation"))
            .await
            .unwrap();
        assert_eq!(account.name, "vicotrbb");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].header("authorization"),
            Some("Bearer ghs_installation")
        );
        assert_eq!(seen[0].header("x-github-api-version"), Some(VERSION));
    }

    #[tokio::test]
    async fn get_issue_shows_the_body_and_recent_comments() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api.github.com/repos/octocat/Hello-World/issues/1347",
                200,
                recorded("issue"),
            ),
            Exchange::new(
                "GET",
                "/api.github.com/repos/octocat/Hello-World/issues/1347/comments",
                200,
                recorded("comments"),
            ),
        ])
        .await;
        let input = json!({"repo": "octocat/Hello-World", "number": 1347});
        let shown = Github
            .call(&pat(&fixture), "get_issue", input)
            .await
            .unwrap();
        assert_eq!(shown["repo"], "octocat/Hello-World");
        assert_eq!(shown["body"], "I'm having a problem with this.");
        assert_eq!(shown["pull_request"], true);
        assert_eq!(shown["labels"], json!(["bug"]));
        assert_eq!(shown["recent_comments"][1]["author"], "hubot");
        let traversal = json!({"repo": "../admin", "number": 1});
        let refused = Github.call(&pat(&fixture), "get_issue", traversal).await;
        assert!(matches!(refused, Err(Error::Invalid(_))));
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].header("authorization"),
            Some("Bearer github_pat_secret")
        );
        assert_eq!(seen[0].header("accept"), Some(JSON));
    }

    #[tokio::test]
    async fn get_file_decodes_text_and_saves_binary_as_a_file() {
        let mut binary = recorded("contents");
        binary["name"] = json!("logo.png");
        binary["content"] = json!("iVBORw0KGgo=");
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api.github.com/repos/octokit/octokit.rb/contents/README.md?ref=v1.0",
                200,
                recorded("contents"),
            ),
            Exchange::new(
                "GET",
                "/api.github.com/repos/octokit/octokit.rb/contents/docs/logo%3F.png",
                200,
                binary,
            ),
        ])
        .await;
        let cx = pat(&fixture);
        let input = json!({"repo": "octokit/octokit.rb", "path": "README.md", "ref": "v1.0"});
        let text = Github.call(&cx, "get_file", input).await.unwrap();
        assert_eq!(
            text["text"],
            "# Hello World\n\nWelcome to the repository.\n"
        );
        let input = json!({"repo": "octokit/octokit.rb", "path": "/docs/logo?.png"});
        let file = Github.call(&cx, "get_file", input).await.unwrap();
        assert_eq!(file[crate::FILE]["name"], "logo.png");
        assert_eq!(file[crate::FILE]["base64"], "iVBORw0KGgo=");
        let input = json!({"repo": "octokit/octokit.rb", "path": "docs/../../secrets"});
        let refused = Github.call(&cx, "get_file", input).await;
        assert!(matches!(refused, Err(Error::Invalid(_))));
        fixture.finish().await;
    }

    #[tokio::test]
    async fn create_issue_and_comment_send_only_what_github_expects() {
        let comment = recorded("comments")[0].clone();
        let fixture = Fixture::start(vec![
            Exchange::new(
                "POST",
                "/api.github.com/repos/octocat/Hello-World/issues",
                201,
                recorded("issue"),
            ),
            Exchange::new(
                "POST",
                "/api.github.com/repos/octocat/Hello-World/issues/1347/comments",
                201,
                comment,
            ),
        ])
        .await;
        let cx = pat(&fixture);
        let input =
            json!({"repo": "octocat/Hello-World", "title": "Found a bug", "labels": ["bug"]});
        let made = Github.call(&cx, "create_issue", input).await.unwrap();
        assert_eq!(
            made,
            json!({"number": 1347, "url": "https://github.com/octocat/Hello-World/issues/1347"})
        );
        let input = json!({"repo": "octocat/Hello-World", "number": 1347, "body": "Me too"});
        let said = Github.call(&cx, "comment", input).await.unwrap();
        assert_eq!(said["id"], 1);
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].json(),
            json!({"title": "Found a bug", "labels": ["bug"]})
        );
        assert_eq!(seen[1].json(), json!({"body": "Me too"}));
    }

    #[tokio::test]
    async fn poll_starts_at_the_newest_notification_and_then_reports_newer_ones() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api.github.com/notifications?all=true&per_page=1",
                200,
                json!([recorded("notifications")[1]]),
            ),
            Exchange::new(
                "GET",
                "/api.github.com/notifications?since=2014-11-06T10%3A00%3A00Z&per_page=50",
                200,
                recorded("notifications"),
            ),
            Exchange::new("GET", "/api.github.com/notifications", 403, json!({})),
        ])
        .await;
        let cx = pat(&fixture);
        let started = Github.poll(&cx, None).await.unwrap();
        assert!(started.events.is_empty());
        let since = started.cursor.unwrap();
        assert_eq!(since, "2014-11-06T10:00:00Z");
        let polled = Github.poll(&cx, Some(&since)).await.unwrap();
        assert_eq!(polled.cursor.as_deref(), Some("2014-11-07T22:01:45Z"));
        assert_eq!(polled.events[0].id, "2@2014-11-07T22:01:45Z");
        assert_eq!(
            polled.events[0].summary,
            "mention in octocat/Hello-World: Greetings"
        );
        let refused = Github
            .poll(&cx, Some("2014-11-07T22:01:45Z"))
            .await
            .unwrap();
        assert!(
            refused.events.is_empty(),
            "an app token cannot read notifications"
        );
        assert_eq!(refused.cursor.as_deref(), Some("2014-11-07T22:01:45Z"));
        fixture.finish().await;
    }
}

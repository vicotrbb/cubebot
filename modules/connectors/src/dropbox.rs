//! Dropbox through the person's own scoped app, signed in with PKCE and an offline refresh token
//! (https://developers.dropbox.com/oauth-guide, https://www.dropbox.com/developers/documentation/http/documentation).
//! Dropbox answers endpoint errors with 409 and an `error_summary` such as `path/not_found/`.
use crate::{
    ATTACH, Account, Answer, Attachment, Auth, Category, Connector, Context, Error, Field,
    FieldKind, Level, Manifest, OAuth2, Risk, Scope, Step, ToolSpec,
};
use serde::Deserialize;
use serde_json::{Value, json};

const API: &str = "https://api.dropboxapi.com/2";
const CONTENT: &str = "https://content.dropboxapi.com/2";
/// The most characters of a text file the model gets.
const LONGEST: usize = 50_000;

pub struct Dropbox;

static MANIFEST: Manifest = Manifest {
    id: "dropbox",
    name: "Dropbox",
    description: "Browse, search, read and upload files in Dropbox through your own Dropbox app.",
    category: Category::Files,
    auth: &[Auth::OAuth2(OAuth2 {
        authorize: "https://www.dropbox.com/oauth2/authorize",
        token: "https://api.dropboxapi.com/oauth2/token",
        device: None,
        // Dropbox revokes with a bearer call to /2/auth/token/revoke, not RFC 7009.
        revoke: None,
        scopes: &[
            Scope {
                scope: "account_info.read",
                product: None,
            },
            Scope {
                scope: "files.metadata.read",
                product: None,
            },
            Scope {
                scope: "files.content.read",
                product: None,
            },
            Scope {
                scope: "files.content.write",
                product: None,
            },
        ],
        // A refresh token; access tokens last about four hours.
        params: &[("token_access_type", "offline")],
        // PKCE stands in for the app secret, at sign-in and on every refresh.
        fields: &[Field {
            key: "client_id",
            label: "App key",
            kind: FieldKind::Text,
            required: true,
            placeholder: "",
            help: "From the app's Settings tab in the Dropbox App Console.",
        }],
    })],
    config: &[],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_folder",
            description: "The files and folders in a folder; pass next from a previous answer for more.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"path":{"type":"string","description":"A folder path such as /Projects, or empty for the top folder."},"limit":{"type":"integer","minimum":1,"maximum":500},"next":{"type":"string","description":"The next cursor of a previous answer; the other inputs are then ignored."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "search",
            description: "Find files and folders by name and content, optionally under a folder.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"query":{"type":"string"},"path":{"type":"string","description":"Only under this folder, such as /Projects."},"limit":{"type":"integer","minimum":1,"maximum":100},"next":{"type":"string","description":"The next cursor of a previous answer."}},"required":["query"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "download",
            description: "A file's text, or the file saved to the workspace when it is not text.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"path":{"type":"string","description":"The file's path such as /Notes/plan.md, or its id:… id."}},"required":["path"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "upload",
            description: "Save text or a workspace file to a path in Dropbox, renamed if the name is taken unless overwrite is true.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"path":{"type":"string","description":"Where to save it, such as /Notes/plan.md."},"text":{"type":"string"},"workspace_file":{"type":"string","description":"A path in the workspace to upload instead of text."},"overwrite":{"type":"boolean"}},"required":["path"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "create_folder",
            description: "Create a folder, with any missing parent folders.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"path":{"type":"string","description":"Such as /Projects/2026."}},"required":["path"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "move",
            description: "Move or rename a file or folder; the destination must not exist.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"from":{"type":"string"},"to":{"type":"string"}},"required":["from","to"],"additionalProperties":false}"#,
        },
    ],
    events: &[],
    level: Level::Write,
    guide: &[
        Step {
            title: "Create a Dropbox app",
            body: "In the Dropbox App Console choose Create app, pick Scoped access, then Full Dropbox (or App folder to confine it to one folder), and name it.",
            link: Some("https://www.dropbox.com/developers/apps"),
            copy: None,
        },
        Step {
            title: "Tick its permissions",
            body: "On the app's Permissions tab tick these scopes and press Submit. Sign-in fails if any is left unticked.",
            link: None,
            copy: Some(
                "account_info.read files.metadata.read files.content.read files.content.write",
            ),
        },
        Step {
            title: "Add the redirect address",
            body: "On the Settings tab, under OAuth 2 › Redirect URIs, add {redirect_uri} exactly and press Add. The app needs no secret: Cubebot signs in with PKCE.",
            link: None,
            copy: Some("{redirect_uri}"),
        },
        Step {
            title: "Copy the app key and sign in",
            body: "Copy the App key from the Settings tab, paste it below, and sign in with the Dropbox account to connect.",
            link: None,
            copy: None,
        },
    ],
    warning: Some(
        "A new app is in development status and allows up to 500 Dropbox users, which is plenty for your own account.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    #[serde(default)]
    path: String,
    limit: Option<u16>,
    next: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: String,
    path: Option<String>,
    limit: Option<u16>,
    next: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Path {
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Upload {
    path: String,
    text: Option<String>,
    #[serde(rename = "workspace_file")]
    file: Option<Attachment>,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Move {
    from: String,
    to: String,
}

impl Connector for Dropbox {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let me = cx
                .json(cx.post(&format!("{API}/users/get_current_account"))?)
                .await?;
            let name = me["email"]
                .as_str()
                .or(me["name"]["display_name"].as_str())
                .unwrap_or("Dropbox account");
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "list_folder" => list(cx, input).await,
                "search" => search(cx, input).await,
                "download" => download(cx, input).await,
                "upload" => upload(cx, input).await,
                "create_folder" => create_folder(cx, input).await,
                "move" => move_entry(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }
}

/// An RPC call; a 409 naming `not_found` is reported as such.
async fn rpc(cx: &Context, route: &str, body: Value) -> Result<Value, Error> {
    let request = cx.post(&format!("{API}/{route}"))?.json(&body);
    cx.json(request).await.map_err(found)
}

fn found(error: Error) -> Error {
    match error {
        Error::Rejected(message) if message.contains("not_found") => Error::NotFound(message),
        other => other,
    }
}

/// A path the API accepts: `/…`, an `id:…` or, where `empty` allows, the top folder.
fn path(text: &str, empty: bool) -> Result<String, Error> {
    let text = text.trim();
    let valid = (empty && text.is_empty())
        || text.starts_with('/')
        || text.starts_with("id:")
        || text.starts_with("rev:");
    match valid && !text.chars().any(char::is_control) {
        // The top folder is "", never "/".
        true => Ok(if text == "/" {
            String::new()
        } else {
            text.into()
        }),
        false => Err(Error::Invalid(format!(
            "{text:?} is not a Dropbox path; paths start with /"
        ))),
    }
}

/// Dropbox-API-Arg is JSON in an HTTP header, so everything past ASCII is escaped.
fn header_json(value: &Value) -> String {
    value
        .to_string()
        .encode_utf16()
        .map(|unit| match unit {
            0x20..=0x7e => char::from(unit as u8).to_string(),
            _ => format!("\\u{unit:04x}"),
        })
        .collect()
}

async fn list(cx: &Context, input: Value) -> Result<Value, Error> {
    let list: List = crate::input(input)?;
    let listed = match list.next {
        Some(cursor) => rpc(cx, "files/list_folder/continue", json!({"cursor": cursor})).await?,
        None => {
            let body = json!({"path": path(&list.path, true)?, "limit": list.limit.unwrap_or(100).clamp(1, 500)});
            rpc(cx, "files/list_folder", body).await?
        }
    };
    let entries: Vec<Value> = listed["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .map(entry)
        .collect();
    Ok(json!({"entries": entries, "next": more(&listed)}))
}

async fn search(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: Search = crate::input(input)?;
    let found = match search.next {
        Some(cursor) => rpc(cx, "files/search/continue_v2", json!({"cursor": cursor})).await?,
        None => {
            let mut options = json!({"max_results": search.limit.unwrap_or(20).clamp(1, 100)});
            if let Some(folder) = &search.path {
                options["path"] = json!(path(folder, true)?);
            }
            let body = json!({"query": search.query, "options": options});
            rpc(cx, "files/search_v2", body).await?
        }
    };
    let matches: Vec<Value> = found["matches"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|found| entry(&found["metadata"]["metadata"]))
        .collect();
    Ok(json!({"matches": matches, "next": more(&found)}))
}

fn more(listed: &Value) -> Value {
    match listed["has_more"] == true {
        true => listed["cursor"].clone(),
        false => Value::Null,
    }
}

/// Text comes back as text; anything else is saved to the workspace.
async fn download(cx: &Context, input: Value) -> Result<Value, Error> {
    let wanted = path(&crate::input::<Path>(input)?.path, false)?;
    let request = cx
        .post(&format!("{CONTENT}/files/download"))?
        .header("dropbox-api-arg", header_json(&json!({"path": wanted})));
    let (bytes, _) = cx.bytes(request).await.map_err(found)?;
    let name = wanted.rsplit('/').next().unwrap_or("file");
    match String::from_utf8(bytes) {
        Ok(text) if !text.contains('\0') => {
            Ok(json!({"path": wanted, "text": clip(&text, LONGEST)}))
        }
        Ok(text) => Ok(crate::file(name, None, text.as_bytes())),
        Err(binary) => Ok(crate::file(name, None, binary.as_bytes())),
    }
}

/// One request: the files/upload endpoint takes up to 150 MiB, past the 25 MiB any upload may be.
async fn upload(cx: &Context, input: Value) -> Result<Value, Error> {
    let upload: Upload = crate::input(input)?;
    let target = path(&upload.path, false)?;
    let bytes = match (upload.text, upload.file) {
        (Some(text), None) => text.into_bytes(),
        (None, Some(file)) => file.bytes()?,
        _ => {
            return Err(Error::Invalid(format!(
                "give exactly one of text and {ATTACH}"
            )));
        }
    };
    let mode = if upload.overwrite { "overwrite" } else { "add" };
    let arg = json!({"path": target, "mode": mode, "autorename": !upload.overwrite});
    let request = cx
        .post(&format!("{CONTENT}/files/upload"))?
        .header("dropbox-api-arg", header_json(&arg))
        .header("content-type", "application/octet-stream")
        .body(bytes);
    Ok(entry(&cx.json(request).await.map_err(found)?))
}

async fn create_folder(cx: &Context, input: Value) -> Result<Value, Error> {
    let folder = path(&crate::input::<Path>(input)?.path, false)?;
    let made = rpc(
        cx,
        "files/create_folder_v2",
        json!({"path": folder, "autorename": false}),
    )
    .await?;
    let mut shown = entry(&made["metadata"]);
    shown["type"] = json!("folder");
    Ok(shown)
}

async fn move_entry(cx: &Context, input: Value) -> Result<Value, Error> {
    let moved: Move = crate::input(input)?;
    let body = json!({"from_path": path(&moved.from, false)?, "to_path": path(&moved.to, false)?, "autorename": false});
    let done = rpc(cx, "files/move_v2", body).await?;
    Ok(entry(&done["metadata"]))
}

fn entry(metadata: &Value) -> Value {
    json!({
        "type": metadata[".tag"],
        "name": metadata["name"],
        "path": metadata["path_display"],
        "id": metadata["id"],
        "size": metadata["size"],
        "modified": metadata["server_modified"],
    })
}

fn clip(text: &str, longest: usize) -> String {
    match text.char_indices().nth(longest) {
        Some((end, _)) => format!("{}\n…[cut short]", &text[..end]),
        None => text.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    fn cx(fixture: &Fixture) -> Context {
        let credential = Credential::Token {
            token: "sl.dropbox-token".into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    fn recorded(name: &str) -> Value {
        let path = format!(
            "{}/fixtures/dropbox/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn list_folder_starts_at_the_top_and_continues_with_the_cursor() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "POST",
                "/api.dropboxapi.com/2/files/list_folder",
                200,
                recorded("list_folder"),
            ),
            Exchange::new(
                "POST",
                "/api.dropboxapi.com/2/files/list_folder/continue",
                200,
                json!({"entries": [], "cursor": "ZtkX9_EHj3x7PMkVuFIhwKYXEpwpLwyxp9vMKomUhllil9q7eWiAu", "has_more": false}),
            ),
        ])
        .await;
        let listed = Dropbox
            .call(&cx(&fixture), "list_folder", json!({"path": "/"}))
            .await
            .unwrap();
        assert_eq!(
            listed["entries"][0]["path"],
            "/Homework/math/Prime_Numbers.txt"
        );
        assert_eq!(listed["entries"][1]["type"], "folder");
        let next = listed["next"].as_str().unwrap().to_owned();
        let rest = Dropbox
            .call(&cx(&fixture), "list_folder", json!({"next": next}))
            .await
            .unwrap();
        assert_eq!(rest["next"], Value::Null);
        let seen = fixture.finish().await;
        assert_eq!(seen[0].json(), json!({"path": "", "limit": 100}));
        assert_eq!(
            seen[0].header("authorization"),
            Some("Bearer sl.dropbox-token")
        );
        assert_eq!(seen[1].json()["cursor"], json!(next));
    }

    #[tokio::test]
    async fn search_reads_nested_metadata_and_scopes_to_a_folder() {
        let fixture = Fixture::start(vec![Exchange::new(
            "POST",
            "/api.dropboxapi.com/2/files/search_v2",
            200,
            recorded("search"),
        )])
        .await;
        let found = Dropbox
            .call(
                &cx(&fixture),
                "search",
                json!({"query": "prime", "path": "/Homework", "limit": 5}),
            )
            .await
            .unwrap();
        assert_eq!(found["matches"][0]["name"], "Prime_Numbers.txt");
        assert_eq!(found["next"], Value::Null);
        let sent = fixture.finish().await[0].json();
        assert_eq!(
            sent,
            json!({"query": "prime", "options": {"max_results": 5, "path": "/Homework"}})
        );
    }

    #[tokio::test]
    async fn download_returns_text_or_a_file_and_escapes_the_header() {
        let fixture = Fixture::start(vec![
            Exchange::raw(
                "POST",
                "/content.dropboxapi.com/2/files/download",
                200,
                "2, 3, 5, 7",
            ),
            Exchange::raw(
                "POST",
                "/content.dropboxapi.com/2/files/download",
                200,
                "\u{0}PNG",
            ),
        ])
        .await;
        let text = Dropbox
            .call(&cx(&fixture), "download", json!({"path": "/Notas/núm.txt"}))
            .await
            .unwrap();
        assert_eq!(text["text"], "2, 3, 5, 7");
        let binary = Dropbox
            .call(
                &cx(&fixture),
                "download",
                json!({"path": "/Photos/logo.png"}),
            )
            .await
            .unwrap();
        assert_eq!(binary[crate::FILE]["name"], "logo.png");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].header("dropbox-api-arg"),
            Some(r#"{"path":"/Notas/n\u00fam.txt"}"#)
        );
    }

    #[tokio::test]
    async fn upload_sends_a_workspace_file_as_bytes_and_refuses_both_sources() {
        let fixture = Fixture::start(vec![Exchange::new(
            "POST",
            "/content.dropboxapi.com/2/files/upload",
            200,
            recorded("list_folder")["entries"][0].clone(),
        )])
        .await;
        let file = serde_json::to_value(Attachment::new("primes.txt", b"2 3 5")).unwrap();
        let input = json!({"path": "/Homework/math/Prime_Numbers.txt", "workspace_file": file});
        let made = Dropbox.call(&cx(&fixture), "upload", input).await.unwrap();
        assert_eq!(made["id"], "id:a4ayc_80_OEAAAAAAAAAXw");
        let both = json!({"path": "/a.txt", "text": "x", "workspace_file": file});
        let refused = Dropbox.call(&cx(&fixture), "upload", both).await;
        assert!(matches!(refused, Err(Error::Invalid(_))));
        let seen = fixture.finish().await;
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].body, b"2 3 5");
        assert_eq!(
            seen[0].header("content-type"),
            Some("application/octet-stream")
        );
        let arg: Value = serde_json::from_str(seen[0].header("dropbox-api-arg").unwrap()).unwrap();
        assert_eq!(
            arg,
            json!({"path": "/Homework/math/Prime_Numbers.txt", "mode": "add", "autorename": true})
        );
    }

    #[tokio::test]
    async fn create_folder_and_move_name_the_result() {
        let metadata = json!({"metadata": {"name": "2026", "path_display": "/Projects/2026", "id": "id:a4ayc_80_OEAAAAAAAAAYa"}});
        let fixture = Fixture::start(vec![
            Exchange::new(
                "POST",
                "/api.dropboxapi.com/2/files/create_folder_v2",
                200,
                metadata.clone(),
            ),
            Exchange::new(
                "POST",
                "/api.dropboxapi.com/2/files/move_v2",
                200,
                json!({"metadata": {".tag": "folder", "name": "2027", "path_display": "/Projects/2027", "id": "id:a4ayc_80_OEAAAAAAAAAYa"}}),
            ),
        ])
        .await;
        let made = Dropbox
            .call(
                &cx(&fixture),
                "create_folder",
                json!({"path": "/Projects/2026"}),
            )
            .await
            .unwrap();
        assert_eq!(
            (made["type"].as_str(), made["path"].as_str()),
            (Some("folder"), Some("/Projects/2026"))
        );
        let moved = Dropbox
            .call(
                &cx(&fixture),
                "move",
                json!({"from": "/Projects/2026", "to": "/Projects/2027"}),
            )
            .await
            .unwrap();
        assert_eq!(moved["path"], "/Projects/2027");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[1].json(),
            json!({"from_path": "/Projects/2026", "to_path": "/Projects/2027", "autorename": false})
        );
    }

    #[tokio::test]
    async fn refusals_map_to_errors_and_bad_input_never_reaches_dropbox() {
        let route = "/api.dropboxapi.com/2/files/list_folder";
        let cases = [
            (
                401,
                json!({"error_summary": "expired_access_token/", "error": {".tag": "expired_access_token"}}),
            ),
            (
                403,
                json!({"error_summary": "missing_scope/", "error": {".tag": "missing_scope"}}),
            ),
            (404, json!({})),
            (
                409,
                json!({"error_summary": "path/not_found/..", "error": {".tag": "path", "path": {".tag": "not_found"}}}),
            ),
            (429, json!({"error_summary": "too_many_requests/"})),
        ];
        let exchanges = cases
            .iter()
            .map(|(status, body)| {
                Exchange::new("POST", route, *status, body.clone()).header("retry-after", "300")
            })
            .collect();
        let fixture = Fixture::start(exchanges).await;
        let mut got = Vec::new();
        for _ in &cases {
            let answer = Dropbox
                .call(&cx(&fixture), "list_folder", json!({"path": "/Gone"}))
                .await;
            got.push(answer.unwrap_err());
        }
        assert_eq!(got[0], Error::Unauthorized);
        assert!(matches!(got[1], Error::Forbidden(_)));
        assert!(matches!(got[2], Error::NotFound(_)));
        assert!(matches!(&got[3], Error::NotFound(message) if message.contains("path/not_found")));
        assert!(matches!(got[4], Error::Unavailable(_)));
        let relative = Dropbox
            .call(&cx(&fixture), "download", json!({"path": "Homework/a.txt"}))
            .await;
        assert!(matches!(relative, Err(Error::Invalid(_))));
        let unknown = Dropbox
            .call(
                &cx(&fixture),
                "move",
                json!({"from": "/a", "to": "/b", "force": true}),
            )
            .await;
        assert!(matches!(unknown, Err(Error::Invalid(_))));
        assert_eq!(fixture.finish().await.len(), cases.len());
    }
}

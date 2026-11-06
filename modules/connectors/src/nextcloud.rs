//! Nextcloud Files over WebDAV with an app password
//! (https://docs.nextcloud.com/server/latest/developer_manual/client_apis/WebDAV/basic.html,
//! search: https://docs.nextcloud.com/server/latest/developer_manual/client_apis/WebDAV/search.html,
//! app passwords: https://docs.nextcloud.com/server/latest/user_manual/en/session_management.html).
//! Paths are relative to the person's files and every request stays on the configured server,
//! through the CalDAV connector's PROPFIND reading and same-site rule.
use crate::{
    Account, Answer, Attachment, Auth, Category, Connector, Context, Error, Field, FieldKind,
    Level, Manifest, PRIVATE_ADDRESS, Risk, Step, ToolSpec,
    dav::{Entry, authed, escape_xml, read},
};
use reqwest::{Method, Url, header};
use serde::Deserialize;
use serde_json::{Value, json};

const LISTED: usize = 200;
const FOUND: usize = 50;
/// The most characters of a text file shown inline.
const LONGEST_TEXT: usize = 50_000;
const PROPS: &str = "<d:prop><d:displayname/><d:getcontenttype/><d:getlastmodified/><d:resourcetype/><oc:size/></d:prop>";

pub struct Nextcloud;

static MANIFEST: Manifest = Manifest {
    id: "nextcloud",
    name: "Nextcloud Files",
    description: "Browse, search, download, upload and organise the files on a Nextcloud server.",
    category: Category::Files,
    auth: &[Auth::Password {
        fields: &[
            Field {
                key: "username",
                label: "Username",
                kind: FieldKind::Text,
                required: true,
                placeholder: "victor",
                help: "Your user id, as in the WebDAV address under Files › Files settings: …/remote.php/dav/files/<username>/.",
            },
            Field {
                key: "password",
                label: "App password",
                kind: FieldKind::Secret,
                required: true,
                placeholder: "xxxxx-xxxxx-xxxxx-xxxxx-xxxxx",
                help: "Made for Cubebot under Settings › Security, not your login password.",
            },
        ],
    }],
    config: &[Field {
        key: "url",
        label: "Server address",
        kind: FieldKind::Url,
        required: true,
        placeholder: "https://cloud.example.com",
        help: "The address you open Nextcloud at.",
    }],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_folder",
            description: "The files and folders in a folder, with sizes, types and when each changed.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"path":{"type":"string","description":"A folder such as Documents/Taxes; empty for the top."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "search",
            description: "Files and folders whose name contains a text, anywhere under a folder.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"name":{"type":"string","description":"Part of the name, such as invoice."},"folder":{"type":"string","description":"Where to look; everywhere when absent."},"limit":{"type":"integer","minimum":1,"maximum":50}},"required":["name"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "download",
            description: "A file's content: text files as text, anything else saved to the workspace.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"path":{"type":"string","description":"Such as Documents/notes.md."}},"required":["path"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "upload",
            description: "Write a workspace file, or text, to a path. Refuses to replace an existing file unless overwrite is true.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"path":{"type":"string","description":"Where to write, with the file name, such as Documents/report.pdf."},"workspace_file":{"type":"string","description":"A file in your workspace to upload."},"text":{"type":"string","description":"Text to write instead of a workspace file."},"overwrite":{"type":"boolean"}},"required":["path"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "create_folder",
            description: "Create a folder; its parent must exist.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "move",
            description: "Move or rename a file or folder. Refuses to replace an existing one unless overwrite is true.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"from":{"type":"string"},"to":{"type":"string","description":"The new path, with the name."},"overwrite":{"type":"boolean"}},"required":["from","to"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "delete",
            description: "Delete a file or folder. Nextcloud keeps it in the trash bin, where it can be restored, unless the trash app is off.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}"#,
        },
    ],
    events: &[],
    level: Level::Write,
    guide: &[
        Step {
            title: "Create an app password",
            body: "In Nextcloud open Settings (your avatar › Settings) › Security. Under Devices & sessions, enter Cubebot as the app name and choose Create new app password, then copy the password and your username shown with it.",
            link: None,
            copy: None,
        },
        Step {
            title: "Enter the server's address",
            body: "Use the address you open Nextcloud at, such as https://cloud.example.com or http://192.168.1.20:8080, then the username and app password.",
            link: None,
            copy: None,
        },
        PRIVATE_ADDRESS,
    ],
    warning: Some(
        "An app password reaches all of your files, not one folder. Revoke it under Settings › Security when you remove the connection.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Folder {
    #[serde(default)]
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    name: String,
    #[serde(default)]
    folder: String,
    limit: Option<usize>,
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
    workspace_file: Option<Attachment>,
    text: Option<String>,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Move {
    from: String,
    to: String,
    #[serde(default)]
    overwrite: bool,
}

impl Connector for Nextcloud {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let home = home(cx)?;
            read(cx, &home, "PROPFIND", home.clone(), "0", &propfind()).await?;
            let name = cx.setting("username").unwrap_or_default();
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
                "move" => rename(cx, input).await,
                "delete" => delete(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }
}

fn propfind() -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">{PROPS}</d:propfind>"#
    )
}

fn server(cx: &Context) -> Result<Url, Error> {
    let address = cx
        .setting("url")
        .ok_or_else(|| Error::Invalid("the connection has no server address".into()))?;
    Url::parse(address).map_err(|_| Error::Invalid("the server address is not valid".into()))
}

fn user(cx: &Context) -> Result<&str, Error> {
    cx.setting("username")
        .ok_or_else(|| Error::Invalid("the connection has no username".into()))
}

/// `<server>/remote.php/dav/files/<user>/`, the root every path is under.
fn home(cx: &Context) -> Result<Url, Error> {
    under(server(cx)?, &["remote.php", "dav", "files", user(cx)?, ""])
}

fn under(mut url: Url, segments: &[&str]) -> Result<Url, Error> {
    url.path_segments_mut()
        .map_err(|_| Error::Invalid("the server address is not valid".into()))?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}

/// A path's segments, refusing any that could climb out of the person's files.
fn segments(path: &str) -> Result<Vec<&str>, Error> {
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    if parts.iter().any(|part| matches!(*part, "." | "..")) || path.contains(['\\', '\0']) {
        return Err(Error::Invalid(format!(
            "{path:?} is not a path in your files"
        )));
    }
    Ok(parts)
}

/// The address of `path` under the person's files.
fn at(cx: &Context, path: &str) -> Result<Url, Error> {
    match segments(path)?.as_slice() {
        [] => home(cx),
        segments => under(home(cx)?, segments),
    }
}

/// A response's path relative to the person's files, decoded.
fn relative(home: &Url, href: &str) -> String {
    let path = decode(href.split(['?', '#']).next().unwrap_or_default());
    let root = decode(home.path());
    path.strip_prefix(&root)
        .unwrap_or(&path)
        .trim_end_matches('/')
        .to_owned()
}

/// Percent-decoding, so `%20` reads as a space; invalid escapes are kept as they are.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = text
            .get(index + 1..index + 3)
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (bytes[index], hex) {
            (b'%', Some(byte)) => {
                decoded.push(byte);
                index += 3;
            }
            (byte, _) => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn item(home: &Url, entry: &Entry) -> Value {
    let folder = entry.is("collection");
    json!({
        "path": relative(home, &entry.href),
        "folder": folder,
        "size": entry.text("size").and_then(|size| size.parse::<u64>().ok()),
        "type": entry.text("getcontenttype").filter(|_| !folder),
        "modified": entry.text("getlastmodified"),
    })
}

async fn list(cx: &Context, input: Value) -> Result<Value, Error> {
    let folder: Folder = crate::input(input)?;
    let home = home(cx)?;
    let url = at(cx, &folder.path)?;
    let (_, found) = read(cx, &home, "PROPFIND", url, "1", &propfind()).await?;
    let shown = segments(&folder.path)?.join("/");
    // The first response is the folder itself.
    let items: Vec<Value> = found
        .iter()
        .map(|entry| item(&home, entry))
        .filter(|item| item["path"] != shown.as_str())
        .collect();
    let more = items.len().saturating_sub(LISTED);
    let items: Vec<Value> = items.into_iter().take(LISTED).collect();
    Ok(json!({"items": items, "more": more}))
}

/// A DAV SEARCH (RFC 5323) on names under a folder.
async fn search(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: Search = crate::input(input)?;
    let limit = search.limit.unwrap_or(FOUND).clamp(1, FOUND);
    let home = home(cx)?;
    let mut scope = vec!["files", user(cx)?];
    scope.extend(segments(&search.folder)?);
    let scope = escape_xml(&format!("/{}", scope.join("/")));
    let name = escape_xml(&search.name.replace(['%', '_'], ""));
    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><d:searchrequest xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns"><d:basicsearch><d:select>{PROPS}</d:select><d:from><d:scope><d:href>{scope}</d:href><d:depth>infinity</d:depth></d:scope></d:from><d:where><d:like><d:prop><d:displayname/></d:prop><d:literal>%{name}%</d:literal></d:like></d:where><d:orderby/><d:limit><d:nresults>{limit}</d:nresults></d:limit></d:basicsearch></d:searchrequest>"#
    );
    let root = under(server(cx)?, &["remote.php", "dav", ""])?;
    let (_, found) = read(cx, &home, "SEARCH", root, "0", &body).await?;
    let items: Vec<Value> = found.iter().take(limit).map(|e| item(&home, e)).collect();
    Ok(json!({"items": items}))
}

async fn download(cx: &Context, input: Value) -> Result<Value, Error> {
    let path = crate::input::<Path>(input)?.path;
    let home = home(cx)?;
    let url = at(cx, &path)?;
    let (bytes, kind) = cx
        .bytes(authed(cx, Method::GET, &home, &url)?)
        .await
        .map_err(sabre)?;
    let name = segments(&path)?
        .last()
        .copied()
        .unwrap_or("file")
        .to_owned();
    let kind = kind.unwrap_or_else(|| "application/octet-stream".into());
    match String::from_utf8(bytes) {
        Ok(text) if textual(&kind) => {
            let clipped = text.chars().count() > LONGEST_TEXT;
            let text: String = text.chars().take(LONGEST_TEXT).collect();
            Ok(json!({"name": name, "type": kind, "text": text, "truncated": clipped}))
        }
        Ok(text) => Ok(crate::file(&name, Some(&kind), text.as_bytes())),
        Err(binary) => Ok(crate::file(&name, Some(&kind), binary.as_bytes())),
    }
}

fn textual(kind: &str) -> bool {
    let kind = kind.split(';').next().unwrap_or_default().trim();
    kind.starts_with("text/") || kind.ends_with("json") || kind.ends_with("xml")
}

async fn upload(cx: &Context, input: Value) -> Result<Value, Error> {
    let upload: Upload = crate::input(input)?;
    let bytes = match (upload.workspace_file, upload.text) {
        (Some(file), None) => file.bytes()?,
        (None, Some(text)) => text.into_bytes(),
        _ => {
            return Err(Error::Invalid(
                "give either workspace_file or text, not both".into(),
            ));
        }
    };
    if segments(&upload.path)?.is_empty() {
        return Err(Error::Invalid("path names the file to write".into()));
    }
    let home = home(cx)?;
    let mut request = authed(cx, Method::PUT, &home, &at(cx, &upload.path)?)?;
    if !upload.overwrite {
        request = request.header(header::IF_NONE_MATCH, "*");
    }
    let size = bytes.len();
    let response = cx.send(request.body(bytes)).await.map_err(sabre)?;
    let etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|etag| etag.to_str().ok());
    Ok(json!({"path": segments(&upload.path)?.join("/"), "size": size, "etag": etag}))
}

async fn create_folder(cx: &Context, input: Value) -> Result<Value, Error> {
    let path = crate::input::<Path>(input)?.path;
    let home = home(cx)?;
    let method = Method::from_bytes(b"MKCOL").map_err(|_| Error::Invalid("MKCOL".into()))?;
    cx.send(authed(cx, method, &home, &at(cx, &path)?)?)
        .await
        .map_err(sabre)?;
    Ok(json!({"created": segments(&path)?.join("/")}))
}

async fn rename(cx: &Context, input: Value) -> Result<Value, Error> {
    let moved: Move = crate::input(input)?;
    let home = home(cx)?;
    let to = at(cx, &moved.to)?;
    let method = Method::from_bytes(b"MOVE").map_err(|_| Error::Invalid("MOVE".into()))?;
    let request = authed(cx, method, &home, &at(cx, &moved.from)?)?
        .header("destination", to.as_str())
        .header("overwrite", if moved.overwrite { "T" } else { "F" });
    cx.send(request).await.map_err(sabre)?;
    Ok(json!({"moved": segments(&moved.from)?.join("/"), "to": segments(&moved.to)?.join("/")}))
}

async fn delete(cx: &Context, input: Value) -> Result<Value, Error> {
    let path = crate::input::<Path>(input)?.path;
    if segments(&path)?.is_empty() {
        return Err(Error::Invalid(
            "refusing to delete all of your files".into(),
        ));
    }
    let home = home(cx)?;
    cx.send(authed(cx, Method::DELETE, &home, &at(cx, &path)?)?)
        .await
        .map_err(sabre)?;
    Ok(json!({"deleted": segments(&path)?.join("/")}))
}

/// Nextcloud answers errors as Sabre XML; the model gets its `s:message` alone.
fn sabre(error: Error) -> Error {
    let message = |text: String| match text.split_once("<s:message>") {
        Some((_, rest)) => {
            let message = rest.split_once("</s:message>").map_or(rest, |(m, _)| m);
            message
                .replace("&amp;", "&")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
        }
        None => text,
    };
    match error {
        Error::Forbidden(text) => Error::Forbidden(message(text)),
        Error::NotFound(text) => Error::NotFound(message(text)),
        Error::Rejected(text) => Error::Rejected(message(text)),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };
    use base64::Engine;

    const PASSWORD: &str = "xxxxx-xxxxx-xxxxx-xxxxx-xxxxx";
    const HOME: &str = "/remote.php/dav/files/admin/";

    fn cx(fixture: &Fixture) -> Context {
        let credential = Credential::Fields {
            fields: [("password".into(), PASSWORD.into())].into(),
        };
        let config = json!({"url": fixture.origin(), "username": "admin"});
        Context::new(config, credential).with_origin(&fixture.origin())
    }

    /// Answers recorded from Nextcloud 35.0.2 with its default files.
    fn recorded(name: &str) -> String {
        let path = format!(
            "{}/fixtures/nextcloud/{name}.xml",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(path).unwrap()
    }

    #[tokio::test]
    async fn list_folder_names_files_and_folders_without_the_folder_itself() {
        let fixture = Fixture::start(vec![Exchange::raw(
            "PROPFIND",
            HOME,
            207,
            &recorded("list"),
        )])
        .await;
        let listed = Nextcloud
            .call(&cx(&fixture), "list_folder", json!({}))
            .await
            .unwrap();
        let items = listed["items"].as_array().unwrap();
        assert!(items.iter().all(|item| item["path"] != ""));
        assert_eq!(
            items[0],
            json!({"path": "Documents", "folder": true, "size": 1108446, "type": null, "modified": "Fri, 06 Nov 2026 02:03:16 GMT"})
        );
        let manual = items
            .iter()
            .find(|item| item["path"] == "Nextcloud Manual.pdf");
        assert_eq!(manual.unwrap()["type"], "application/pdf");
        assert_eq!(listed["more"], 0);
        let seen = fixture.finish().await;
        let basic = base64::engine::general_purpose::STANDARD.encode(format!("admin:{PASSWORD}"));
        assert_eq!(
            seen[0].header("authorization"),
            Some(format!("Basic {basic}").as_str())
        );
        assert_eq!(seen[0].header("depth"), Some("1"));
    }

    #[tokio::test]
    async fn search_asks_for_names_containing_the_text_under_a_folder() {
        let fixture = Fixture::start(vec![Exchange::raw(
            "SEARCH",
            "/remote.php/dav/",
            207,
            &recorded("search"),
        )])
        .await;
        let input = json!({"name": "man%ual", "folder": "Documents & Co", "limit": 5});
        let found = Nextcloud
            .call(&cx(&fixture), "search", input)
            .await
            .unwrap();
        assert_eq!(found["items"][0]["path"], "Nextcloud Manual.pdf");
        assert_eq!(found["items"][0]["size"], 30411524);
        let sent = fixture.finish().await[0].text();
        assert!(
            sent.contains("<d:href>/files/admin/Documents &amp; Co</d:href>"),
            "{sent}"
        );
        assert!(sent.contains("<d:literal>%manual%</d:literal>"), "{sent}");
        assert!(sent.contains("<d:nresults>5</d:nresults>"), "{sent}");
    }

    #[tokio::test]
    async fn download_answers_text_inline_and_anything_else_as_a_file() {
        let fixture = Fixture::start(vec![
            Exchange::raw(
                "GET",
                "/remote.php/dav/files/admin/Notes/Readme%20(1).md",
                200,
                "# Hello",
            )
            .header("content-type", "text/markdown; charset=utf-8"),
            Exchange::raw(
                "GET",
                "/remote.php/dav/files/admin/scan.pdf",
                200,
                "%PDF-1.7",
            )
            .header("content-type", "application/pdf"),
        ])
        .await;
        let cx = cx(&fixture);
        let text = Nextcloud
            .call(&cx, "download", json!({"path": "/Notes/Readme (1).md"}))
            .await
            .unwrap();
        assert_eq!(text["text"], "# Hello");
        assert_eq!(text["name"], "Readme (1).md");
        let pdf = Nextcloud
            .call(&cx, "download", json!({"path": "scan.pdf"}))
            .await
            .unwrap();
        assert_eq!(pdf[crate::FILE]["name"], "scan.pdf");
        assert_eq!(pdf[crate::FILE]["content_type"], "application/pdf");
        fixture.finish().await;
    }

    #[tokio::test]
    async fn writes_never_replace_unless_asked_and_stay_in_the_persons_files() {
        let fixture = Fixture::start(vec![
            Exchange::raw("PUT", "/remote.php/dav/files/admin/Notes/todo.md", 201, "")
                .header("etag", "\"5f1c\""),
            Exchange::raw("PUT", "/remote.php/dav/files/admin/Notes/scan.pdf", 204, ""),
            Exchange::raw(
                "PUT",
                "/remote.php/dav/files/admin/Notes/todo.md",
                412,
                &recorded("precondition"),
            ),
            Exchange::raw("MKCOL", "/remote.php/dav/files/admin/Archive", 201, ""),
            Exchange::raw("MOVE", "/remote.php/dav/files/admin/Notes/todo.md", 201, ""),
            Exchange::raw("DELETE", "/remote.php/dav/files/admin/Archive", 204, ""),
        ])
        .await;
        let cx = cx(&fixture);
        let call = |tool: &'static str, input: Value| Nextcloud.call(&cx, tool, input);
        let made = call("upload", json!({"path": "Notes/todo.md", "text": "milk"}))
            .await
            .unwrap();
        assert_eq!(
            made,
            json!({"path": "Notes/todo.md", "size": 4, "etag": "\"5f1c\""})
        );
        let file = crate::Attachment::new("scan.pdf", b"%PDF");
        let input = json!({"path": "Notes/scan.pdf", "workspace_file": file, "overwrite": true});
        call("upload", input).await.unwrap();
        let again = call("upload", json!({"path": "Notes/todo.md", "text": "eggs"})).await;
        let exists =
            "An If-None-Match header was specified, but the ETag matched (or * was specified).";
        assert_eq!(again, Err(Error::Rejected(exists.into())));
        call("create_folder", json!({"path": "Archive"}))
            .await
            .unwrap();
        let input = json!({"from": "Notes/todo.md", "to": "Archive/todo.md"});
        call("move", input).await.unwrap();
        call("delete", json!({"path": "Archive/"})).await.unwrap();
        let refused = [
            (
                "upload",
                json!({"path": "a.md", "text": "x", "workspace_file": file}),
            ),
            ("upload", json!({"path": "", "text": "x"})),
            ("delete", json!({"path": "/"})),
            ("download", json!({"path": "Notes/../../other/secret"})),
            ("list_folder", json!({"folder": "Notes"})),
        ];
        for (tool, input) in refused {
            let refused = call(tool, input).await;
            assert!(
                matches!(refused, Err(Error::Invalid(_))),
                "{tool}: {refused:?}"
            );
        }
        let origin = fixture.origin();
        let seen = fixture.finish().await;
        assert_eq!(seen[0].header("if-none-match"), Some("*"));
        assert_eq!(seen[0].text(), "milk");
        assert_eq!(seen[1].header("if-none-match"), None);
        assert_eq!(seen[1].body, b"%PDF");
        let destination = format!("{origin}remote.php/dav/files/admin/Archive/todo.md");
        assert_eq!(seen[4].header("destination"), Some(destination.as_str()));
        assert_eq!(seen[4].header("overwrite"), Some("F"));
    }

    #[tokio::test]
    async fn refusals_map_to_errors() {
        let cases = [
            (401, Error::Unauthorized),
            (
                403,
                Error::Forbidden("the server answered 403 Forbidden".into()),
            ),
            (
                404,
                Error::NotFound("the server answered 404 Not Found".into()),
            ),
            (
                429,
                Error::Unavailable("the server answered 429 Too Many Requests".into()),
            ),
        ];
        for (status, expected) in cases {
            let fixture = Fixture::start(vec![
                Exchange::raw("PROPFIND", "/remote.php/dav/files/admin/Gone", status, "")
                    .header("retry-after", "3600"),
            ])
            .await;
            let listed = Nextcloud
                .call(&cx(&fixture), "list_folder", json!({"path": "Gone"}))
                .await;
            assert_eq!(listed, Err(expected), "{status}");
            fixture.finish().await;
        }
    }

    /// Every tool against a real server: set `CUBEBOT_LIVE_NEXTCLOUD_URL`, `_USERNAME` and
    /// `_PASSWORD` (an app password) and run
    /// `cargo test -p cubebot-connectors live_nextcloud -- --ignored`. It writes, then deletes,
    /// a `Cubebot live test` folder.
    #[tokio::test]
    #[ignore = "needs a Nextcloud server"]
    async fn live_nextcloud() {
        let env = |key: &str| std::env::var(format!("CUBEBOT_LIVE_NEXTCLOUD_{key}")).unwrap();
        let credential = Credential::Fields {
            fields: [("password".into(), env("PASSWORD"))].into(),
        };
        let config = json!({"url": env("URL"), "username": env("USERNAME")});
        let cx = Context::new(config, credential);
        println!("check: {:?}", Nextcloud.check(&cx).await.unwrap());
        let call = |tool: &'static str, input: Value| {
            let cx = &cx;
            async move {
                let answer = Nextcloud.call(cx, tool, input).await;
                let shown = format!("{answer:?}");
                println!("{tool}: {}", shown.chars().take(600).collect::<String>());
                answer
            }
        };
        let folder = "Cubebot live test";
        call("create_folder", json!({"path": folder}))
            .await
            .unwrap();
        let notes = format!("{folder}/Notes (1) & more.md");
        call("upload", json!({"path": notes, "text": "# Live\nhello"}))
            .await
            .unwrap();
        let again = call("upload", json!({"path": notes, "text": "no"})).await;
        assert!(matches!(again, Err(Error::Rejected(_))));
        let file = crate::Attachment::new("pixel.bin", &[0, 159, 146, 150]);
        let binary = format!("{folder}/pixel.bin");
        let input = json!({"path": binary, "workspace_file": file, "overwrite": true});
        call("upload", input).await.unwrap();
        let listed = call("list_folder", json!({"path": folder})).await.unwrap();
        assert_eq!(listed["items"].as_array().unwrap().len(), 2);
        let found = call("search", json!({"name": "NOTES (1)", "folder": folder}))
            .await
            .unwrap();
        assert_eq!(found["items"][0]["path"], notes);
        let text = call("download", json!({"path": notes})).await.unwrap();
        assert_eq!(text["text"], "# Live\nhello");
        let bytes = call("download", json!({"path": binary})).await.unwrap();
        assert_eq!(bytes[crate::FILE]["base64"], "AJ+Slg==");
        let renamed = format!("{folder}/Renamed.md");
        call("move", json!({"from": notes, "to": renamed}))
            .await
            .unwrap();
        let missing = call("download", json!({"path": notes})).await;
        assert!(matches!(missing, Err(Error::NotFound(_))));
        call("delete", json!({"path": folder})).await.unwrap();
    }
}

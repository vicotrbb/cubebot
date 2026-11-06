//! Google Drive (https://developers.google.com/drive/api/reference/rest/v3).
use super::{LONGEST_TEXT, at, clip, id};
use crate::{Context, Error};
use serde::Deserialize;
use serde_json::{Value, json};

const FILES: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
const LISTED: &str = "id,name,mimeType,modifiedTime,webViewLink";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    name: Option<String>,
    query: Option<String>,
    limit: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    file: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Upload {
    name: String,
    text: String,
    mime_type: Option<String>,
    folder: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Share {
    file: String,
    email: String,
    role: String,
    message: Option<String>,
}

pub(super) async fn call(cx: &Context, name: &str, input: Value) -> Result<Value, Error> {
    match name {
        "search" => search(cx, input).await,
        "read" => read(cx, input).await,
        "upload" => upload(cx, input).await,
        "share" => share(cx, input).await,
        _ => Err(Error::UnknownTool),
    }
}

async fn search(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: Search = crate::input(input)?;
    let query = match (search.query, search.name) {
        (Some(query), _) => query,
        (None, Some(name)) => {
            let quoted = name.replace('\\', "\\\\").replace('\'', "\\'");
            format!("name contains '{quoted}' and trashed = false")
        }
        (None, None) => "trashed = false".into(),
    };
    let limit = search.limit.unwrap_or(20).clamp(1, 100).to_string();
    let request = cx.get(FILES)?.query(&[
        ("q", query.as_str()),
        ("pageSize", &limit),
        ("orderBy", "modifiedTime desc"),
        ("fields", &format!("files({LISTED})")),
        ("supportsAllDrives", "true"),
        ("includeItemsFromAllDrives", "true"),
    ]);
    let found = cx.json(request).await?;
    Ok(json!({"files": found["files"]}))
}

/// Google's own formats are exported as text; other files come down as they are, as text when
/// they are text and as a workspace file otherwise.
async fn read(cx: &Context, input: Value) -> Result<Value, Error> {
    let file = id(&crate::input::<Read>(input)?.file).to_owned();
    let request = cx.get(&at(FILES, &[&file]))?.query(&[
        ("fields", "id,name,mimeType,size"),
        ("supportsAllDrives", "true"),
    ]);
    let meta = cx.json(request).await?;
    let name = meta["name"].as_str().unwrap_or("file");
    let kind = meta["mimeType"].as_str().unwrap_or_default();
    let request = match export(kind) {
        Some(format) => cx
            .get(&at(FILES, &[&file, "export"]))?
            .query(&[("mimeType", format)]),
        None if kind.starts_with("application/vnd.google-apps.") => {
            return Err(Error::Invalid(format!("a {kind} has no content to read")));
        }
        None => cx
            .get(&at(FILES, &[&file]))?
            .query(&[("alt", "media"), ("supportsAllDrives", "true")]),
    };
    let (bytes, sent) = cx.bytes(request).await?;
    let kind = sent.as_deref().unwrap_or(kind);
    match String::from_utf8(bytes) {
        Ok(text) if textual(kind) => {
            Ok(json!({"name": name, "type": kind, "text": clip(&text, LONGEST_TEXT)}))
        }
        Ok(text) => Ok(crate::file(name, Some(kind), text.as_bytes())),
        Err(binary) => Ok(crate::file(name, Some(kind), binary.as_bytes())),
    }
}

fn export(kind: &str) -> Option<&'static str> {
    match kind.strip_prefix("application/vnd.google-apps.")? {
        "document" | "presentation" => Some("text/plain"),
        "spreadsheet" => Some("text/csv"),
        "drawing" => Some("image/png"),
        _ => None,
    }
}

fn textual(kind: &str) -> bool {
    let kind = kind.split(';').next().unwrap_or_default().trim();
    kind.starts_with("text/") || kind.ends_with("json") || kind.ends_with("xml")
}

/// A one-request multipart upload: the metadata, then the content.
async fn upload(cx: &Context, input: Value) -> Result<Value, Error> {
    let upload: Upload = crate::input(input)?;
    let kind = upload.mime_type.as_deref().unwrap_or("text/plain");
    if kind.contains(['\r', '\n']) {
        return Err(Error::Invalid("mime_type is one line".into()));
    }
    let mut meta = json!({"name": upload.name, "mimeType": kind});
    if let Some(folder) = &upload.folder {
        meta["parents"] = json!([id(folder)]);
    }
    let mut boundary = String::from("cubebot-upload");
    while upload.text.contains(&boundary) {
        boundary.push('x');
    }
    let body = format!(
        "--{boundary}\r\ncontent-type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\ncontent-type: {kind}\r\n\r\n{}\r\n--{boundary}--\r\n",
        upload.text
    );
    let request = cx
        .post(UPLOAD)?
        .query(&[
            ("uploadType", "multipart"),
            ("fields", LISTED),
            ("supportsAllDrives", "true"),
        ])
        .header(
            "content-type",
            format!("multipart/related; boundary={boundary}"),
        )
        .body(body);
    cx.json(request).await
}

/// Google emails the person the link, so this reaches someone.
async fn share(cx: &Context, input: Value) -> Result<Value, Error> {
    let share: Share = crate::input(input)?;
    let body = json!({"type": "user", "role": share.role, "emailAddress": share.email});
    let mut query = vec![
        ("sendNotificationEmail", "true"),
        ("supportsAllDrives", "true"),
    ];
    if let Some(message) = &share.message {
        query.push(("emailMessage", message));
    }
    let path = at(FILES, &[id(&share.file), "permissions"]);
    let made = cx.json(cx.post(&path)?.query(&query).json(&body)).await?;
    Ok(json!({"permission": made["id"], "role": made["role"], "email": share.email}))
}

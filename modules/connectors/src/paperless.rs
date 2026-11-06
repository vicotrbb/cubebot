//! Paperless-ngx's REST API with an API token (https://docs.paperless-ngx.com/api/). Every
//! request goes to the address the person entered. The API version is left to the server's
//! default: the fields read here (ids, titles, tag ids, content) are the same in versions 9 and 10.
use crate::{
    Account, Answer, Attachment, Auth, Category, Connector, Context, Error, Field, FieldKind,
    Level, Manifest, PRIVATE_ADDRESS, Risk, Step, ToolSpec,
};
use reqwest::{Method, RequestBuilder};
use serde::Deserialize;
use serde_json::{Value, json};

const FOUND: u32 = 25;
const MOST_FOUND: u32 = 100;
/// The most characters of a document's text shown.
const LONGEST_CONTENT: usize = 50_000;
const LONGEST_HIT: usize = 300;
const LISTED: &str = "id,title,created,added,correspondent,document_type,tags,original_file_name";

pub struct Paperless;

static MANIFEST: Manifest = Manifest {
    id: "paperless",
    name: "Paperless-ngx",
    description: "Search, read, tag and add documents in a Paperless-ngx archive.",
    category: Category::Files,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "API token",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "",
            help: "From My Profile in Paperless-ngx's user menu.",
        }],
    }],
    config: &[Field {
        key: "url",
        label: "Server address",
        kind: FieldKind::Url,
        required: true,
        placeholder: "http://paperless.local:8000",
        help: "The address you open Paperless-ngx at.",
    }],
    products: &[],
    tools: &[
        ToolSpec {
            name: "search_documents",
            description: "Documents matching a full-text query and having all the given tag ids, newest first without a query. Pages of up to 100.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"query":{"type":"string","description":"Full-text search, such as electricity invoice or created:[2026 to 2027]."},"tags":{"type":"array","items":{"type":"integer"},"description":"Tag ids from list_tags; documents must have all of them."},"page":{"type":"integer","minimum":1},"limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_document",
            description: "A document's details and its OCR text.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"id":{"type":"integer"}},"required":["id"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "download_document",
            description: "Save a document's file to the workspace: the archived PDF, or the file as uploaded with original.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"id":{"type":"integer"},"original":{"type":"boolean"}},"required":["id"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_tags",
            description: "Tags with their ids and how many documents have each, optionally those whose name contains a text.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"name":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "edit_tags",
            description: "Add and remove tags, by id, on documents.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"documents":{"type":"array","items":{"type":"integer"},"minItems":1},"add":{"type":"array","items":{"type":"integer"}},"remove":{"type":"array","items":{"type":"integer"}}},"required":["documents"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "upload_document",
            description: "Add a workspace file to Paperless, optionally titled and tagged. It is processed in the background; the answer is the task id.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"workspace_file":{"type":"string","description":"A file in your workspace, such as a PDF or an image."},"title":{"type":"string"},"tags":{"type":"array","items":{"type":"integer"}}},"required":["workspace_file"],"additionalProperties":false}"#,
        },
    ],
    events: &[],
    level: Level::Write,
    guide: &[
        Step {
            title: "Make an API token",
            body: "In Paperless-ngx open the user menu at the top right and choose My Profile. Next to API Auth Token choose the circular arrow button to generate one, then copy it.",
            link: None,
            copy: None,
        },
        Step {
            title: "Enter the server's address",
            body: "Use the address you open Paperless-ngx at, such as http://paperless.local:8000 or http://192.168.1.20:8000, then paste the token.",
            link: None,
            copy: None,
        },
        PRIVATE_ADDRESS,
    ],
    warning: Some(
        "The token acts as your Paperless-ngx user and sees every document that user can. Generating a new token in My Profile replaces the old one and breaks this connection.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: Option<String>,
    #[serde(default)]
    tags: Vec<u64>,
    page: Option<u32>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    id: u64,
    #[serde(default)]
    original: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tags {
    name: Option<String>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    documents: Vec<u64>,
    #[serde(default)]
    add: Vec<u64>,
    #[serde(default)]
    remove: Vec<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Upload {
    workspace_file: Attachment,
    title: Option<String>,
    #[serde(default)]
    tags: Vec<u64>,
}

impl Connector for Paperless {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let settings = cx.json(api(cx, Method::GET, "ui_settings/")?).await?;
            let name = settings["user"]["username"]
                .as_str()
                .unwrap_or("Paperless-ngx");
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "search_documents" => search(cx, input).await,
                "read_document" => read(cx, input).await,
                "download_document" => download(cx, input).await,
                "list_tags" => tags(cx, input).await,
                "edit_tags" => edit(cx, input).await,
                "upload_document" => upload(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }
}

/// A request to `/api/<path>` on the configured server, with the token.
fn api(cx: &Context, method: Method, path: &str) -> Result<RequestBuilder, Error> {
    let server = cx
        .setting("url")
        .ok_or_else(|| Error::Invalid("the connection has no server address".into()))?;
    let url = format!("{}/api/{path}", server.trim_end_matches('/'));
    Ok(cx
        .request(method, &url)?
        .header("authorization", format!("Token {}", cx.token()?))
        .header("accept", "application/json"))
}

fn clip(text: &str, most: usize) -> (String, bool) {
    match text.char_indices().nth(most) {
        Some((end, _)) => (format!("{}…", &text[..end]), true),
        None => (text.to_owned(), false),
    }
}

fn joined(ids: &[u64]) -> String {
    ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",")
}

fn listed(document: &Value) -> Value {
    let mut shown = json!({
        "id": document["id"],
        "title": document["title"],
        "created": document["created"],
        "added": document["added"],
        "correspondent": document["correspondent"],
        "document_type": document["document_type"],
        "tags": document["tags"],
        "file": document["original_file_name"],
    });
    if let Some(hit) = document["__search_hit__"]["highlights"]
        .as_str()
        .filter(|hit| !hit.is_empty())
    {
        shown["match"] = json!(clip(hit, LONGEST_HIT).0);
    }
    shown
}

async fn search(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: Search = crate::input(input)?;
    let page = search.page.unwrap_or(1).max(1);
    let limit = search.limit.unwrap_or(FOUND).clamp(1, MOST_FOUND);
    let mut query = vec![
        ("page", page.to_string()),
        ("page_size", limit.to_string()),
        ("fields", format!("{LISTED},__search_hit__")),
    ];
    match search.query.filter(|text| !text.trim().is_empty()) {
        Some(text) => query.push(("query", text)),
        None => query.push(("ordering", "-created".into())),
    }
    if !search.tags.is_empty() {
        query.push(("tags__id__all", joined(&search.tags)));
    }
    let found = cx
        .json(api(cx, Method::GET, "documents/")?.query(&query))
        .await?;
    let documents: Vec<Value> = found["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(listed)
        .collect();
    let next = found["next"].is_string().then_some(page + 1);
    Ok(json!({"documents": documents, "count": found["count"], "next_page": next}))
}

async fn read(cx: &Context, input: Value) -> Result<Value, Error> {
    let id = crate::input::<Document>(input)?.id;
    let document = cx
        .json(api(cx, Method::GET, &format!("documents/{id}/"))?)
        .await?;
    let (content, truncated) = clip(
        document["content"].as_str().unwrap_or_default(),
        LONGEST_CONTENT,
    );
    let mut shown = listed(&document);
    shown["archive_serial_number"] = document["archive_serial_number"].clone();
    shown["page_count"] = document["page_count"].clone();
    shown["content"] = json!(content);
    shown["truncated"] = json!(truncated);
    Ok(shown)
}

/// The archived PDF unless the original is asked for or there is none, named as Paperless names it.
async fn download(cx: &Context, input: Value) -> Result<Value, Error> {
    let document: Document = crate::input(input)?;
    let id = document.id;
    let fields = [("fields", "original_file_name,archived_file_name")];
    let names = cx
        .json(api(cx, Method::GET, &format!("documents/{id}/"))?.query(&fields))
        .await?;
    let archived = names["archived_file_name"]
        .as_str()
        .filter(|_| !document.original);
    let name = archived
        .or(names["original_file_name"].as_str())
        .unwrap_or("document");
    let original = if archived.is_some() { "false" } else { "true" };
    let request = api(cx, Method::GET, &format!("documents/{id}/download/"))?
        .query(&[("original", original)]);
    let (bytes, kind) = cx.bytes(request).await?;
    Ok(crate::file(name, kind.as_deref(), &bytes))
}

async fn tags(cx: &Context, input: Value) -> Result<Value, Error> {
    let tags: Tags = crate::input(input)?;
    let limit = tags.limit.unwrap_or(MOST_FOUND).clamp(1, MOST_FOUND);
    let mut query = vec![("page_size", limit.to_string())];
    query.extend(tags.name.map(|name| ("name__icontains", name)));
    let found = cx
        .json(api(cx, Method::GET, "tags/")?.query(&query))
        .await?;
    let shown: Vec<Value> = found["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(
            |tag| json!({"id": tag["id"], "name": tag["name"], "documents": tag["document_count"]}),
        )
        .collect();
    Ok(json!({"tags": shown, "count": found["count"]}))
}

async fn edit(cx: &Context, input: Value) -> Result<Value, Error> {
    let edit: Edit = crate::input(input)?;
    if edit.documents.is_empty() || (edit.add.is_empty() && edit.remove.is_empty()) {
        return Err(Error::Invalid(
            "name documents and tags to add or remove".into(),
        ));
    }
    let body = json!({
        "documents": edit.documents,
        "method": "modify_tags",
        "parameters": {"add_tags": edit.add, "remove_tags": edit.remove},
    });
    cx.json(api(cx, Method::POST, "documents/bulk_edit/")?.json(&body))
        .await?;
    Ok(json!({"documents": edit.documents, "added": edit.add, "removed": edit.remove}))
}

/// A `multipart/form-data` body (RFC 7578) built by hand, as the HTTP client is built without it.
async fn upload(cx: &Context, input: Value) -> Result<Value, Error> {
    let upload: Upload = crate::input(input)?;
    let bytes = upload.workspace_file.bytes()?;
    let mut fields: Vec<(&str, String)> = upload
        .tags
        .iter()
        .map(|tag| ("tags", tag.to_string()))
        .collect();
    fields.extend(upload.title.map(|title| ("title", title)));
    let mut boundary = format!("cubebot-{:x}", crate::dav::now_nanos());
    let inside = |boundary: &str, fields: &[(&str, String)]| {
        bytes
            .windows(boundary.len())
            .any(|part| part == boundary.as_bytes())
            || fields.iter().any(|(_, value)| value.contains(boundary))
    };
    while inside(&boundary, &fields) {
        boundary.push('x');
    }
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend(
            format!(
                "--{boundary}\r\ncontent-disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .bytes(),
        );
    }
    // Quotes and line breaks would end the header early.
    let file: String = upload
        .workspace_file
        .name
        .chars()
        .map(|c| if c == '"' || c.is_control() { '_' } else { c })
        .collect();
    body.extend(format!("--{boundary}\r\ncontent-disposition: form-data; name=\"document\"; filename=\"{file}\"\r\ncontent-type: application/octet-stream\r\n\r\n").bytes());
    body.extend(bytes);
    body.extend(format!("\r\n--{boundary}--\r\n").bytes());
    let request = api(cx, Method::POST, "documents/post_document/")?
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body);
    let task = cx.json(request).await?;
    Ok(
        json!({"task": task, "status": "queued; it appears in search once Paperless has processed it"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    const TOKEN: &str = "0123456789abcdef0123456789abcdef01234567";

    fn cx(fixture: &Fixture) -> Context {
        let credential = Credential::Fields {
            fields: [("token".into(), TOKEN.into())].into(),
        };
        Context::new(json!({"url": fixture.origin()}), credential).with_origin(&fixture.origin())
    }

    /// Answers recorded from Paperless-ngx 3.4.2 (API version 10).
    fn recorded(name: &str) -> Value {
        let path = format!(
            "{}/fixtures/paperless/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn search_documents_sends_the_query_tags_and_page_and_keeps_the_match() {
        let mut page = recorded("search");
        page["next"] = json!("http://paperless.local/api/documents/?page=3");
        let fixture = Fixture::start(vec![
            Exchange::new("GET", "/api/documents/", 200, page),
            Exchange::new("GET", "/api/documents/", 200, recorded("search")),
        ])
        .await;
        let cx = cx(&fixture);
        let input = json!({"query": "tax", "tags": [3, 1], "page": 2, "limit": 10});
        let found = Paperless
            .call(&cx, "search_documents", input)
            .await
            .unwrap();
        assert_eq!(
            found["documents"][0],
            json!({"id": 2, "title": "Tax assessment 2025", "created": "2026-11-06", "added": "2026-11-06T02:11:04.130566Z", "correspondent": null, "document_type": null, "tags": [3], "file": "tax.pdf", "match": "<b>Tax</b> assessment 2025 Amount 1234 EUR"})
        );
        assert_eq!(found["next_page"], 3);
        let newest = Paperless.call(&cx, "search_documents", json!({})).await;
        assert_eq!(newest.unwrap()["next_page"], Value::Null);
        let seen = fixture.finish().await;
        let fields = "fields=id%2Ctitle%2Ccreated%2Cadded%2Ccorrespondent%2Cdocument_type%2Ctags%2Coriginal_file_name%2C__search_hit__";
        assert_eq!(
            seen[0].target,
            format!("/api/documents/?page=2&page_size=10&{fields}&query=tax&tags__id__all=3%2C1")
        );
        assert!(seen[1].target.ends_with("&ordering=-created"));
        let token = format!("Token {TOKEN}");
        assert_eq!(seen[0].header("authorization"), Some(token.as_str()));
    }

    #[tokio::test]
    async fn read_document_gives_details_and_text_and_download_names_the_archived_pdf() {
        let fixture = Fixture::start(vec![
            Exchange::new("GET", "/api/documents/2/", 200, recorded("document")),
            Exchange::new("GET", "/api/documents/2/", 200, recorded("document")),
            Exchange::raw("GET", "/api/documents/2/download/", 200, "%PDF-1.4")
                .header("content-type", "application/pdf"),
        ])
        .await;
        let cx = cx(&fixture);
        let read = Paperless
            .call(&cx, "read_document", json!({"id": 2}))
            .await
            .unwrap();
        assert_eq!(read["content"], "Tax assessment 2025 Amount 1234 EUR");
        assert_eq!(
            (&read["page_count"], &read["truncated"]),
            (&json!(1), &json!(false))
        );
        let file = Paperless
            .call(&cx, "download_document", json!({"id": 2}))
            .await
            .unwrap();
        assert_eq!(
            file[crate::FILE]["name"],
            "2026-11-06 Tax assessment 2025.pdf"
        );
        assert_eq!(file[crate::FILE]["content_type"], "application/pdf");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[1].target,
            "/api/documents/2/?fields=original_file_name%2Carchived_file_name"
        );
        assert_eq!(seen[2].target, "/api/documents/2/download/?original=false");
    }

    #[tokio::test]
    async fn tags_are_listed_edited_in_bulk_and_given_to_an_upload() {
        let fixture = Fixture::start(vec![
            Exchange::new("GET", "/api/tags/", 200, recorded("tags")),
            Exchange::new(
                "POST",
                "/api/documents/bulk_edit/",
                200,
                json!({"result": "OK"}),
            ),
            Exchange::new(
                "POST",
                "/api/documents/post_document/",
                200,
                json!("08e75d17-a5f3-48aa-af68-00bb6e98f6e7"),
            ),
        ])
        .await;
        let cx = cx(&fixture);
        let tags = Paperless
            .call(&cx, "list_tags", json!({"name": "tax"}))
            .await
            .unwrap();
        assert_eq!(
            tags["tags"][1],
            json!({"id": 3, "name": "Taxes", "documents": 1})
        );
        let input = json!({"documents": [2], "add": [1], "remove": [3]});
        Paperless.call(&cx, "edit_tags", input).await.unwrap();
        let file = crate::Attachment::new("tax \"2025\".pdf", b"%PDF-1.4");
        let input = json!({"workspace_file": file, "title": "Tax assessment", "tags": [3]});
        let uploaded = Paperless.call(&cx, "upload_document", input).await.unwrap();
        assert_eq!(uploaded["task"], "08e75d17-a5f3-48aa-af68-00bb6e98f6e7");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].target,
            "/api/tags/?page_size=100&name__icontains=tax"
        );
        assert_eq!(
            seen[1].json(),
            json!({"documents": [2], "method": "modify_tags", "parameters": {"add_tags": [1], "remove_tags": [3]}})
        );
        let kind = seen[2].header("content-type").unwrap();
        let boundary = kind.strip_prefix("multipart/form-data; boundary=").unwrap();
        let body = seen[2].text();
        assert!(body.starts_with(&format!(
            "--{boundary}\r\ncontent-disposition: form-data; name=\"tags\"\r\n\r\n3\r\n"
        )));
        assert!(body.contains("name=\"title\"\r\n\r\nTax assessment\r\n"));
        assert!(body.contains("name=\"document\"; filename=\"tax _2025_.pdf\""));
        assert!(body.ends_with(&format!("%PDF-1.4\r\n--{boundary}--\r\n")));
    }

    #[tokio::test]
    async fn refusals_map_to_errors_and_bad_input_never_reaches_the_server() {
        let cases = [
            (
                401,
                json!({"detail": "Invalid token."}),
                Error::Unauthorized,
            ),
            (
                403,
                json!({"detail": "You do not have permission to perform this action."}),
                Error::Forbidden("You do not have permission to perform this action.".into()),
            ),
            (
                404,
                json!({"detail": "No Document matches the given query."}),
                Error::NotFound("No Document matches the given query.".into()),
            ),
            (
                429,
                json!({"detail": "Request was throttled."}),
                Error::Unavailable("rate limited: Request was throttled.".into()),
            ),
        ];
        for (status, body, expected) in cases {
            let fixture = Fixture::start(vec![
                Exchange::new("GET", "/api/documents/9/", status, body)
                    .header("retry-after", "3600"),
            ])
            .await;
            let read = Paperless
                .call(&cx(&fixture), "read_document", json!({"id": 9}))
                .await;
            assert_eq!(read, Err(expected), "{status}");
            fixture.finish().await;
        }
        let fixture = Fixture::start(Vec::new()).await;
        let cx = cx(&fixture);
        let bad = [
            ("read_document", json!({"id": "2/../../users"})),
            ("search_documents", json!({"text": "tax"})),
            ("edit_tags", json!({"documents": [2]})),
            ("upload_document", json!({"workspace_file": "notes/a.pdf"})),
        ];
        for (tool, input) in bad {
            let refused = Paperless.call(&cx, tool, input).await;
            assert!(
                matches!(refused, Err(Error::Invalid(_))),
                "{tool}: {refused:?}"
            );
        }
        assert!(fixture.finish().await.is_empty());
    }

    /// Every tool against a real server: set `CUBEBOT_LIVE_PAPERLESS_URL` and
    /// `CUBEBOT_LIVE_PAPERLESS_TOKEN`, have tags named Invoices and Taxes, and run
    /// `cargo test -p cubebot-connectors live_paperless -- --ignored`. It uploads a one-page PDF
    /// and waits up to two minutes for Paperless to process it.
    #[tokio::test]
    #[ignore = "needs a Paperless-ngx server"]
    async fn live_paperless() {
        let env = |key: &str| std::env::var(format!("CUBEBOT_LIVE_PAPERLESS_{key}")).unwrap();
        let credential = Credential::Fields {
            fields: [("token".into(), env("TOKEN"))].into(),
        };
        let cx = Context::new(json!({"url": env("URL")}), credential);
        println!("check: {:?}", Paperless.check(&cx).await.unwrap());
        let call = |tool: &'static str, input: Value| {
            let cx = &cx;
            async move {
                let answer = Paperless.call(cx, tool, input).await;
                let shown = format!("{answer:?}");
                println!("{tool}: {}", shown.chars().take(700).collect::<String>());
                answer.unwrap()
            }
        };
        let tags = call("list_tags", json!({})).await;
        let tag = |name: &str| {
            let tags = tags["tags"].as_array().unwrap();
            tags.iter().find(|tag| tag["name"] == name).unwrap()["id"].clone()
        };
        let (invoices, taxes) = (tag("Invoices"), tag("Taxes"));
        let stamp = crate::dav::now_nanos();
        let title = format!("Live check {stamp}");
        let pdf = format!(
            "%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n4 0 obj\n<< /Length 52 >>\nstream\nBT /F1 18 Tf 72 720 Td (Livecheck {stamp}) Tj ET\nendstream\nendobj\n5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n"
        );
        let file = crate::Attachment::new("live.pdf", pdf.as_bytes());
        let input = json!({"workspace_file": file, "title": title, "tags": [taxes]});
        call("upload_document", input).await;
        let mut found = Value::Null;
        for _ in 0..60 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let input = json!({"query": format!("title:\"{title}\"")});
            found = call("search_documents", input).await["documents"][0].clone();
            if !found.is_null() {
                break;
            }
        }
        let id = found["id"].clone();
        assert_eq!(found["tags"], json!([taxes]));
        let read = call("read_document", json!({"id": id})).await;
        assert!(read["content"].as_str().unwrap().contains("Livecheck"));
        let archived = call("download_document", json!({"id": id})).await;
        assert!(
            archived[crate::FILE]["name"]
                .as_str()
                .unwrap()
                .ends_with(".pdf")
        );
        let original = call("download_document", json!({"id": id, "original": true})).await;
        assert_eq!(original[crate::FILE]["name"], "live.pdf");
        let input = json!({"documents": [id], "add": [invoices], "remove": [taxes]});
        call("edit_tags", input).await;
        let tagged = call(
            "search_documents",
            json!({"tags": [invoices], "limit": 100}),
        )
        .await;
        let documents = tagged["documents"].as_array().unwrap();
        assert!(documents.iter().any(|document| document["id"] == id));
    }
}

//! Notion through an internal integration's secret (https://developers.notion.com/reference).
//! The integration sees only the pages someone shared with it.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Field, FieldKind, Level, Manifest,
    Risk, Step, ToolSpec,
};
use reqwest::{Method, RequestBuilder};
use serde::Deserialize;
use serde_json::{Value, json};

const API: &str = "https://api.notion.com/v1";
/// Pinned so a Notion release cannot change the answers under us.
const VERSION: &str = "2022-06-28";

pub struct Notion;

static MANIFEST: Manifest = Manifest {
    id: "notion",
    name: "Notion",
    description: "Search, read and write the Notion pages and databases shared with it.",
    category: Category::Work,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "Internal integration secret",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "ntn_…",
            help: "From the integration's Configuration tab.",
        }],
    }],
    config: &[],
    products: &[],
    tools: &[
        ToolSpec {
            name: "search",
            description: "Find pages and databases by title. Leave the query empty to list recent ones.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"query":{"type":"string"},"kind":{"type":"string","enum":["page","database"],"description":"Only pages or only databases."},"limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_page",
            description: "A page's properties and its text, as Markdown-like lines.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"page":{"type":"string","description":"The page's id or URL."}},"required":["page"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "query_database",
            description: "Rows of a database, optionally filtered and sorted with Notion's filter and sorts objects.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"database":{"type":"string","description":"The database's id or URL."},"filter":{"type":"object"},"sorts":{"type":"array","items":{"type":"object"}},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["database"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "create_page",
            description: "Create a page under a page or as a database row, with a title and paragraphs of text.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"parent":{"type":"string","description":"The parent page's or database's id or URL."},"database":{"type":"boolean","description":"True when the parent is a database."},"title":{"type":"string"},"text":{"type":"string","description":"Body text; each blank-line-separated block becomes a paragraph."},"properties":{"type":"object","description":"Other database properties in Notion's format."}},"required":["parent","title"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "append_text",
            description: "Add paragraphs of text to the end of a page.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"page":{"type":"string"},"text":{"type":"string"}},"required":["page","text"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "update_page",
            description: "Change a page's properties in Notion's format, or move it to the trash with archived.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"page":{"type":"string"},"properties":{"type":"object"},"archived":{"type":"boolean"}},"required":["page"],"additionalProperties":false}"#,
        },
    ],
    events: &[],
    level: Level::Write,
    guide: &[
        Step {
            title: "Create an internal integration",
            body: "In Notion's integrations page choose New integration, pick the workspace, and keep the type Internal. Under Capabilities allow reading, updating and inserting content.",
            link: Some("https://www.notion.so/profile/integrations"),
            copy: None,
        },
        Step {
            title: "Copy its secret",
            body: "On the integration's Configuration tab, show and copy the Internal Integration Secret, then paste it below.",
            link: None,
            copy: None,
        },
        Step {
            title: "Share pages with it",
            body: "The integration sees nothing until it is added to a page: open the page, choose ••• › Connections, and add it. Child pages are included.",
            link: None,
            copy: None,
        },
    ],
    warning: None,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    #[serde(default)]
    query: String,
    kind: Option<String>,
    limit: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    database: String,
    filter: Option<Value>,
    sorts: Option<Value>,
    limit: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    parent: String,
    #[serde(default)]
    database: bool,
    title: String,
    #[serde(default)]
    text: String,
    properties: Option<serde_json::Map<String, Value>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    page: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Append {
    page: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    page: String,
    properties: Option<Value>,
    archived: Option<bool>,
}

impl Connector for Notion {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let me = cx.json(request(cx, Method::GET, "/users/me")?).await?;
            let name = me["bot"]["workspace_name"]
                .as_str()
                .or(me["name"].as_str())
                .unwrap_or("Notion workspace");
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "search" => search(cx, input).await,
                "read_page" => read_page(cx, input).await,
                "query_database" => query(cx, input).await,
                "create_page" => create(cx, input).await,
                "append_text" => append(cx, input).await,
                "update_page" => update(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }
}

fn request(cx: &Context, method: Method, path: &str) -> Result<RequestBuilder, Error> {
    Ok(cx
        .request(method, &format!("{API}{path}"))?
        .bearer_auth(cx.token()?)
        .header("notion-version", VERSION))
}

/// The 32 hex digits at the end of a page's id or URL.
fn id(text: &str) -> Result<String, Error> {
    let path = text.split(['?', '#']).next().unwrap_or_default();
    let hex: String = path.chars().filter(char::is_ascii_hexdigit).collect();
    let id = hex
        .get(hex.len().saturating_sub(32)..)
        .filter(|id| id.len() == 32)
        .ok_or_else(|| Error::Invalid(format!("{text:?} is not a Notion id or URL")))?;
    Ok(id.to_ascii_lowercase())
}

async fn search(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: Search = crate::input(input)?;
    let mut body = json!({"query": search.query, "page_size": search.limit.unwrap_or(20)});
    if let Some(kind) = search.kind {
        body["filter"] = json!({"property": "object", "value": kind});
    }
    let found = cx
        .json(request(cx, Method::POST, "/search")?.json(&body))
        .await?;
    Ok(json!({"results": items(&found)}))
}

async fn query(cx: &Context, input: Value) -> Result<Value, Error> {
    let query: Query = crate::input(input)?;
    let mut body = json!({"page_size": query.limit.unwrap_or(25)});
    if let Some(filter) = query.filter {
        body["filter"] = filter;
    }
    if let Some(sorts) = query.sorts {
        body["sorts"] = sorts;
    }
    let path = format!("/databases/{}/query", id(&query.database)?);
    let rows = cx
        .json(request(cx, Method::POST, &path)?.json(&body))
        .await?;
    Ok(json!({"rows": items(&rows), "more": rows["has_more"]}))
}

async fn read_page(cx: &Context, input: Value) -> Result<Value, Error> {
    let page = id(&crate::input::<Page>(input)?.page)?;
    let found = cx
        .json(request(cx, Method::GET, &format!("/pages/{page}"))?)
        .await?;
    let path = format!("/blocks/{page}/children?page_size=100");
    let blocks = cx.json(request(cx, Method::GET, &path)?).await?;
    let text: Vec<String> = blocks["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(line)
        .collect();
    Ok(json!({
        "page": item(&found),
        "properties": properties(&found["properties"]),
        "text": text.join("\n"),
        "more": blocks["has_more"],
    }))
}

async fn create(cx: &Context, input: Value) -> Result<Value, Error> {
    let create: Create = crate::input(input)?;
    let parent = id(&create.parent)?;
    let title = json!([{"text": {"content": create.title}}]);
    let mut properties = create.properties.unwrap_or_default();
    let parent = if create.database {
        properties.insert("Name".into(), json!({"title": title}));
        json!({"database_id": parent})
    } else {
        properties.insert("title".into(), json!({"title": title}));
        json!({"page_id": parent})
    };
    let body =
        json!({"parent": parent, "properties": properties, "children": paragraphs(&create.text)});
    let made = cx
        .json(request(cx, Method::POST, "/pages")?.json(&body))
        .await?;
    Ok(item(&made))
}

async fn append(cx: &Context, input: Value) -> Result<Value, Error> {
    let append: Append = crate::input(input)?;
    let path = format!("/blocks/{}/children", id(&append.page)?);
    let body = json!({"children": paragraphs(&append.text)});
    let added = cx
        .json(request(cx, Method::PATCH, &path)?.json(&body))
        .await?;
    let count = added["results"].as_array().map_or(0, Vec::len);
    Ok(json!({"appended_blocks": count}))
}

async fn update(cx: &Context, input: Value) -> Result<Value, Error> {
    let update: Update = crate::input(input)?;
    let mut body = json!({});
    if let Some(properties) = update.properties {
        body["properties"] = properties;
    }
    if let Some(archived) = update.archived {
        body["archived"] = json!(archived);
    }
    let path = format!("/pages/{}", id(&update.page)?);
    let changed = cx
        .json(request(cx, Method::PATCH, &path)?.json(&body))
        .await?;
    Ok(item(&changed))
}

/// Notion caps a rich-text item at 2000 characters and a request at 100 blocks.
fn paragraphs(text: &str) -> Vec<Value> {
    text.split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .flat_map(|block| {
            let characters: Vec<char> = block.chars().collect();
            characters
                .chunks(2000)
                .map(|chunk| chunk.iter().collect::<String>())
                .collect::<Vec<_>>()
        })
        .take(100)
        .map(|chunk| json!({"object": "block", "type": "paragraph", "paragraph": {"rich_text": [{"type": "text", "text": {"content": chunk}}]}}))
        .collect()
}

fn items(list: &Value) -> Vec<Value> {
    list["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(item)
        .collect()
}

/// A page or database as the model needs it: what it is, its title and where.
fn item(object: &Value) -> Value {
    let title = object["properties"]
        .as_object()
        .into_iter()
        .flat_map(|properties| properties.values())
        .find(|property| property["type"] == "title")
        .map(|property| plain(&property["title"]))
        .unwrap_or_else(|| plain(&object["title"]));
    json!({
        "id": object["id"],
        "object": object["object"],
        "title": title,
        "url": object["url"],
        "last_edited": object["last_edited_time"],
    })
}

fn plain(rich: &Value) -> String {
    rich.as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part["plain_text"].as_str())
        .collect()
}

/// Property values flattened to text where Notion's shape allows it.
fn properties(properties: &Value) -> Value {
    let flat = properties
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, property)| {
            let kind = property["type"].as_str().unwrap_or_default();
            let value = &property[kind];
            let shown = match kind {
                "title" | "rich_text" => json!(plain(value)),
                "select" | "status" => value["name"].clone(),
                "multi_select" => json!(
                    value
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|o| o["name"].clone())
                        .collect::<Vec<_>>()
                ),
                "date" => value["start"].clone(),
                _ => value.clone(),
            };
            (name.clone(), shown)
        });
    Value::Object(flat.collect())
}

fn line(block: &Value) -> Option<String> {
    let kind = block["type"].as_str()?;
    let text = plain(&block[kind]["rich_text"]);
    let prefix = match kind {
        "heading_1" => "# ",
        "heading_2" => "## ",
        "heading_3" => "### ",
        "bulleted_list_item" => "- ",
        "numbered_list_item" => "1. ",
        "quote" => "> ",
        "to_do" if block[kind]["checked"] == true => "[x] ",
        "to_do" => "[ ] ",
        "child_page" => return Some(format!("[page] {}", block[kind]["title"].as_str()?)),
        "paragraph" | "callout" | "toggle" | "code" => "",
        _ => return None,
    };
    Some(format!("{prefix}{text}"))
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
            fields: [("token".into(), "ntn_secret".into())].into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    fn recorded(name: &str) -> Value {
        let path = format!("{}/fixtures/notion/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn read_page_flattens_properties_and_blocks_to_text() {
        let page = "59833787-2cf9-4fdf-8782-e53db20768a5";
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                &format!("/api.notion.com/v1/pages/{}", page.replace('-', "")),
                200,
                recorded("page"),
            ),
            Exchange::new(
                "GET",
                &format!(
                    "/api.notion.com/v1/blocks/{}/children",
                    page.replace('-', "")
                ),
                200,
                recorded("blocks"),
            ),
        ])
        .await;
        let url = format!(
            "https://www.notion.so/Tuscan-kale-{}",
            page.replace('-', "")
        );
        let read = Notion
            .call(&cx(&fixture), "read_page", json!({"page": url}))
            .await
            .unwrap();
        assert_eq!(read["page"]["title"], "Tuscan kale");
        assert_eq!(read["properties"]["Tags"], json!(["Vegetables"]));
        assert_eq!(
            read["text"],
            "## Ingredients\n- Kale\n[x] Wash it\nLacinato kale is a variety of kale."
        );
        let seen = fixture.finish().await;
        assert_eq!(seen[0].header("notion-version"), Some(VERSION));
        assert_eq!(seen[0].header("authorization"), Some("Bearer ntn_secret"));
    }

    #[tokio::test]
    async fn create_page_titles_a_database_row_and_splits_text_into_paragraphs() {
        let fixture = Fixture::start(vec![Exchange::new(
            "POST",
            "/api.notion.com/v1/pages",
            200,
            recorded("page"),
        )])
        .await;
        let input = json!({"parent": "d9824bdc84454327be8b5b47500af6ce", "database": true, "title": "Tuscan kale", "text": "One.\n\nTwo."});
        let made = Notion
            .call(&cx(&fixture), "create_page", input)
            .await
            .unwrap();
        assert_eq!(
            made["url"],
            "https://www.notion.so/Tuscan-kale-598337872cf94fdf8782e53db20768a5"
        );
        let sent = fixture.finish().await[0].json();
        assert_eq!(
            sent["parent"],
            json!({"database_id": "d9824bdc84454327be8b5b47500af6ce"})
        );
        assert_eq!(
            sent["properties"]["Name"]["title"][0]["text"]["content"],
            "Tuscan kale"
        );
        assert_eq!(sent["children"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn search_filters_by_kind_and_a_bad_id_is_refused_before_any_request() {
        let fixture = Fixture::start(vec![Exchange::new(
            "POST",
            "/api.notion.com/v1/search",
            200,
            recorded("search"),
        )])
        .await;
        let found = Notion
            .call(
                &cx(&fixture),
                "search",
                json!({"query": "kale", "kind": "page"}),
            )
            .await
            .unwrap();
        assert_eq!(found["results"][0]["title"], "Tuscan kale");
        let bad = Notion
            .call(&cx(&fixture), "read_page", json!({"page": "kale"}))
            .await;
        assert!(matches!(bad, Err(Error::Invalid(_))));
        let unknown = Notion
            .call(&cx(&fixture), "search", json!({"q": "kale"}))
            .await;
        assert!(
            matches!(unknown, Err(Error::Invalid(_))),
            "unknown fields are refused"
        );
        let sent = fixture.finish().await[0].json();
        assert_eq!(
            sent["filter"],
            json!({"property": "object", "value": "page"})
        );
    }
}

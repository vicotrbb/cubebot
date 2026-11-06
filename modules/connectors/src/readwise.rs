//! Readwise highlights and the Reader library through the person's access token
//! (https://readwise.io/api_deets, https://readwise.io/reader_api). Listing books, highlights or
//! Reader documents is limited to 20 requests a minute per token, so paging is left to the model.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Field, FieldKind, Level, Manifest,
    Risk, Step, ToolSpec,
};
use reqwest::header::AUTHORIZATION;
use serde::Deserialize;
use serde_json::{Value, json};

const API: &str = "https://readwise.io/api";
/// The most characters of a highlight the model gets.
const HIGHLIGHT: usize = 2000;
/// The most characters of a document's text the model gets.
const LONGEST: usize = 40_000;

pub struct Readwise;

static MANIFEST: Manifest = Manifest {
    id: "readwise",
    name: "Readwise",
    description: "Read your Readwise highlights and the articles saved in Readwise Reader.",
    category: Category::Work,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "Access token",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "",
            help: "From readwise.io/access_token; it reads both Readwise and Reader.",
        }],
    }],
    config: &[],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_books",
            description: "Books, articles, tweets and podcasts you have highlights from, most recently highlighted first.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"category":{"type":"string","enum":["books","articles","tweets","supplementals","podcasts"]},"updated_after":{"type":"string","description":"An ISO 8601 time such as 2026-11-01T00:00:00Z."},"limit":{"type":"integer","minimum":1,"maximum":100},"page":{"type":"integer","minimum":1,"description":"The next page from a previous answer."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_highlights",
            description: "Highlights with their notes, optionally of one book from list_books.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"book":{"type":"integer","description":"The book's id from list_books."},"updated_after":{"type":"string","description":"An ISO 8601 time such as 2026-11-01T00:00:00Z."},"limit":{"type":"integer","minimum":1,"maximum":100},"page":{"type":"integer","minimum":1}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_documents",
            description: "Documents saved in Readwise Reader, newest first, filtered by location or category.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"location":{"type":"string","enum":["new","later","shortlist","archive","feed"]},"category":{"type":"string","enum":["article","email","rss","highlight","note","pdf","epub","tweet","video"]},"updated_after":{"type":"string","description":"An ISO 8601 time such as 2026-11-01T00:00:00Z."},"limit":{"type":"integer","minimum":1,"maximum":100},"cursor":{"type":"string","description":"The next cursor from a previous answer."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_document",
            description: "A Reader document's text, from its id in list_documents.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"document":{"type":"string","description":"The document's id."}},"required":["document"],"additionalProperties":false}"#,
        },
    ],
    events: &[],
    level: Level::Read,
    guide: &[Step {
        title: "Copy your access token",
        body: "Sign in to Readwise, open the access token page, generate a token if none is shown, and paste it below. The same token reads your highlights and your Reader library.",
        link: Some("https://readwise.io/access_token"),
        copy: None,
    }],
    warning: Some(
        "Readwise allows 20 list requests a minute per token for books, highlights and Reader documents; beyond that, calls fail for a minute.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Books {
    category: Option<String>,
    updated_after: Option<String>,
    limit: Option<u8>,
    page: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Highlights {
    book: Option<u64>,
    updated_after: Option<String>,
    limit: Option<u8>,
    page: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Documents {
    location: Option<String>,
    category: Option<String>,
    updated_after: Option<String>,
    limit: Option<u8>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    document: String,
}

impl Connector for Readwise {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            // Answers 204 for a working token; Readwise names no account.
            get(cx, "/v2/auth/", Vec::new()).await?;
            Ok(Account {
                name: "Readwise".into(),
            })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "list_books" => books(cx, input).await,
                "list_highlights" => highlights(cx, input).await,
                "list_documents" => documents(cx, input).await,
                "read_document" => document(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }
}

type Query = Vec<(&'static str, String)>;

async fn get(cx: &Context, path: &str, query: Query) -> Result<Value, Error> {
    let request = cx
        .get(&format!("{API}{path}"))?
        .header(AUTHORIZATION, format!("Token {}", cx.token()?))
        .query(&query);
    cx.json(request).await
}

fn list(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

fn clip(text: &Value, longest: usize) -> Value {
    match text.as_str() {
        Some(text) => json!(text.chars().take(longest).collect::<String>()),
        None => Value::Null,
    }
}

/// Readwise v2 pages by number: the next one, when `next` says there is one.
fn paged(query: &mut Query, limit: Option<u8>, page: Option<u32>) -> u32 {
    let page = page.unwrap_or(1).max(1);
    query.push(("page_size", limit.unwrap_or(25).clamp(1, 100).to_string()));
    query.push(("page", page.to_string()));
    page
}

fn next_page(answer: &Value, page: u32) -> Value {
    match answer["next"].is_string() {
        true => json!(page + 1),
        false => Value::Null,
    }
}

async fn books(cx: &Context, input: Value) -> Result<Value, Error> {
    let books: Books = crate::input(input)?;
    let mut query = Vec::new();
    let page = paged(&mut query, books.limit, books.page);
    query.extend(books.category.map(|category| ("category", category)));
    query.extend(books.updated_after.map(|after| ("updated__gt", after)));
    let found = get(cx, "/v2/books/", query).await?;
    let shown: Vec<Value> = list(&found["results"])
        .map(|book| {
            json!({
                "id": book["id"], "title": book["title"], "author": book["author"],
                "category": book["category"], "source": book["source"],
                "highlights": book["num_highlights"], "last_highlight_at": book["last_highlight_at"],
                "source_url": book["source_url"],
            })
        })
        .collect();
    Ok(json!({"books": shown, "next_page": next_page(&found, page)}))
}

async fn highlights(cx: &Context, input: Value) -> Result<Value, Error> {
    let highlights: Highlights = crate::input(input)?;
    let mut query = Vec::new();
    let page = paged(&mut query, highlights.limit, highlights.page);
    query.extend(highlights.book.map(|book| ("book_id", book.to_string())));
    query.extend(highlights.updated_after.map(|after| ("updated__gt", after)));
    let found = get(cx, "/v2/highlights/", query).await?;
    let shown: Vec<Value> = list(&found["results"])
        .map(|highlight| {
            let tags: Vec<&Value> = list(&highlight["tags"]).map(|tag| &tag["name"]).collect();
            json!({
                "id": highlight["id"], "book": highlight["book_id"],
                "text": clip(&highlight["text"], HIGHLIGHT), "note": clip(&highlight["note"], HIGHLIGHT),
                "highlighted_at": highlight["highlighted_at"], "url": highlight["url"], "tags": tags,
            })
        })
        .collect();
    Ok(json!({"highlights": shown, "next_page": next_page(&found, page)}))
}

async fn documents(cx: &Context, input: Value) -> Result<Value, Error> {
    let documents: Documents = crate::input(input)?;
    let mut query = vec![(
        "limit",
        documents.limit.unwrap_or(25).clamp(1, 100).to_string(),
    )];
    query.extend(documents.location.map(|location| ("location", location)));
    query.extend(documents.category.map(|category| ("category", category)));
    query.extend(documents.updated_after.map(|after| ("updatedAfter", after)));
    query.extend(documents.cursor.map(|cursor| ("pageCursor", cursor)));
    let found = get(cx, "/v3/list/", query).await?;
    let shown: Vec<Value> = list(&found["results"]).map(summary).collect();
    Ok(json!({"documents": shown, "next": found["nextPageCursor"]}))
}

fn summary(document: &Value) -> Value {
    json!({
        "id": document["id"], "title": document["title"], "author": document["author"],
        "category": document["category"], "location": document["location"],
        "url": document["url"], "source_url": document["source_url"],
        "summary": clip(&document["summary"], HIGHLIGHT), "word_count": document["word_count"],
        "reading_progress": document["reading_progress"], "saved_at": document["saved_at"],
        "published_date": document["published_date"],
    })
}

async fn document(cx: &Context, input: Value) -> Result<Value, Error> {
    let id = crate::input::<Document>(input)?.document;
    let valid = (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric());
    if !valid {
        return Err(Error::Invalid(
            "document must be an id from list_documents".into(),
        ));
    }
    let query = vec![("id", id), ("withHtmlContent", "true".into())];
    let found = get(cx, "/v3/list/", query).await?;
    let document = found["results"]
        .get(0)
        .ok_or_else(|| Error::NotFound("no Reader document has that id".into()))?;
    let html = document["html_content"].as_str().unwrap_or_default();
    let text = crate::rss::html_text(html);
    let truncated = text.chars().count() > LONGEST;
    let mut shown = summary(document);
    shown["text"] = json!(text.chars().take(LONGEST).collect::<String>());
    shown["truncated"] = json!(truncated);
    Ok(shown)
}

#[cfg(test)]
mod tests {
    //! Answers are trimmed from the examples in Readwise's API references.
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    const TOKEN: &str = "rw-test-token-1234";

    fn cx(fixture: &Fixture) -> Context {
        let credential = Credential::Fields {
            fields: [("token".into(), TOKEN.into())].into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    fn recorded(name: &str) -> Value {
        let path = format!(
            "{}/fixtures/readwise/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn list_books_and_highlights_page_by_number_with_the_token_header() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/readwise.io/api/v2/books/?page_size=2&page=1&category=books&updated__gt=2026-11-01T00%3A00%3A00Z",
                200,
                recorded("books"),
            ),
            Exchange::new(
                "GET",
                "/readwise.io/api/v2/highlights/?page_size=25&page=2&book_id=1776",
                200,
                recorded("highlights"),
            ),
        ])
        .await;
        let cx = cx(&fixture);
        let input =
            json!({"category": "books", "updated_after": "2026-11-01T00:00:00Z", "limit": 2});
        let books = Readwise.call(&cx, "list_books", input).await.unwrap();
        assert_eq!(
            books["books"][0],
            json!({"id": 1776, "title": "Range", "author": "David Epstein", "category": "books", "source": "kindle", "highlights": 2, "last_highlight_at": "2026-11-02T18:30:00Z", "source_url": null})
        );
        assert_eq!(books["next_page"], 2);
        let input = json!({"book": 1776, "page": 2});
        let highlights = Readwise.call(&cx, "list_highlights", input).await.unwrap();
        assert_eq!(
            highlights,
            json!({"highlights": [{"id": 59758950, "book": 1776, "text": "The best way to learn is to try.", "note": "Try it", "highlighted_at": "2026-11-02T18:30:00Z", "url": null, "tags": ["favorite"]}], "next_page": null})
        );
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].header("authorization"),
            Some(&*format!("Token {TOKEN}"))
        );
    }

    #[tokio::test]
    async fn list_documents_filters_and_pages_with_a_cursor() {
        let fixture = Fixture::start(vec![Exchange::new(
            "GET",
            "/readwise.io/api/v3/list/?limit=10&location=later&category=article&pageCursor=01gm6kjzabcd",
            200,
            recorded("documents"),
        )])
        .await;
        let input = json!({"location": "later", "category": "article", "limit": 10, "cursor": "01gm6kjzabcd"});
        let listed = Readwise
            .call(&cx(&fixture), "list_documents", input)
            .await
            .unwrap();
        assert_eq!(listed["next"], "01gm6kjzefgh");
        let first = &listed["documents"][0];
        assert_eq!(first["id"], "01gwfvp9pyaabcdgmx14f6ha0");
        assert_eq!(first["title"], "Foo Bar");
        assert_eq!(
            first["url"],
            "https://read.readwise.io/new/read/01gwfvp9pyaabcdgmx14f6ha0"
        );
        assert!(first.get("html_content").is_none());
        fixture.finish().await;
    }

    #[tokio::test]
    async fn read_document_turns_its_html_into_text_and_refuses_bad_ids() {
        let mut found = recorded("documents");
        found["results"][0]["html_content"] =
            json!("<div><h1>Foo Bar</h1><p>First &amp; second.</p><script>x()</script></div>");
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/readwise.io/api/v3/list/?id=01gwfvp9pyaabcdgmx14f6ha0&withHtmlContent=true",
                200,
                found,
            ),
            Exchange::new(
                "GET",
                "/readwise.io/api/v3/list/?id=01missing&withHtmlContent=true",
                200,
                json!({"count": 0, "nextPageCursor": null, "results": []}),
            ),
        ])
        .await;
        let cx = cx(&fixture);
        let input = json!({"document": "01gwfvp9pyaabcdgmx14f6ha0"});
        let read = Readwise.call(&cx, "read_document", input).await.unwrap();
        assert_eq!(read["text"], "Foo Bar\nFirst & second.");
        assert_eq!(read["truncated"], false);
        let missing = Readwise
            .call(&cx, "read_document", json!({"document": "01missing"}))
            .await;
        assert!(matches!(missing, Err(Error::NotFound(_))));
        for input in [
            json!({"document": "../v2/auth"}),
            json!({"document": "a", "html": true}),
        ] {
            let refused = Readwise.call(&cx, "read_document", input).await;
            assert!(matches!(refused, Err(Error::Invalid(_))));
        }
        fixture.finish().await;
    }

    #[tokio::test]
    async fn refusals_map_to_errors() {
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
                json!({"detail": "Not found."}),
                Error::NotFound("Not found.".into()),
            ),
            (
                429,
                json!({"detail": "Request was throttled. Expected available in 60 seconds."}),
                Error::Unavailable(
                    "rate limited: Request was throttled. Expected available in 60 seconds.".into(),
                ),
            ),
        ];
        for (status, body, expected) in cases {
            let answer = Exchange::new("GET", "/readwise.io/api/v3/list/", status, body)
                .header("retry-after", "3600");
            let fixture = Fixture::start(vec![answer]).await;
            let listed = Readwise
                .call(&cx(&fixture), "list_documents", json!({}))
                .await;
            assert_eq!(listed, Err(expected), "{status}");
            fixture.finish().await;
        }
    }
}

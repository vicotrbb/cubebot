use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Field, FieldKind, Level, Manifest,
    Risk, Step, ToolSpec,
};
use serde::Deserialize;
use serde_json::{Value, json};

const API: &str = "https://api.giphy.com/v1/gifs";
const RATINGS: [&str; 4] = ["g", "pg", "pg-13", "r"];

pub struct Giphy;

static MANIFEST: Manifest = Manifest {
    id: "giphy",
    name: "GIPHY",
    description: "Find GIFs on GIPHY for agents to post in chat.",
    category: Category::Chat,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "api_key",
            label: "API key",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "",
            help: "From the dashboard at developers.giphy.com after creating an API app.",
        }],
    }],
    config: &[],
    products: &[],
    tools: &[
        ToolSpec {
            name: "search",
            description: "Search GIPHY for GIFs. GIFs are for expression, so pick one that fits the moment, then pass its gif URL to send_gif to post it in chat.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"query":{"type":"string","description":"What the GIF should show, such as high five or mind blown."},"limit":{"type":"integer","minimum":1,"maximum":10,"description":"5 unless given."},"rating":{"type":"string","enum":["g","pg","pg-13","r"],"description":"The most mature content allowed; pg-13 unless given."}},"required":["query"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "trending",
            description: "GIFs trending on GIPHY now. GIFs are for expression, so pick one that fits the moment, then pass its gif URL to send_gif to post it in chat.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"limit":{"type":"integer","minimum":1,"maximum":10,"description":"5 unless given."},"rating":{"type":"string","enum":["g","pg","pg-13","r"],"description":"The most mature content allowed; pg-13 unless given."}},"additionalProperties":false}"#,
        },
    ],
    events: &[],
    level: Level::Read,
    guide: &[Step {
        title: "Create a GIPHY API key",
        body: "Sign in at developers.giphy.com, open the dashboard, create an API app (not an SDK app), and paste its API key below. Ask GIPHY to upgrade the key from the dashboard if agents need more than its beta limit.",
        link: Some("https://developers.giphy.com/dashboard/"),
        copy: None,
    }],
    warning: Some(
        "A new GIPHY key is a beta key with a low hourly request limit; past it, calls fail until the hour resets.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: String,
    limit: Option<u8>,
    rating: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Trending {
    limit: Option<u8>,
    rating: Option<String>,
}

impl Connector for Giphy {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            get(cx, "trending", vec![("limit", "1".into())]).await?;
            Ok(Account {
                name: "GIPHY".into(),
            })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "search" => search(cx, input).await,
                "trending" => trending(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }
}

type Query = Vec<(&'static str, String)>;

async fn get(cx: &Context, path: &str, query: Query) -> Result<Value, Error> {
    let request = cx
        .get(&format!("{API}/{path}"))?
        .query(&[("api_key", cx.secret("api_key")?)])
        .query(&query);
    cx.json(request).await
}

fn shown(limit: Option<u8>, rating: Option<String>) -> Result<Query, Error> {
    let rating = rating.unwrap_or_else(|| "pg-13".into());
    if !RATINGS.contains(&rating.as_str()) {
        return Err(Error::Invalid("rating is one of g, pg, pg-13 or r".into()));
    }
    let limit = limit.unwrap_or(5).clamp(1, 10).to_string();
    Ok(vec![("limit", limit), ("rating", rating)])
}

async fn gifs(cx: &Context, path: &str, query: Query) -> Result<Value, Error> {
    let found = get(cx, path, query).await?;
    let gifs: Vec<Value> = found["data"]
        .as_array()
        .into_iter()
        .flatten()
        .map(gif)
        .collect();
    Ok(json!({"gifs": gifs}))
}

fn gif(found: &Value) -> Value {
    let images = &found["images"];
    let downsized = images["downsized"]["url"]
        .as_str()
        .is_some_and(|url| !url.is_empty());
    let image = match downsized {
        true => &images["downsized"],
        false => &images["fixed_height"],
    };
    json!({
        "id": found["id"], "title": found["title"], "gif": image["url"],
        "width": image["width"], "height": image["height"], "page": found["url"],
    })
}

async fn search(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: Search = crate::input(input)?;
    let words = search.query.trim();
    if words.is_empty() {
        return Err(Error::Invalid("query is empty".into()));
    }
    let mut query = vec![("q", words.to_owned())];
    query.extend(shown(search.limit, search.rating)?);
    query.push(("lang", "en".into()));
    gifs(cx, "search", query).await
}

async fn trending(cx: &Context, input: Value) -> Result<Value, Error> {
    let trending: Trending = crate::input(input)?;
    gifs(cx, "trending", shown(trending.limit, trending.rating)?).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    const KEY: &str = "giphy-test-key-1234";

    fn cx(fixture: &Fixture) -> Context {
        let credential = Credential::Fields {
            fields: [("api_key".into(), KEY.into())].into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    fn found(id: &str, downsized: Value) -> Value {
        json!({
            "type": "gif", "id": id, "title": format!("{id} GIF"),
            "url": format!("https://giphy.com/gifs/{id}"),
            "images": {
                "downsized": downsized,
                "fixed_height": {"url": format!("https://media.giphy.com/media/{id}/200.gif"), "width": "356", "height": "200"},
            },
        })
    }

    #[tokio::test]
    async fn search_maps_results_and_sends_the_key_as_api_key() {
        let downsized = json!({"url": "https://media.giphy.com/media/a1/giphy-downsized.gif", "width": "480", "height": "270"});
        let answer = json!({"data": [found("a1", downsized), found("b2", json!({"url": ""}))], "meta": {"status": 200}});
        let fixture = Fixture::start(vec![Exchange::new(
            "GET",
            &format!(
                "/api.giphy.com/v1/gifs/search?api_key={KEY}&q=high+five&limit=2&rating=g&lang=en"
            ),
            200,
            answer,
        )])
        .await;
        let input = json!({"query": " high five ", "limit": 2, "rating": "g"});
        let shown = Giphy.call(&cx(&fixture), "search", input).await.unwrap();
        assert_eq!(
            shown,
            json!({"gifs": [
                {"id": "a1", "title": "a1 GIF", "gif": "https://media.giphy.com/media/a1/giphy-downsized.gif", "width": "480", "height": "270", "page": "https://giphy.com/gifs/a1"},
                {"id": "b2", "title": "b2 GIF", "gif": "https://media.giphy.com/media/b2/200.gif", "width": "356", "height": "200", "page": "https://giphy.com/gifs/b2"},
            ]})
        );
        fixture.finish().await;
    }

    #[tokio::test]
    async fn trending_defaults_and_bad_input_is_refused() {
        let fixture = Fixture::start(vec![Exchange::new(
            "GET",
            &format!("/api.giphy.com/v1/gifs/trending?api_key={KEY}&limit=5&rating=pg-13"),
            200,
            json!({"data": [found("c3", Value::Null)]}),
        )])
        .await;
        let cx = cx(&fixture);
        let shown = Giphy.call(&cx, "trending", json!({})).await.unwrap();
        assert_eq!(
            shown["gifs"][0]["gif"],
            "https://media.giphy.com/media/c3/200.gif"
        );
        for (tool, input) in [
            ("search", json!({"query": "  "})),
            ("search", json!({"query": "cat", "rating": "nc-17"})),
            ("trending", json!({"query": "cat"})),
        ] {
            let refused = Giphy.call(&cx, tool, input).await;
            assert!(matches!(refused, Err(Error::Invalid(_))), "{refused:?}");
        }
        fixture.finish().await;
    }

    #[tokio::test]
    async fn a_refused_key_is_unauthorized() {
        let fixture = Fixture::start(vec![Exchange::new(
            "GET",
            "/api.giphy.com/v1/gifs/trending",
            401,
            json!({"message": "Unauthorized"}),
        )])
        .await;
        let refused = Giphy.call(&cx(&fixture), "trending", json!({})).await;
        assert_eq!(refused, Err(Error::Unauthorized));
        fixture.finish().await;
    }
}

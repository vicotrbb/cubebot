//! Trello through an API key and a user token (https://developer.atlassian.com/cloud/trello/rest/).
//! Both travel in the documented `Authorization: OAuth …` header, so neither lands in a URL
//! (https://developer.atlassian.com/cloud/trello/guides/rest-api/authorization/).
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Field, FieldKind, Level, Manifest,
    Risk, Step, ToolSpec,
};
use reqwest::{Method, RequestBuilder};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const API: &str = "https://api.trello.com/1";
const LONGEST: usize = 8_000;
/// Cards shown per list of a board; a busy list is cut short.
const CARDS: usize = 100;

pub struct Trello;

static MANIFEST: Manifest = Manifest {
    id: "trello",
    name: "Trello",
    description: "Read Trello boards and cards, create, move and archive cards, and comment.",
    category: Category::Work,
    auth: &[Auth::ApiKey {
        fields: &[
            Field {
                key: "key",
                label: "API key",
                kind: FieldKind::Secret,
                required: true,
                placeholder: "32 hex characters",
                help: "From your Power-Up's API key tab.",
            },
            Field {
                key: "token",
                label: "Token",
                kind: FieldKind::Secret,
                required: true,
                placeholder: "ATTA…",
                help: "The token Trello shows after you allow access.",
            },
        ],
    }],
    config: &[],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_boards",
            description: "Your open boards, with their ids and links.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_board",
            description: "A board's open lists and the open cards on each.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"board":{"type":"string","description":"The board's id or URL."}},"required":["board"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_card",
            description: "A card's description, checklists, members and latest comments.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"card":{"type":"string","description":"The card's id or URL."}},"required":["card"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "create_card",
            description: "Add a card to a list, with an optional description and due date.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"list":{"type":"string","description":"The list's id, from read_board."},"name":{"type":"string"},"description":{"type":"string"},"due":{"type":"string","description":"An ISO 8601 date and time."},"position":{"type":"string","enum":["top","bottom"]}},"required":["list","name"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "update_card",
            description: "Rename a card, change its description or due date, mark it done, move it to another list, or archive it.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"card":{"type":"string","description":"The card's id or URL."},"name":{"type":"string"},"description":{"type":"string"},"due":{"type":"string","description":"An ISO 8601 date and time."},"done":{"type":"boolean","description":"Whether the due date is complete."},"list":{"type":"string","description":"A list id to move the card to."},"archived":{"type":"boolean"}},"required":["card"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "comment_card",
            description: "Comment on a card; its members and watchers are notified.",
            risk: Risk::Send,
            product: None,
            input: r#"{"type":"object","properties":{"card":{"type":"string","description":"The card's id or URL."},"text":{"type":"string"}},"required":["card","text"],"additionalProperties":false}"#,
        },
    ],
    events: &[],
    level: Level::Write,
    guide: &[
        Step {
            title: "Create a Power-Up",
            body: "Trello API keys belong to a Power-Up. In the Power-Up admin portal choose New, give it a name such as Cubebot, pick your Workspace, fill in an email and author, and create it.",
            link: Some("https://trello.com/power-ups/admin"),
            copy: None,
        },
        Step {
            title: "Generate its API key",
            body: "Open the Power-Up, go to its API key tab and choose Generate a new API key. Copy the API key (not the secret) and paste it below.",
            link: None,
            copy: None,
        },
        Step {
            title: "Allow access and copy the token",
            body: "Open this address with your API key in place of YOUR_KEY, check the account and choose Allow. Trello then shows a token: paste it below. expiration=never keeps it working; use 30days if you prefer it to lapse.",
            link: None,
            copy: Some(
                "https://trello.com/1/authorize?expiration=never&scope=read,write&response_type=token&name=Cubebot&key=YOUR_KEY",
            ),
        },
    ],
    warning: Some(
        "The token acts as you on every board you can see. Revoke it under your Trello account's Settings › Applications.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoInput {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Board {
    board: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Card {
    card: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    list: String,
    name: String,
    description: Option<String>,
    due: Option<String>,
    position: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    card: String,
    name: Option<String>,
    description: Option<String>,
    due: Option<String>,
    done: Option<bool>,
    list: Option<String>,
    archived: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Comment {
    card: String,
    text: String,
}

impl Connector for Trello {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let me = request(cx, Method::GET, "/members/me")?.query(&[("fields", "username")]);
            let me = cx.json(me).await?;
            let name = me["username"].as_str().unwrap_or("Trello account");
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "list_boards" => list_boards(cx, input).await,
                "read_board" => read_board(cx, input).await,
                "read_card" => read_card(cx, input).await,
                "create_card" => create_card(cx, input).await,
                "update_card" => update_card(cx, input).await,
                "comment_card" => comment_card(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }
}

fn request(cx: &Context, method: Method, path: &str) -> Result<RequestBuilder, Error> {
    let authorization = format!(
        r#"OAuth oauth_consumer_key="{}", oauth_token="{}""#,
        cx.secret("key")?,
        cx.secret("token")?
    );
    Ok(cx
        .request(method, &format!("{API}{path}"))?
        .header(reqwest::header::AUTHORIZATION, authorization))
}

/// A board's or card's id (24 hex digits) or short link (8 characters), alone or in its URL
/// (`https://trello.com/c/<short link>/…`).
fn id(text: &str) -> Result<String, Error> {
    let found = match text.split_once("trello.com/") {
        Some((_, path)) => path.split('/').nth(1),
        None => Some(text),
    };
    found
        .filter(|id| matches!(id.len(), 8 | 24) && id.bytes().all(|b| b.is_ascii_alphanumeric()))
        .map(str::to_owned)
        .ok_or_else(|| Error::Invalid(format!("{text:?} is not a Trello id or URL")))
}

async fn list_boards(cx: &Context, input: Value) -> Result<Value, Error> {
    crate::input::<NoInput>(input)?;
    let query = [("filter", "open"), ("fields", "name,url,dateLastActivity")];
    let request = request(cx, Method::GET, "/members/me/boards")?.query(&query);
    let boards = cx.json(request).await?;
    let board = |board: &Value| json!({"id": board["id"], "name": board["name"], "url": board["url"], "last_activity": board["dateLastActivity"]});
    Ok(json!({"boards": each(&boards, board)}))
}

async fn read_board(cx: &Context, input: Value) -> Result<Value, Error> {
    let board = id(&crate::input::<Board>(input)?.board)?;
    let query = [
        ("filter", "open"),
        ("fields", "name"),
        ("cards", "open"),
        ("card_fields", "name,due,dueComplete,shortUrl,labels"),
    ];
    let path = format!("/boards/{board}/lists");
    let lists = cx
        .json(request(cx, Method::GET, &path)?.query(&query))
        .await?;
    let list = |list: &Value| {
        let cards = list["cards"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let shown: Vec<Value> = cards.iter().take(CARDS).map(card).collect();
        json!({"id": list["id"], "name": list["name"], "cards": shown, "more": cards.len() > CARDS})
    };
    Ok(json!({"lists": each(&lists, list)}))
}

async fn read_card(cx: &Context, input: Value) -> Result<Value, Error> {
    let card_id = id(&crate::input::<Card>(input)?.card)?;
    let query = [
        (
            "fields",
            "name,desc,due,dueComplete,closed,idBoard,idList,shortUrl,labels",
        ),
        ("checklists", "all"),
        ("checklist_fields", "name"),
        ("members", "true"),
        ("member_fields", "fullName,username"),
        ("actions", "commentCard"),
        ("actions_limit", "20"),
    ];
    let path = format!("/cards/{card_id}");
    let found = cx
        .json(request(cx, Method::GET, &path)?.query(&query))
        .await?;
    let mut shown = card(&found);
    shown["description"] = json!(clip(text(&found["desc"])));
    shown["archived"] = found["closed"].clone();
    shown["board"] = found["idBoard"].clone();
    shown["list"] = found["idList"].clone();
    shown["members"] = json!(each(&found["members"], |m| m["username"].clone()));
    shown["checklists"] = json!(each(&found["checklists"], checklist));
    shown["comments"] = json!(each(&found["actions"], comment));
    Ok(shown)
}

async fn create_card(cx: &Context, input: Value) -> Result<Value, Error> {
    let create: Create = crate::input(input)?;
    let fields = [
        ("idList", Some(id(&create.list)?)),
        ("name", Some(create.name)),
        ("desc", create.description),
        ("due", create.due),
        ("pos", create.position),
    ];
    let made = cx
        .json(request(cx, Method::POST, "/cards")?.json(&present(fields)))
        .await?;
    Ok(card(&made))
}

async fn update_card(cx: &Context, input: Value) -> Result<Value, Error> {
    let update: Update = crate::input(input)?;
    let list = update.list.as_deref().map(id).transpose()?;
    let fields = [
        ("name", update.name.map(Value::from)),
        ("desc", update.description.map(Value::from)),
        ("due", update.due.map(Value::from)),
        ("dueComplete", update.done.map(Value::from)),
        ("idList", list.map(Value::from)),
        ("closed", update.archived.map(Value::from)),
    ];
    let path = format!("/cards/{}", id(&update.card)?);
    let changed = cx
        .json(request(cx, Method::PUT, &path)?.json(&present(fields)))
        .await?;
    let mut shown = card(&changed);
    shown["list"] = changed["idList"].clone();
    shown["archived"] = changed["closed"].clone();
    Ok(shown)
}

async fn comment_card(cx: &Context, input: Value) -> Result<Value, Error> {
    let comment: Comment = crate::input(input)?;
    let path = format!("/cards/{}/actions/comments", id(&comment.card)?);
    let body = json!({"text": comment.text});
    let made = cx
        .json(request(cx, Method::POST, &path)?.json(&body))
        .await?;
    Ok(json!({"id": made["id"], "date": made["date"]}))
}

/// The fields that were given, as a request body.
fn present<V: Into<Value>, const N: usize>(fields: [(&str, Option<V>); N]) -> Map<String, Value> {
    fields
        .into_iter()
        .filter_map(|(key, value)| Some((key.to_owned(), value?.into())))
        .collect()
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn clip(text: &str) -> String {
    match text.char_indices().nth(LONGEST) {
        Some((end, _)) => format!("{}\n…[cut short]", &text[..end]),
        None => text.into(),
    }
}

fn each(list: &Value, shown: impl Fn(&Value) -> Value) -> Vec<Value> {
    list.as_array().into_iter().flatten().map(shown).collect()
}

fn card(card: &Value) -> Value {
    json!({
        "id": card["id"],
        "name": card["name"],
        "due": card["due"],
        "done": card["dueComplete"],
        "labels": each(&card["labels"], |label| label["name"].clone()),
        "url": card["shortUrl"],
    })
}

fn checklist(checklist: &Value) -> Value {
    let item = |item: &Value| json!({"name": item["name"], "done": item["state"] == "complete"});
    json!({"name": checklist["name"], "items": each(&checklist["checkItems"], item)})
}

fn comment(action: &Value) -> Value {
    json!({
        "author": action["memberCreator"]["username"],
        "date": action["date"],
        "text": clip(text(&action["data"]["text"])),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    const CARD: &str = "5abbe4b7ddc1b351ef961414";

    fn cx(fixture: &Fixture) -> Context {
        let credential = Credential::Fields {
            fields: [
                ("key".into(), "trello-api-key".into()),
                ("token".into(), "trello-user-token".into()),
            ]
            .into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    fn recorded(name: &str) -> Value {
        let path = format!("{}/fixtures/trello/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn boards_lists_and_cards_are_read_with_both_secrets_in_the_header() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api.trello.com/1/members/me/boards?filter=open&fields=name%2Curl%2CdateLastActivity",
                200,
                recorded("boards"),
            ),
            Exchange::new(
                "GET",
                "/api.trello.com/1/boards/5abbe4b7/lists",
                200,
                recorded("lists"),
            ),
            Exchange::new(
                "GET",
                &format!("/api.trello.com/1/cards/{CARD}"),
                200,
                recorded("card"),
            ),
        ])
        .await;
        let cx = cx(&fixture);
        let boards = Trello.call(&cx, "list_boards", json!({})).await.unwrap();
        assert_eq!(boards["boards"][0]["name"], "Groceries");
        let url = "https://trello.com/b/5abbe4b7/groceries";
        let board = Trello
            .call(&cx, "read_board", json!({"board": url}))
            .await
            .unwrap();
        assert_eq!(board["lists"][0]["name"], "To buy");
        assert_eq!(
            board["lists"][0]["cards"][0],
            json!({"id": CARD, "name": "Milk", "due": "2026-11-06T12:00:00.000Z", "done": false, "labels": ["dairy"], "url": "https://trello.com/c/DRbmOQ1k"})
        );
        let card = Trello
            .call(&cx, "read_card", json!({"card": CARD}))
            .await
            .unwrap();
        assert_eq!(card["description"], "Whole, two litres");
        assert_eq!(card["members"], json!(["victorbona"]));
        assert_eq!(
            card["checklists"][0]["items"],
            json!([{"name": "Check the fridge", "done": true}])
        );
        assert_eq!(
            card["comments"][0],
            json!({"author": "victorbona", "date": "2026-11-04T18:00:00.000Z", "text": "Oat milk too"})
        );
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].header("authorization"),
            Some(r#"OAuth oauth_consumer_key="trello-api-key", oauth_token="trello-user-token""#)
        );
        assert!(!seen[0].target.contains("token"), "no secret in the URL");
        assert!(seen[2].target.contains("actions=commentCard"));
    }

    #[tokio::test]
    async fn cards_are_created_moved_archived_and_commented_on() {
        let fixture = Fixture::start(vec![
            Exchange::new("POST", "/api.trello.com/1/cards", 200, recorded("card")),
            Exchange::new(
                "PUT",
                &format!("/api.trello.com/1/cards/{CARD}"),
                200,
                recorded("card"),
            ),
            Exchange::new(
                "POST",
                "/api.trello.com/1/cards/DRbmOQ1k/actions/comments",
                200,
                json!({"id": "5abbe4b7ddc1b351ef961999", "type": "commentCard", "date": "2026-11-05T10:00:00.000Z", "data": {"text": "Bought"}}),
            ),
        ])
        .await;
        let cx = cx(&fixture);
        let list = "5abbe4b7ddc1b351ef961400";
        let create = json!({"list": list, "name": "Milk", "position": "top"});
        let made = Trello.call(&cx, "create_card", create).await.unwrap();
        assert_eq!(made["id"], CARD);
        let update = json!({"card": CARD, "list": "5abbe4b7ddc1b351ef961401", "archived": true});
        Trello.call(&cx, "update_card", update).await.unwrap();
        let comment = json!({"card": "https://trello.com/c/DRbmOQ1k/12-milk", "text": "Bought"});
        let posted = Trello.call(&cx, "comment_card", comment).await.unwrap();
        assert_eq!(posted["id"], "5abbe4b7ddc1b351ef961999");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].json(),
            json!({"idList": list, "name": "Milk", "pos": "top"})
        );
        assert_eq!(
            seen[1].json(),
            json!({"idList": "5abbe4b7ddc1b351ef961401", "closed": true})
        );
        assert_eq!(seen[2].json(), json!({"text": "Bought"}));
    }

    #[tokio::test]
    async fn bad_input_is_refused_before_any_request() {
        let fixture = Fixture::start(Vec::new()).await;
        let cx = cx(&fixture);
        for (tool, input) in [
            ("read_card", json!({"card": "../members/me"})),
            ("read_card", json!({"card": CARD, "fields": "all"})),
            ("update_card", json!({"card": CARD, "list": "nope"})),
            ("list_boards", json!({"all": true})),
        ] {
            let refused = Trello.call(&cx, tool, input.clone()).await;
            assert!(matches!(refused, Err(Error::Invalid(_))), "{input}");
        }
        assert!(fixture.finish().await.is_empty());
    }

    #[tokio::test]
    async fn refusals_map_to_errors() {
        let path = format!("/api.trello.com/1/cards/{CARD}");
        let cases = [
            (
                Exchange::raw("GET", &path, 401, "invalid token"),
                Error::Unauthorized,
            ),
            (
                Exchange::raw("GET", &path, 403, "unauthorized card permission requested"),
                Error::Forbidden("unauthorized card permission requested".into()),
            ),
            (
                Exchange::raw("GET", &path, 404, "The requested resource was not found."),
                Error::NotFound("The requested resource was not found.".into()),
            ),
            (
                Exchange::raw("GET", &path, 400, "invalid id"),
                Error::Rejected("invalid id".into()),
            ),
        ];
        for (exchange, expected) in cases {
            let fixture = Fixture::start(vec![exchange]).await;
            let refused = Trello
                .call(&cx(&fixture), "read_card", json!({"card": CARD}))
                .await;
            assert_eq!(refused, Err(expected));
            fixture.finish().await;
        }
        let limited = Exchange::new(
            "GET",
            &path,
            429,
            json!({"error": "API_TOKEN_LIMIT_EXCEEDED", "message": "Rate limit exceeded"}),
        )
        .header("retry-after", "600");
        let fixture = Fixture::start(vec![limited]).await;
        let refused = Trello
            .call(&cx(&fixture), "read_card", json!({"card": CARD}))
            .await;
        assert!(matches!(refused, Err(Error::Unavailable(_))));
        fixture.finish().await;
    }
}

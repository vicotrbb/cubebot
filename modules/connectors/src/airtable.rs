//! Airtable through a personal access token (https://airtable.com/developers/web/api/introduction,
//! https://support.airtable.com/docs/creating-personal-access-tokens). Only the bases listed in
//! the connection's settings are used: a call naming another is refused before any request.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Field, FieldKind, Level, Manifest,
    Risk, Step, ToolSpec,
};
use reqwest::{Method, RequestBuilder, Url};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const API: &str = "https://api.airtable.com/v0";
/// Airtable's largest page of records.
const LARGEST_PAGE: u8 = 100;
/// Airtable writes at most ten records per request.
const BATCH: usize = 10;

pub struct Airtable;

static MANIFEST: Manifest = Manifest {
    id: "airtable",
    name: "Airtable",
    description: "Read the schema and records of chosen Airtable bases, and create or update records.",
    category: Category::Work,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "Personal access token",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "pat…",
            help: "From airtable.com/create/tokens; shown only once.",
        }],
    }],
    config: &[Field {
        key: "bases",
        label: "Bases",
        kind: FieldKind::Lines,
        required: true,
        placeholder: "appXXXXXXXXXXXXXX",
        help: "The ids of the bases agents may use, one per line: the app… part of a base's URL.",
    }],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_bases",
            description: "The bases this connection may use, with their names.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_schema",
            description: "A base's tables with their fields and views.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"base":{"type":"string","description":"The base id, app…"}},"required":["base"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_records",
            description: "Records of a table, optionally through a view or filtered by an Airtable formula.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"base":{"type":"string"},"table":{"type":"string","description":"The table's name or id."},"formula":{"type":"string","description":"An Airtable formula, such as {Status} = 'Done'."},"view":{"type":"string","description":"A view's name or id; its filters and order apply."},"fields":{"type":"array","items":{"type":"string"},"description":"Only these fields."},"limit":{"type":"integer","minimum":1,"maximum":100},"offset":{"type":"string","description":"The next value of a previous answer."}},"required":["base","table"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "create_records",
            description: "Create up to 10 records in a table, each given as an object of field names to values.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"base":{"type":"string"},"table":{"type":"string"},"records":{"type":"array","minItems":1,"maxItems":10,"items":{"type":"object"}},"typecast":{"type":"boolean","description":"Let Airtable convert text to the field's type, such as a new select option."}},"required":["base","table","records"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "update_records",
            description: "Change fields of up to 10 records; fields left out keep their values.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"base":{"type":"string"},"table":{"type":"string"},"records":{"type":"array","minItems":1,"maxItems":10,"items":{"type":"object","properties":{"id":{"type":"string"},"fields":{"type":"object"}},"required":["id","fields"],"additionalProperties":false}},"typecast":{"type":"boolean"}},"required":["base","table","records"],"additionalProperties":false}"#,
        },
    ],
    events: &[],
    level: Level::Write,
    guide: &[
        Step {
            title: "Create a personal access token",
            body: "On Airtable's Builder hub choose Create token and name it, for example Cubebot.",
            link: Some("https://airtable.com/create/tokens"),
            copy: None,
        },
        Step {
            title: "Add its scopes",
            body: "With + Add a scope, add data.records:read, data.records:write and schema.bases:read. Add user.email:read too if you want the connection named by your email.",
            link: None,
            copy: Some("data.records:read data.records:write schema.bases:read"),
        },
        Step {
            title: "Grant it bases",
            body: "Under Access choose + Add a base for each base agents may use, then Create token. Copy the token, which Airtable shows only once, and paste it below.",
            link: None,
            copy: None,
        },
        Step {
            title: "List the bases here",
            body: "Paste each base's id, the part of its URL that starts with app, one per line under Bases. Agents can use only these, even if the token reaches more.",
            link: None,
            copy: None,
        },
    ],
    warning: Some(
        "Airtable allows 5 requests a second per base and answers more with a 30-second lockout; agents then get an error and must slow down. The token can do only what your own Airtable account can.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoInput {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Base {
    base: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    base: String,
    table: String,
    formula: Option<String>,
    view: Option<String>,
    #[serde(default)]
    fields: Vec<String>,
    limit: Option<u8>,
    offset: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    base: String,
    table: String,
    records: Vec<Map<String, Value>>,
    #[serde(default)]
    typecast: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    base: String,
    table: String,
    records: Vec<Change>,
    #[serde(default)]
    typecast: bool,
}

#[derive(serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    id: String,
    fields: Map<String, Value>,
}

impl Connector for Airtable {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let me = cx
                .json(request(cx, Method::GET, &format!("{API}/meta/whoami"))?)
                .await?;
            let name = me["email"]
                .as_str()
                .or(me["id"].as_str())
                .unwrap_or("Airtable account");
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "list_bases" => list_bases(cx, input).await,
                "read_schema" => read_schema(cx, input).await,
                "list_records" => list_records(cx, input).await,
                "create_records" => create_records(cx, input).await,
                "update_records" => update_records(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }
}

fn request(cx: &Context, method: Method, url: &str) -> Result<RequestBuilder, Error> {
    Ok(cx.request(method, url)?.bearer_auth(cx.token()?))
}

fn allowed(cx: &Context) -> Vec<&str> {
    let bases = cx.config()["bases"].as_array().map(Vec::as_slice);
    bases
        .unwrap_or_default()
        .iter()
        .filter_map(|base| base.as_str())
        .map(str::trim)
        .collect()
}

/// A base this connection lists; any other is refused without a request.
fn base<'a>(cx: &Context, base: &'a str) -> Result<&'a str, Error> {
    if allowed(cx).contains(&base) {
        return Ok(base);
    }
    Err(Error::Forbidden(format!(
        "{base:?} is not one of this connection's bases; list_bases names them"
    )))
}

/// The URL of a table's records, with its name escaped as one path segment.
fn table(base: &str, table: &str) -> Result<String, Error> {
    if table.is_empty() {
        return Err(Error::Invalid("name a table".into()));
    }
    let mut url = Url::parse(API).map_err(|_| Error::Invalid("not a valid address".into()))?;
    url.path_segments_mut()
        .map_err(|_| Error::Invalid("not a valid address".into()))?
        .extend([base, table]);
    Ok(url.into())
}

async fn list_bases(cx: &Context, input: Value) -> Result<Value, Error> {
    crate::input::<NoInput>(input)?;
    // shortcut: one page of up to 1000 bases; a granted base beyond it is shown without a name.
    let url = format!("{API}/meta/bases");
    let found = cx.json(request(cx, Method::GET, &url)?).await?;
    let seen = found["bases"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let bases: Vec<Value> = allowed(cx)
        .into_iter()
        .map(|id| match seen.iter().find(|base| base["id"] == id) {
            Some(base) => {
                json!({"id": id, "name": base["name"], "permission": base["permissionLevel"]})
            }
            None => json!({"id": id, "name": null, "permission": "not granted to the token"}),
        })
        .collect();
    Ok(json!({"bases": bases}))
}

async fn read_schema(cx: &Context, input: Value) -> Result<Value, Error> {
    let input: Base = crate::input(input)?;
    let url = format!("{API}/meta/bases/{}/tables", base(cx, &input.base)?);
    let found = cx.json(request(cx, Method::GET, &url)?).await?;
    let named =
        |item: &Value| json!({"id": item["id"], "name": item["name"], "type": item["type"]});
    let table = |table: &Value| {
        json!({
            "id": table["id"],
            "name": table["name"],
            "primary_field": table["primaryFieldId"],
            "fields": each(&table["fields"], named),
            "views": each(&table["views"], named),
        })
    };
    Ok(json!({"tables": each(&found["tables"], table)}))
}

async fn list_records(cx: &Context, input: Value) -> Result<Value, Error> {
    let list: List = crate::input(input)?;
    let url = table(base(cx, &list.base)?, &list.table)?;
    let limit = list.limit.unwrap_or(25).min(LARGEST_PAGE);
    let mut query = vec![("pageSize", limit.to_string())];
    query.extend(list.formula.map(|formula| ("filterByFormula", formula)));
    query.extend(list.view.map(|view| ("view", view)));
    query.extend(list.offset.map(|offset| ("offset", offset)));
    query.extend(list.fields.into_iter().map(|field| ("fields[]", field)));
    let found = cx
        .json(request(cx, Method::GET, &url)?.query(&query))
        .await?;
    Ok(json!({"records": records(&found), "next": found["offset"]}))
}

fn batch<T>(records: &[T]) -> Result<(), Error> {
    if (1..=BATCH).contains(&records.len()) {
        return Ok(());
    }
    Err(Error::Invalid(format!("give 1 to {BATCH} records")))
}

async fn create_records(cx: &Context, input: Value) -> Result<Value, Error> {
    let create: Create = crate::input(input)?;
    batch(&create.records)?;
    let url = table(base(cx, &create.base)?, &create.table)?;
    let rows: Vec<Value> = create
        .records
        .into_iter()
        .map(|fields| json!({"fields": fields}))
        .collect();
    let body = json!({"records": rows, "typecast": create.typecast});
    let made = cx
        .json(request(cx, Method::POST, &url)?.json(&body))
        .await?;
    Ok(json!({"records": records(&made)}))
}

async fn update_records(cx: &Context, input: Value) -> Result<Value, Error> {
    let update: Update = crate::input(input)?;
    batch(&update.records)?;
    let url = table(base(cx, &update.base)?, &update.table)?;
    let body = json!({"records": update.records, "typecast": update.typecast});
    let changed = cx
        .json(request(cx, Method::PATCH, &url)?.json(&body))
        .await?;
    Ok(json!({"records": records(&changed)}))
}

fn each(list: &Value, shown: impl Fn(&Value) -> Value) -> Vec<Value> {
    list.as_array().into_iter().flatten().map(shown).collect()
}

fn records(found: &Value) -> Vec<Value> {
    let record = |record: &Value| json!({"id": record["id"], "created": record["createdTime"], "fields": record["fields"]});
    each(&found["records"], record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    const BASE: &str = "appLkNDICXNqxSDhG";

    fn cx(fixture: &Fixture) -> Context {
        let credential = Credential::Fields {
            fields: [("token".into(), "patAirtableSecret".into())].into(),
        };
        let config = json!({"bases": [BASE, "appMissingBase0001"]});
        Context::new(config, credential).with_origin(&fixture.origin())
    }

    fn recorded(name: &str) -> Value {
        let path = format!(
            "{}/fixtures/airtable/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn bases_and_schema_are_read_for_listed_bases_only() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api.airtable.com/v0/meta/bases",
                200,
                recorded("bases"),
            ),
            Exchange::new(
                "GET",
                &format!("/api.airtable.com/v0/meta/bases/{BASE}/tables"),
                200,
                recorded("tables"),
            ),
        ])
        .await;
        let cx = cx(&fixture);
        let bases = Airtable.call(&cx, "list_bases", json!({})).await.unwrap();
        assert_eq!(
            bases["bases"],
            json!([
                {"id": BASE, "name": "Apartment Hunting", "permission": "create"},
                {"id": "appMissingBase0001", "name": null, "permission": "not granted to the token"},
            ])
        );
        let schema = Airtable
            .call(&cx, "read_schema", json!({"base": BASE}))
            .await
            .unwrap();
        assert_eq!(schema["tables"][0]["name"], "Apartments");
        assert_eq!(
            schema["tables"][0]["fields"][1],
            json!({"id": "fldoaIqdn5szURHpw", "name": "Pictures", "type": "multipleAttachments"})
        );
        let other = Airtable
            .call(&cx, "read_schema", json!({"base": "appOtherBase00000"}))
            .await;
        assert!(matches!(other, Err(Error::Forbidden(_))));
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].header("authorization"),
            Some("Bearer patAirtableSecret")
        );
    }

    #[tokio::test]
    async fn records_are_listed_with_a_formula_and_paged() {
        let fixture = Fixture::start(vec![Exchange::new(
            "GET",
            &format!(
                "/api.airtable.com/v0/{BASE}/Design%20projects?pageSize=2&filterByFormula=%7BStatus%7D+%3D+%27Done%27&offset=itrA&fields%5B%5D=Name"
            ),
            200,
            recorded("records"),
        )])
        .await;
        let input = json!({"base": BASE, "table": "Design projects", "formula": "{Status} = 'Done'", "fields": ["Name"], "limit": 2, "offset": "itrA"});
        let found = Airtable
            .call(&cx(&fixture), "list_records", input)
            .await
            .unwrap();
        assert_eq!(found["next"], "itrHmXmMyZ2Ux8vXt/recSUCzSVb6LDb4V7");
        assert_eq!(
            found["records"][0],
            json!({"id": "rec560UJdUtocSouk", "created": "2026-10-02T01:16:49.000Z", "fields": {"Name": "Coffee packaging"}})
        );
        fixture.finish().await;
    }

    #[tokio::test]
    async fn records_are_created_and_updated_in_small_batches() {
        let path = format!("/api.airtable.com/v0/{BASE}/tblQ0q8Bl2KBdbyFT");
        let fixture = Fixture::start(vec![
            Exchange::new("POST", &path, 200, recorded("records")),
            Exchange::new("PATCH", &path, 200, recorded("records")),
        ])
        .await;
        let cx = cx(&fixture);
        let create = json!({"base": BASE, "table": "tblQ0q8Bl2KBdbyFT", "records": [{"Name": "Coffee packaging"}], "typecast": true});
        let made = Airtable.call(&cx, "create_records", create).await.unwrap();
        assert_eq!(made["records"][0]["id"], "rec560UJdUtocSouk");
        let update = json!({"base": BASE, "table": "tblQ0q8Bl2KBdbyFT", "records": [{"id": "rec560UJdUtocSouk", "fields": {"Status": "Done"}}]});
        Airtable.call(&cx, "update_records", update).await.unwrap();
        let eleven: Vec<Value> = (0..11).map(|_| json!({"Name": "x"})).collect();
        let many = json!({"base": BASE, "table": "tblQ0q8Bl2KBdbyFT", "records": eleven});
        let refused = Airtable.call(&cx, "create_records", many).await;
        assert!(matches!(refused, Err(Error::Invalid(_))));
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].json(),
            json!({"records": [{"fields": {"Name": "Coffee packaging"}}], "typecast": true})
        );
        assert_eq!(
            seen[1].json(),
            json!({"records": [{"id": "rec560UJdUtocSouk", "fields": {"Status": "Done"}}], "typecast": false})
        );
    }

    #[tokio::test]
    async fn bad_input_is_refused_before_any_request() {
        let fixture = Fixture::start(Vec::new()).await;
        let cx = cx(&fixture);
        for (tool, input) in [
            (
                "list_records",
                json!({"base": "appOtherBase00000", "table": "T"}),
            ),
            ("list_records", json!({"base": BASE, "table": ""})),
            (
                "list_records",
                json!({"base": BASE, "table": "T", "sort": []}),
            ),
            (
                "update_records",
                json!({"base": BASE, "table": "T", "records": [{"id": "rec1"}]}),
            ),
        ] {
            let refused = Airtable.call(&cx, tool, input.clone()).await;
            assert!(
                matches!(refused, Err(Error::Invalid(_) | Error::Forbidden(_))),
                "{input}"
            );
        }
        assert!(fixture.finish().await.is_empty());
    }

    #[tokio::test]
    async fn refusals_map_to_errors() {
        let path = format!("/api.airtable.com/v0/meta/bases/{BASE}/tables");
        let cases = [
            (
                401,
                json!({"error": {"type": "AUTHENTICATION_REQUIRED", "message": "Authentication required"}}),
                Error::Unauthorized,
            ),
            (
                403,
                json!({"error": {"type": "INVALID_PERMISSIONS_OR_MODEL_NOT_FOUND", "message": "Invalid permissions, or the requested model was not found."}}),
                Error::Forbidden(
                    "Invalid permissions, or the requested model was not found.".into(),
                ),
            ),
            (
                404,
                json!({"error": "NOT_FOUND"}),
                Error::NotFound("NOT_FOUND".into()),
            ),
            (
                422,
                json!({"error": {"type": "INVALID_VALUE_FOR_COLUMN", "message": "Field \"Status\" cannot accept the provided value"}}),
                Error::Rejected("Field \"Status\" cannot accept the provided value".into()),
            ),
        ];
        for (status, body, expected) in cases {
            let fixture = Fixture::start(vec![Exchange::new("GET", &path, status, body)]).await;
            let refused = Airtable
                .call(&cx(&fixture), "read_schema", json!({"base": BASE}))
                .await;
            assert_eq!(refused, Err(expected));
            fixture.finish().await;
        }
        let limited = Exchange::new(
            "GET",
            &path,
            429,
            json!({"errors": [{"error": "RATE_LIMIT_REACHED", "message": "Rate limit exceeded. Please try again later"}]}),
        )
        .header("retry-after", "30");
        let fixture = Fixture::start(vec![limited]).await;
        let refused = Airtable
            .call(&cx(&fixture), "read_schema", json!({"base": BASE}))
            .await;
        assert!(matches!(refused, Err(Error::Unavailable(_))));
        fixture.finish().await;
    }
}

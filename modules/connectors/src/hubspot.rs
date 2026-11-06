//! HubSpot CRM through a service key or a legacy private app's token, on the date-versioned
//! 2026-09 APIs (https://developers.hubspot.com/docs/api-reference/latest/overview). Legacy
//! private apps can no longer be created since October 2026; service keys replace them
//! (https://developers.hubspot.com/docs/apps/developer-platform/build-apps/authentication/account-service-keys).
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Field, FieldKind, Level, Manifest,
    Risk, Step, ToolSpec,
};
use reqwest::{Method, RequestBuilder};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const API: &str = "https://api.hubapi.com";
/// Pinned so a HubSpot release cannot change the answers under us; supported 18 months at least.
const VERSION: &str = "2026-09";
/// HubSpot's search answers at most 200 records a page.
const LARGEST_PAGE: u8 = 200;
const LONGEST: usize = 8_000;

pub struct Hubspot;

static MANIFEST: Manifest = Manifest {
    id: "hubspot",
    name: "HubSpot",
    description: "Search and read HubSpot contacts, companies and deals, update them and add notes.",
    category: Category::Work,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "Service key",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "pat-na1-…",
            help: "From Development › Keys › Service keys. A legacy private app's access token works too.",
        }],
    }],
    config: &[],
    products: &[],
    tools: &[
        ToolSpec {
            name: "search",
            description: "Find contacts, companies or deals by text, or with HubSpot filter groups. Answers chosen properties.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"object":{"type":"string","enum":["contact","company","deal"]},"query":{"type":"string","description":"Text matched against the default searchable properties, such as name, email or domain."},"filters":{"type":"array","items":{"type":"object"},"description":"HubSpot filterGroups, such as [{\"filters\":[{\"propertyName\":\"dealstage\",\"operator\":\"EQ\",\"value\":\"closedwon\"}]}]."},"properties":{"type":"array","items":{"type":"string"},"description":"Properties to answer; a few common ones when absent."},"limit":{"type":"integer","minimum":1,"maximum":200},"after":{"type":"string","description":"The next value of a previous answer."}},"required":["object"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_record",
            description: "One contact, company or deal with its properties and the ids of associated contacts, companies, deals and notes.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"object":{"type":"string","enum":["contact","company","deal"]},"id":{"type":"string"},"properties":{"type":"array","items":{"type":"string"}}},"required":["object","id"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "create_record",
            description: "Create a contact, company or deal from property values, such as email and firstname or dealname and amount.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"object":{"type":"string","enum":["contact","company","deal"]},"properties":{"type":"object"}},"required":["object","properties"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "update_record",
            description: "Change property values of a contact, company or deal; an empty string clears one.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"object":{"type":"string","enum":["contact","company","deal"]},"id":{"type":"string"},"properties":{"type":"object"}},"required":["object","id","properties"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "add_note",
            description: "Add an internal note to a contact, company or deal's timeline.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"object":{"type":"string","enum":["contact","company","deal"]},"id":{"type":"string"},"text":{"type":"string","description":"The note; simple HTML is kept."}},"required":["object","id","text"],"additionalProperties":false}"#,
        },
    ],
    events: &[],
    level: Level::Write,
    guide: &[
        Step {
            title: "Create a service key",
            body: "In HubSpot go to Development › Keys › Service keys and choose Create service key. Only super admins and users with the Developer tools access permission can. Name it, for example Cubebot.",
            link: Some(
                "https://developers.hubspot.com/docs/apps/developer-platform/build-apps/authentication/account-service-keys",
            ),
            copy: None,
        },
        Step {
            title: "Add its scopes",
            body: "Choose Add new scope, tick the scopes below, then Update, Create, and confirm. Leave out the write scopes for a read-only connection.",
            link: None,
            copy: Some(
                "crm.objects.contacts.read crm.objects.contacts.write crm.objects.companies.read crm.objects.companies.write crm.objects.deals.read crm.objects.deals.write",
            ),
        },
        Step {
            title: "Copy the key",
            body: "Open the key, choose Show and then Copy, and paste it below. An existing legacy private app's access token (Development › Legacy apps › the app › Auth › Show token) works the same way.",
            link: None,
            copy: None,
        },
    ],
    warning: Some(
        "If you switch on the key's automatic rotation, HubSpot replaces it every 3, 6 or 12 months and the old one stops working: paste the new key here each time.",
    ),
};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Object {
    Contact,
    Company,
    Deal,
}

impl Object {
    fn path(self) -> &'static str {
        match self {
            Self::Contact => "contacts",
            Self::Company => "companies",
            Self::Deal => "deals",
        }
    }

    /// Properties answered when none are asked for.
    fn common(self) -> &'static [&'static str] {
        match self {
            Self::Contact => &[
                "firstname",
                "lastname",
                "email",
                "phone",
                "company",
                "jobtitle",
            ],
            Self::Company => &["name", "domain", "city", "country", "industry"],
            Self::Deal => &["dealname", "amount", "dealstage", "pipeline", "closedate"],
        }
    }

    /// HubSpot's own association type from a note to this object.
    fn note_association(self) -> u16 {
        match self {
            Self::Contact => 202,
            Self::Company => 190,
            Self::Deal => 214,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    object: Object,
    query: Option<String>,
    filters: Option<Vec<Value>>,
    properties: Option<Vec<String>>,
    limit: Option<u8>,
    after: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    object: Object,
    id: String,
    properties: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Write {
    object: Object,
    id: Option<String>,
    properties: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Note {
    object: Object,
    id: String,
    text: String,
}

impl Connector for Hubspot {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let path = format!("/account-info/{VERSION}/details");
            let account = cx.json(request(cx, Method::GET, &path)?).await?;
            let name = match (account["uiDomain"].as_str(), &account["portalId"]) {
                (_, Value::Number(portal)) => format!("HubSpot account {portal}"),
                (Some(domain), _) => domain.to_owned(),
                _ => "HubSpot account".to_owned(),
            };
            Ok(Account { name })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "search" => search(cx, input).await,
                "read_record" => read_record(cx, input).await,
                "create_record" => write_record(cx, input, false).await,
                "update_record" => write_record(cx, input, true).await,
                "add_note" => add_note(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }
}

fn request(cx: &Context, method: Method, path: &str) -> Result<RequestBuilder, Error> {
    Ok(cx
        .request(method, &format!("{API}{path}"))?
        .bearer_auth(cx.token()?))
}

fn objects(object: Object) -> String {
    format!("/crm/objects/{VERSION}/{}", object.path())
}

/// A record id: HubSpot's are decimal numbers.
fn id(text: &str) -> Result<&str, Error> {
    if (1..=20).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(text);
    }
    Err(Error::Invalid(format!(
        "{text:?} is not a HubSpot record id"
    )))
}

fn properties(object: Object, asked: Option<Vec<String>>) -> Vec<String> {
    asked.unwrap_or_else(|| object.common().iter().map(|p| p.to_string()).collect())
}

async fn search(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: Search = crate::input(input)?;
    let mut body = json!({
        "properties": properties(search.object, search.properties),
        "limit": search.limit.unwrap_or(20).min(LARGEST_PAGE),
    });
    if let Some(query) = search.query {
        body["query"] = json!(query);
    }
    if let Some(filters) = search.filters {
        body["filterGroups"] = json!(filters);
    }
    if let Some(after) = search.after {
        body["after"] = json!(after);
    }
    let path = format!("{}/search", objects(search.object));
    let found = cx
        .json(request(cx, Method::POST, &path)?.json(&body))
        .await?;
    let results: Vec<Value> = found["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(record)
        .collect();
    Ok(json!({
        "total": found["total"],
        "results": results,
        "next": found["paging"]["next"]["after"],
    }))
}

async fn read_record(cx: &Context, input: Value) -> Result<Value, Error> {
    let read: Read = crate::input(input)?;
    let path = format!("{}/{}", objects(read.object), id(&read.id)?);
    let query = [
        (
            "properties",
            properties(read.object, read.properties).join(","),
        ),
        ("associations", "contacts,companies,deals,notes".to_owned()),
    ];
    let found = cx
        .json(request(cx, Method::GET, &path)?.query(&query))
        .await?;
    let mut shown = record(&found);
    shown["associations"] = associations(&found["associations"]);
    Ok(shown)
}

async fn write_record(cx: &Context, input: Value, existing: bool) -> Result<Value, Error> {
    let write: Write = crate::input(input)?;
    let (method, path) = match (existing, write.id.as_deref()) {
        (true, Some(record)) => (
            Method::PATCH,
            format!("{}/{}", objects(write.object), id(record)?),
        ),
        (false, None) => (Method::POST, objects(write.object)),
        (true, None) => return Err(Error::Invalid("name the record to change".into())),
        (false, Some(_)) => return Err(Error::Invalid("a new record has no id".into())),
    };
    let body = json!({"properties": write.properties});
    let written = cx.json(request(cx, method, &path)?.json(&body)).await?;
    Ok(record(&written))
}

async fn add_note(cx: &Context, input: Value) -> Result<Value, Error> {
    let note: Note = crate::input(input)?;
    let to = id(&note.id)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let body = json!({
        "properties": {"hs_note_body": note.text, "hs_timestamp": now.to_string()},
        "associations": [{
            "to": {"id": to},
            "types": [{"associationCategory": "HUBSPOT_DEFINED", "associationTypeId": note.object.note_association()}],
        }],
    });
    let path = format!("/crm/objects/{VERSION}/notes");
    let made = cx
        .json(request(cx, Method::POST, &path)?.json(&body))
        .await?;
    Ok(json!({"id": made["id"], "created": made["createdAt"]}))
}

/// A record as the model needs it; long text such as a description is cut short.
fn record(record: &Value) -> Value {
    let properties: Map<String, Value> = record["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, value)| {
            let value = match value.as_str() {
                Some(text) if text.chars().count() > LONGEST => {
                    json!(format!(
                        "{}…[cut short]",
                        text.chars().take(LONGEST).collect::<String>()
                    ))
                }
                _ => value.clone(),
            };
            (name.clone(), value)
        })
        .collect();
    json!({
        "id": record["id"],
        "properties": properties,
        "updated": record["updatedAt"],
        "url": record["url"],
    })
}

/// Associated ids by object type, each once whatever its association labels.
fn associations(found: &Value) -> Value {
    let grouped = found.as_object().into_iter().flatten().map(|(kind, list)| {
        let mut ids: Vec<&str> = list["results"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item["id"].as_str())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        (kind.clone(), json!(ids))
    });
    Value::Object(grouped.collect())
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
            fields: [("token".into(), "pat-na1-hubspot-secret".into())].into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    fn recorded(name: &str) -> Value {
        let path = format!(
            "{}/fixtures/hubspot/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn search_sends_text_and_filters_and_pages_with_after() {
        let fixture = Fixture::start(vec![Exchange::new(
            "POST",
            "/api.hubapi.com/crm/objects/2026-09/deals/search",
            200,
            recorded("search"),
        )])
        .await;
        let filters = json!([{"filters": [{"propertyName": "dealstage", "operator": "EQ", "value": "closedwon"}]}]);
        let input = json!({"object": "deal", "query": "Acme", "filters": filters, "limit": 10, "after": "20"});
        let found = Hubspot.call(&cx(&fixture), "search", input).await.unwrap();
        assert_eq!(found["total"], 2);
        assert_eq!(found["next"], "30");
        assert_eq!(
            found["results"][0]["properties"]["dealname"],
            "Acme renewal"
        );
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].header("authorization"),
            Some("Bearer pat-na1-hubspot-secret")
        );
        assert_eq!(
            seen[0].json(),
            json!({
                "properties": ["dealname", "amount", "dealstage", "pipeline", "closedate"],
                "limit": 10,
                "query": "Acme",
                "filterGroups": filters,
                "after": "20",
            })
        );
    }

    #[tokio::test]
    async fn read_record_collapses_associations_to_ids() {
        let fixture = Fixture::start(vec![Exchange::new(
            "GET",
            "/api.hubapi.com/crm/objects/2026-09/contacts/151?properties=email%2Cfirstname&associations=contacts%2Ccompanies%2Cdeals%2Cnotes",
            200,
            recorded("contact"),
        )])
        .await;
        let input = json!({"object": "contact", "id": "151", "properties": ["email", "firstname"]});
        let read = Hubspot
            .call(&cx(&fixture), "read_record", input)
            .await
            .unwrap();
        assert_eq!(read["properties"]["email"], "bh@hubspot.com");
        assert_eq!(
            read["associations"],
            json!({"companies": ["2341"], "deals": ["8801", "8802"]})
        );
        fixture.finish().await;
    }

    #[tokio::test]
    async fn records_are_created_updated_and_noted() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "POST",
                "/api.hubapi.com/crm/objects/2026-09/companies",
                201,
                recorded("company"),
            ),
            Exchange::new(
                "PATCH",
                "/api.hubapi.com/crm/objects/2026-09/companies/2341",
                200,
                recorded("company"),
            ),
            Exchange::new(
                "POST",
                "/api.hubapi.com/crm/objects/2026-09/notes",
                201,
                json!({"id": "17147287858", "properties": {"hs_note_body": "Called"}, "createdAt": "2026-11-05T10:00:00.000Z", "updatedAt": "2026-11-05T10:00:00.000Z", "archived": false}),
            ),
        ])
        .await;
        let cx = cx(&fixture);
        let create =
            json!({"object": "company", "properties": {"name": "Acme", "domain": "acme.example"}});
        let made = Hubspot.call(&cx, "create_record", create).await.unwrap();
        assert_eq!(made["id"], "2341");
        let update =
            json!({"object": "company", "id": "2341", "properties": {"city": "Porto Alegre"}});
        Hubspot.call(&cx, "update_record", update).await.unwrap();
        let note = json!({"object": "company", "id": "2341", "text": "Called"});
        let noted = Hubspot.call(&cx, "add_note", note).await.unwrap();
        assert_eq!(noted["id"], "17147287858");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].json(),
            json!({"properties": {"name": "Acme", "domain": "acme.example"}})
        );
        assert_eq!(
            seen[1].json(),
            json!({"properties": {"city": "Porto Alegre"}})
        );
        let sent = seen[2].json();
        assert_eq!(sent["properties"]["hs_note_body"], "Called");
        assert!(
            sent["properties"]["hs_timestamp"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .is_ok()
        );
        assert_eq!(
            sent["associations"],
            json!([{"to": {"id": "2341"}, "types": [{"associationCategory": "HUBSPOT_DEFINED", "associationTypeId": 190}]}])
        );
    }

    #[tokio::test]
    async fn bad_input_is_refused_before_any_request() {
        let fixture = Fixture::start(Vec::new()).await;
        let cx = cx(&fixture);
        for (tool, input) in [
            (
                "read_record",
                json!({"object": "contact", "id": "../owners"}),
            ),
            ("read_record", json!({"object": "ticket", "id": "1"})),
            ("search", json!({"object": "deal", "sorts": []})),
            ("update_record", json!({"object": "deal", "properties": {}})),
            (
                "add_note",
                json!({"object": "deal", "id": "x1", "text": "Hi"}),
            ),
        ] {
            let refused = Hubspot.call(&cx, tool, input.clone()).await;
            assert!(matches!(refused, Err(Error::Invalid(_))), "{input}");
        }
        assert!(fixture.finish().await.is_empty());
    }

    #[tokio::test]
    async fn refusals_map_to_errors() {
        let path = "/api.hubapi.com/crm/objects/2026-09/deals/9";
        let error = |category: &str, message: &str| json!({"status": "error", "message": message, "correlationId": "aeb5f871-7f07-4993-9211-075dc63e7cbf", "category": category});
        let missing = "This app hasn't been granted all required scopes to make this call.";
        let cases = [
            (
                401,
                error(
                    "INVALID_AUTHENTICATION",
                    "Authentication credentials not found.",
                ),
                Error::Unauthorized,
            ),
            (
                403,
                error("MISSING_SCOPES", missing),
                Error::Forbidden(missing.into()),
            ),
            (
                404,
                error("OBJECT_NOT_FOUND", "Object not found."),
                Error::NotFound("Object not found.".into()),
            ),
            (
                400,
                error("VALIDATION_ERROR", "Property values were not valid"),
                Error::Rejected("Property values were not valid".into()),
            ),
        ];
        let input = json!({"object": "deal", "id": "9"});
        for (status, body, expected) in cases {
            let fixture = Fixture::start(vec![Exchange::new("GET", path, status, body)]).await;
            let refused = Hubspot
                .call(&cx(&fixture), "read_record", input.clone())
                .await;
            assert_eq!(refused, Err(expected));
            fixture.finish().await;
        }
        let limited = Exchange::new(
            "GET",
            path,
            429,
            error("RATE_LIMITS", "You have reached your ten secondly limit."),
        )
        .header("retry-after", "60");
        let fixture = Fixture::start(vec![limited]).await;
        let refused = Hubspot.call(&cx(&fixture), "read_record", input).await;
        assert!(matches!(refused, Err(Error::Unavailable(_))));
        fixture.finish().await;
    }
}

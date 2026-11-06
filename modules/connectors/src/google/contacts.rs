//! Google Contacts through the People API
//! (https://developers.google.com/people/api/rest/v1/people/searchContacts).
use crate::{Context, Error};
use serde::Deserialize;
use serde_json::{Value, json};

const SEARCH: &str = "https://people.googleapis.com/v1/people:searchContacts";
const FIELDS: &str = "names,emailAddresses,phoneNumbers";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: String,
    limit: Option<u8>,
}

/// Google answers a search from a cache it fills on an empty-query request, so one goes first.
pub(super) async fn search(cx: &Context, input: Value) -> Result<Value, Error> {
    let search: Search = crate::input(input)?;
    let warm = cx
        .get(SEARCH)?
        .query(&[("query", ""), ("readMask", FIELDS)]);
    cx.json(warm).await?;
    let limit = search.limit.unwrap_or(10).clamp(1, 30).to_string();
    let request = cx.get(SEARCH)?.query(&[
        ("query", search.query.as_str()),
        ("readMask", FIELDS),
        ("pageSize", &limit),
    ]);
    let found = cx.json(request).await?;
    let contacts: Vec<Value> = found["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|result| contact(&result["person"]))
        .collect();
    Ok(json!({"contacts": contacts}))
}

fn contact(person: &Value) -> Value {
    let values = |field: &str| -> Vec<Value> {
        person[field]
            .as_array()
            .into_iter()
            .flatten()
            .map(|item| item["value"].clone())
            .collect()
    };
    json!({
        "id": person["resourceName"],
        "name": person["names"][0]["displayName"],
        "emails": values("emailAddresses"),
        "phones": values("phoneNumbers"),
    })
}

//! Stripe through a restricted API key (https://docs.stripe.com/api, https://docs.stripe.com/keys,
//! https://docs.stripe.com/keys/restricted-api-keys). Amounts are integers in the currency's
//! smallest unit, as Stripe gives them. Events stay listable for 30 days.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Event, EventSpec, Field, FieldKind,
    Level, Manifest, Poll, Risk, Step, ToolSpec,
};
use reqwest::{Method, RequestBuilder};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const API: &str = "https://api.stripe.com/v1";
/// Pinned so an account's default API version cannot change the answers under us.
const VERSION: &str = "2025-03-31.basil";
const SUCCEEDED: &str = "payment_intent.succeeded";
const FAILED: &str = "payment_intent.payment_failed";
/// Events each poll reads at most.
const POLLED: &str = "50";

pub struct Stripe;

static MANIFEST: Manifest = Manifest {
    id: "stripe",
    name: "Stripe",
    description: "Look up Stripe customers, payments, invoices and subscriptions, refund, and hear of payments.",
    category: Category::Finance,
    auth: &[Auth::ApiKey {
        fields: &[Field {
            key: "token",
            label: "Restricted API key",
            kind: FieldKind::Secret,
            required: true,
            placeholder: "rk_live_…",
            help: "A restricted key from Developers › API keys. A secret sk_ key also works but can do anything.",
        }],
    }],
    config: &[],
    products: &[],
    tools: &[
        ToolSpec {
            name: "balance",
            description: "The account's available and pending balance per currency.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "search_customers",
            description: "Customers by exact email, or with Stripe search syntax such as name~'ana'; newest first.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"email":{"type":"string"},"query":{"type":"string","description":"Stripe search syntax, such as email:'ana@example.com' or metadata['plan']:'pro'."},"limit":{"type":"integer","minimum":1,"maximum":100},"next":{"type":"string","description":"The next cursor of a previous answer."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_payments",
            description: "Payments (payment intents), newest first, optionally of one customer.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"customer":{"type":"string","description":"A cus_ id."},"limit":{"type":"integer","minimum":1,"maximum":100},"next":{"type":"string","description":"The next cursor of a previous answer."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_payment",
            description: "One payment intent (pi_) or charge (ch_): amount, status, customer and failure reason.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"payment":{"type":"string","description":"A pi_ or ch_ id."}},"required":["payment"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_invoices",
            description: "Invoices, newest first, optionally of one customer or status.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"customer":{"type":"string"},"status":{"type":"string","enum":["draft","open","paid","uncollectible","void"]},"limit":{"type":"integer","minimum":1,"maximum":100},"next":{"type":"string"}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_subscriptions",
            description: "Subscriptions, optionally of one customer or status, with their prices and renewal dates.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"customer":{"type":"string"},"status":{"type":"string","enum":["active","past_due","unpaid","canceled","incomplete","incomplete_expired","trialing","paused","all","ended"]},"limit":{"type":"integer","minimum":1,"maximum":100},"next":{"type":"string"}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "create_refund",
            description: "Refund a payment in full, or part of it with amount in the smallest currency unit. Money goes back to the customer.",
            risk: Risk::Send,
            product: None,
            input: r#"{"type":"object","properties":{"payment":{"type":"string","description":"A pi_ or ch_ id."},"amount":{"type":"integer","minimum":1,"description":"In the smallest unit, such as cents; the whole remaining amount when absent."},"reason":{"type":"string","enum":["duplicate","fraudulent","requested_by_customer"]}},"required":["payment"],"additionalProperties":false}"#,
        },
    ],
    events: &[
        EventSpec {
            kind: "payment.succeeded",
            description: "A payment succeeded.",
            product: None,
        },
        EventSpec {
            kind: "payment.failed",
            description: "A payment attempt failed.",
            product: None,
        },
    ],
    level: Level::Read,
    guide: &[
        Step {
            title: "Create a restricted key",
            body: "In the Stripe Dashboard open Developers › API keys and choose Create restricted key. Start without permissions and name it Cubebot.",
            link: Some("https://dashboard.stripe.com/apikeys"),
            copy: None,
        },
        Step {
            title: "Grant read permissions",
            body: "Set Read on: Balance, Customers, Charges, PaymentIntents, Invoices, Subscriptions and Events. Leave everything else at None.",
            link: None,
            copy: None,
        },
        Step {
            title: "Only to refund: grant Refunds",
            body: "If agents should refund, set Write on Refunds, and give the connection the Full level. Otherwise leave it at None.",
            link: None,
            copy: None,
        },
        Step {
            title: "Copy the key",
            body: "Choose Create key, confirm the code Stripe sends, copy the key (it starts with rk_live_ or rk_test_) and paste it below. Stripe never shows it again.",
            link: None,
            copy: None,
        },
    ],
    warning: Some(
        "Live and test (sandbox) keys see different data: use a live key for real payments. Prefer a restricted key; a secret sk_ key lets agents do anything in the account. Refunds move real money and cannot be undone.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Customers {
    email: Option<String>,
    query: Option<String>,
    limit: Option<u8>,
    next: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Listing {
    customer: Option<String>,
    status: Option<String>,
    limit: Option<u8>,
    next: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Payment {
    payment: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Refund {
    payment: String,
    amount: Option<u64>,
    reason: Option<String>,
}

impl Connector for Stripe {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    /// A restricted key may not read the account, so the balance proves it works then.
    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let name = match cx.json(request(cx, Method::GET, "/account")?).await {
                Ok(account) => named(&account),
                Err(Error::Forbidden(_) | Error::Rejected(_)) => None,
                Err(error) => return Err(error),
            };
            let name = match name {
                Some(name) => name,
                None => {
                    let balance = cx.json(request(cx, Method::GET, "/balance")?).await?;
                    let mode = if balance["livemode"] == true {
                        "live"
                    } else {
                        "test"
                    };
                    format!("Stripe account ({mode})")
                }
            };
            Ok(Account { name })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "balance" => balance(cx, input).await,
                "search_customers" => customers(cx, input).await,
                "list_payments" => listing(cx, input, "/payment_intents", payment).await,
                "read_payment" => read_payment(cx, input).await,
                "list_invoices" => listing(cx, input, "/invoices", invoice).await,
                "list_subscriptions" => listing(cx, input, "/subscriptions", subscription).await,
                "create_refund" => refund(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(poll(cx, cursor))
    }
}

fn request(cx: &Context, method: Method, path: &str) -> Result<RequestBuilder, Error> {
    Ok(cx
        .request(method, &format!("{API}{path}"))?
        .bearer_auth(cx.token()?)
        .header("stripe-version", VERSION))
}

fn named(account: &Value) -> Option<String> {
    let candidates = [
        &account["settings"]["dashboard"]["display_name"],
        &account["business_profile"]["name"],
        &account["email"],
        &account["id"],
    ];
    candidates
        .into_iter()
        .find_map(|name| name.as_str().filter(|name| !name.is_empty()))
        .map(str::to_owned)
}

/// A Stripe id with one of the given prefixes, so it stays one path segment.
fn id<'a>(text: &'a str, prefixes: &[&str]) -> Result<&'a str, Error> {
    let text = text.trim();
    let plain = text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    match plain && text.len() <= 255 && prefixes.iter().any(|prefix| text.starts_with(prefix)) {
        true => Ok(text),
        false => Err(Error::Invalid(format!(
            "{text:?} is not an id starting with {}",
            prefixes.join(" or ")
        ))),
    }
}

async fn balance(cx: &Context, input: Value) -> Result<Value, Error> {
    crate::input::<Empty>(input)?;
    let found = cx.json(request(cx, Method::GET, "/balance")?).await?;
    let amounts = |list: &Value| -> Vec<Value> {
        list.as_array()
            .into_iter()
            .flatten()
            .map(|amount| json!({"amount": amount["amount"], "currency": amount["currency"]}))
            .collect()
    };
    Ok(json!({
        "available": amounts(&found["available"]),
        "pending": amounts(&found["pending"]),
        "live": found["livemode"],
    }))
}

async fn customers(cx: &Context, input: Value) -> Result<Value, Error> {
    let wanted: Customers = crate::input(input)?;
    let limit = wanted.limit.unwrap_or(20).clamp(1, 100).to_string();
    let mut query = vec![("limit", limit.as_str())];
    let path = match &wanted.query {
        Some(search) => {
            query.push(("query", search));
            query.extend(wanted.next.as_deref().map(|page| ("page", page)));
            "/customers/search"
        }
        None => {
            query.extend(wanted.email.as_deref().map(|email| ("email", email)));
            query.extend(
                wanted
                    .next
                    .as_deref()
                    .map(|after| ("starting_after", after)),
            );
            "/customers"
        }
    };
    let found = cx
        .json(request(cx, Method::GET, path)?.query(&query))
        .await?;
    Ok(page(&found, customer))
}

/// A list or search page; `next` is the search's page token or the list's last id.
fn page(found: &Value, shown: fn(&Value) -> Value) -> Value {
    let items = found["data"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let next = match (found["has_more"] == true, found["next_page"].as_str()) {
        (true, Some(page)) => json!(page),
        (true, None) => items.last().map_or(Value::Null, |last| last["id"].clone()),
        (false, _) => Value::Null,
    };
    json!({"items": items.iter().map(shown).collect::<Vec<_>>(), "next": next})
}

async fn listing(
    cx: &Context,
    input: Value,
    path: &str,
    shown: fn(&Value) -> Value,
) -> Result<Value, Error> {
    let wanted: Listing = crate::input(input)?;
    if wanted.status.is_some() && path == "/payment_intents" {
        return Err(Error::Invalid(
            "payments cannot be filtered by status".into(),
        ));
    }
    let limit = wanted.limit.unwrap_or(20).clamp(1, 100).to_string();
    let mut query = vec![("limit", limit.as_str())];
    let filters = [
        ("customer", &wanted.customer),
        ("status", &wanted.status),
        ("starting_after", &wanted.next),
    ];
    query.extend(
        filters
            .iter()
            .filter_map(|(name, value)| Some((*name, value.as_deref()?))),
    );
    let found = cx
        .json(request(cx, Method::GET, path)?.query(&query))
        .await?;
    Ok(page(&found, shown))
}

async fn read_payment(cx: &Context, input: Value) -> Result<Value, Error> {
    let wanted = crate::input::<Payment>(input)?.payment;
    let wanted = id(&wanted, &["pi_", "ch_", "py_"])?;
    let path = match wanted.starts_with("pi_") {
        true => format!("/payment_intents/{wanted}"),
        false => format!("/charges/{wanted}"),
    };
    let found = cx.json(request(cx, Method::GET, &path)?).await?;
    let mut shown = payment(&found);
    shown["refunded"] = found["amount_refunded"].clone();
    shown["failure"] = found["last_payment_error"]["message"]
        .as_str()
        .or(found["failure_message"].as_str())
        .map_or(Value::Null, |message| json!(message));
    shown["receipt_url"] = found["receipt_url"].clone();
    Ok(shown)
}

/// Refunds move money, so a retried call must not refund twice: the same payment, amount and
/// reason within Stripe's 24 hours of idempotency answer with the first refund.
async fn refund(cx: &Context, input: Value) -> Result<Value, Error> {
    let wanted: Refund = crate::input(input)?;
    let payment = id(&wanted.payment, &["pi_", "ch_", "py_"])?;
    let target = if payment.starts_with("pi_") {
        "payment_intent"
    } else {
        "charge"
    };
    let amount = wanted.amount.map(|amount| amount.to_string());
    let mut form = vec![(target, payment)];
    form.extend(amount.as_deref().map(|amount| ("amount", amount)));
    form.extend(wanted.reason.as_deref().map(|reason| ("reason", reason)));
    let digest = Sha256::digest(format!("{form:?}"));
    let key: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    let request = request(cx, Method::POST, "/refunds")?
        .header("idempotency-key", format!("cubebot-refund-{key}"))
        .form(&form);
    let made = cx.json(request).await?;
    Ok(json!({
        "id": made["id"],
        "status": made["status"],
        "amount": made["amount"],
        "currency": made["currency"],
        "payment": payment,
    }))
}

fn customer(item: &Value) -> Value {
    json!({
        "id": item["id"],
        "name": item["name"],
        "email": item["email"],
        "description": item["description"],
        "created": item["created"],
    })
}

fn payment(item: &Value) -> Value {
    json!({
        "id": item["id"],
        "amount": item["amount"],
        "currency": item["currency"],
        "status": item["status"],
        "customer": item["customer"],
        "description": item["description"],
        "created": item["created"],
    })
}

fn invoice(item: &Value) -> Value {
    json!({
        "id": item["id"],
        "number": item["number"],
        "status": item["status"],
        "customer": item["customer"],
        "email": item["customer_email"],
        "currency": item["currency"],
        "due": item["amount_due"],
        "paid": item["amount_paid"],
        "due_date": item["due_date"],
        "url": item["hosted_invoice_url"],
        "created": item["created"],
    })
}

fn subscription(item: &Value) -> Value {
    let items: Vec<Value> = item["items"]["data"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|line| {
            json!({
                "price": line["price"]["id"],
                "product": line["price"]["product"],
                "amount": line["price"]["unit_amount"],
                "interval": line["price"]["recurring"]["interval"],
                "quantity": line["quantity"],
                "renews": line["current_period_end"],
            })
        })
        .collect();
    json!({
        "id": item["id"],
        "status": item["status"],
        "customer": item["customer"],
        "currency": item["currency"],
        "cancel_at_period_end": item["cancel_at_period_end"],
        "items": items,
        "created": item["created"],
    })
}

/// The cursor is the newest event's id, or empty before there was any; Stripe answers the events
/// just after it, newest first. One that aged out of the 30 days starts over from now.
async fn poll(cx: &Context, cursor: Option<&str>) -> Result<Poll, Error> {
    let Some(cursor) = cursor else {
        return newest(cx).await;
    };
    let mut query = vec![
        ("types[]", SUCCEEDED),
        ("types[]", FAILED),
        ("limit", POLLED),
    ];
    if !cursor.is_empty() {
        query.push(("ending_before", cursor));
    }
    let found = match cx
        .json(request(cx, Method::GET, "/events")?.query(&query))
        .await
    {
        Err(Error::NotFound(_)) => return newest(cx).await,
        other => other?,
    };
    let list = found["data"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let events: Vec<Event> = list.iter().rev().filter_map(event).collect();
    let newest = list
        .first()
        .and_then(|first| first["id"].as_str())
        .unwrap_or(cursor);
    Ok(Poll {
        events,
        cursor: Some(newest.into()),
    })
}

async fn newest(cx: &Context) -> Result<Poll, Error> {
    let query = [("types[]", SUCCEEDED), ("types[]", FAILED), ("limit", "1")];
    let found = cx
        .json(request(cx, Method::GET, "/events")?.query(&query))
        .await?;
    let cursor = found["data"][0]["id"].as_str().unwrap_or_default();
    Ok(Poll {
        events: Vec::new(),
        cursor: Some(cursor.into()),
    })
}

fn event(item: &Value) -> Option<Event> {
    let (kind, verb) = match item["type"].as_str()? {
        SUCCEEDED => ("payment.succeeded", "succeeded"),
        FAILED => ("payment.failed", "failed"),
        _ => return None,
    };
    let object = &item["data"]["object"];
    let mut data = payment(object);
    data["failure"] = object["last_payment_error"]["message"].clone();
    Some(Event {
        kind,
        id: item["id"].as_str()?.into(),
        summary: format!(
            "Payment {} {verb}: {} in the smallest {} unit",
            object["id"].as_str().unwrap_or("?"),
            object["amount"],
            object["currency"].as_str().unwrap_or("currency"),
        ),
        data,
    })
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
            fields: [("token".into(), "rk_test_secret".into())].into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    fn recorded(name: &str) -> Value {
        let path = format!("{}/fixtures/stripe/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn list(items: Value, more: bool) -> Value {
        json!({"object": "list", "url": "/v1/events", "has_more": more, "data": items})
    }

    #[tokio::test]
    async fn a_key_that_cannot_read_the_account_is_named_by_its_mode() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api.stripe.com/v1/account",
                403,
                json!({"error": {"message": "The provided key 'rk_test_***cret' does not have the required permissions for this endpoint.", "type": "invalid_request_error"}}),
            ),
            Exchange::new("GET", "/api.stripe.com/v1/balance", 200, recorded("balance")),
        ])
        .await;
        let account = Stripe.check(&cx(&fixture)).await.unwrap();
        assert_eq!(account.name, "Stripe account (test)");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[1].header("authorization"),
            Some("Bearer rk_test_secret")
        );
        assert_eq!(seen[1].header("stripe-version"), Some(VERSION));
    }

    #[tokio::test]
    async fn customers_are_listed_by_email_or_searched_and_page_onwards() {
        let customer = recorded("customer");
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api.stripe.com/v1/customers?limit=1&email=jennyrosen%40example.com",
                200,
                list(json!([customer]), true),
            ),
            Exchange::new(
                "GET",
                "/api.stripe.com/v1/customers/search?limit=20&query=name%7E%27jenny%27&page=WzE2OF0",
                200,
                json!({"object": "search_result", "has_more": true, "next_page": "WzE2OV0", "data": [customer]}),
            ),
        ])
        .await;
        let listed = Stripe
            .call(
                &cx(&fixture),
                "search_customers",
                json!({"email": "jennyrosen@example.com", "limit": 1}),
            )
            .await
            .unwrap();
        assert_eq!(listed["items"][0]["email"], "jennyrosen@example.com");
        assert_eq!(listed["next"], "cus_NffrFeUfNV2Hib");
        let searched = Stripe
            .call(
                &cx(&fixture),
                "search_customers",
                json!({"query": "name~'jenny'", "next": "WzE2OF0"}),
            )
            .await
            .unwrap();
        assert_eq!(searched["next"], "WzE2OV0");
        fixture.finish().await;
    }

    #[tokio::test]
    async fn payments_invoices_and_subscriptions_are_compact() {
        let intent = recorded("payment_intent");
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api.stripe.com/v1/payment_intents?limit=20&customer=cus_NffrFeUfNV2Hib",
                200,
                list(json!([intent]), false),
            ),
            Exchange::new(
                "GET",
                "/api.stripe.com/v1/payment_intents/pi_3MtwBwLkdIwHu7ix28a3tqPa",
                200,
                intent,
            ),
            Exchange::new(
                "GET",
                "/api.stripe.com/v1/subscriptions?limit=20&status=active",
                200,
                list(json!([recorded("subscription")]), false),
            ),
        ])
        .await;
        let payments = Stripe
            .call(
                &cx(&fixture),
                "list_payments",
                json!({"customer": "cus_NffrFeUfNV2Hib"}),
            )
            .await
            .unwrap();
        assert_eq!(payments["items"][0]["amount"], 2000);
        assert_eq!(payments["next"], Value::Null);
        let read = Stripe
            .call(
                &cx(&fixture),
                "read_payment",
                json!({"payment": "pi_3MtwBwLkdIwHu7ix28a3tqPa"}),
            )
            .await
            .unwrap();
        assert_eq!(read["failure"], "Your card was declined.");
        let subscriptions = Stripe
            .call(
                &cx(&fixture),
                "list_subscriptions",
                json!({"status": "active"}),
            )
            .await
            .unwrap();
        assert_eq!(subscriptions["items"][0]["items"][0]["renews"], 1699999999);
        fixture.finish().await;
    }

    #[tokio::test]
    async fn a_refund_names_its_payment_and_is_idempotent() {
        let refund = json!({"id": "re_1Nispe2eZvKYlo2Cd31jOCgZ", "object": "refund", "amount": 500, "charge": "ch_1NirD82eZvKYlo2CIvbtLWuY", "currency": "usd", "payment_intent": "pi_3MtwBwLkdIwHu7ix28a3tqPa", "reason": "requested_by_customer", "status": "succeeded"});
        let fixture = Fixture::start(vec![
            Exchange::new("POST", "/api.stripe.com/v1/refunds", 200, refund.clone()),
            Exchange::new("POST", "/api.stripe.com/v1/refunds", 200, refund),
        ])
        .await;
        let input = json!({"payment": "pi_3MtwBwLkdIwHu7ix28a3tqPa", "amount": 500, "reason": "requested_by_customer"});
        let made = Stripe
            .call(&cx(&fixture), "create_refund", input.clone())
            .await
            .unwrap();
        assert_eq!(
            (made["status"].as_str(), made["amount"].as_u64()),
            (Some("succeeded"), Some(500))
        );
        Stripe
            .call(&cx(&fixture), "create_refund", input)
            .await
            .unwrap();
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].text(),
            "payment_intent=pi_3MtwBwLkdIwHu7ix28a3tqPa&amount=500&reason=requested_by_customer"
        );
        let key = seen[0].header("idempotency-key").unwrap();
        assert!(key.starts_with("cubebot-refund-"));
        assert_eq!(seen[1].header("idempotency-key"), Some(key));
    }

    #[tokio::test]
    async fn refusals_map_to_errors_and_bad_input_never_reaches_stripe() {
        let path = "/api.stripe.com/v1/charges/ch_1";
        let cases = [
            (
                401,
                json!({"error": {"message": "Invalid API Key provided: rk_test_***cret", "type": "invalid_request_error"}}),
            ),
            (
                403,
                json!({"error": {"message": "The provided key does not have the required permissions for this endpoint.", "type": "invalid_request_error"}}),
            ),
            (
                404,
                json!({"error": {"code": "resource_missing", "message": "No such charge: 'ch_1'", "param": "id", "type": "invalid_request_error"}}),
            ),
            (
                429,
                json!({"error": {"message": "Too many requests", "type": "rate_limit_error"}}),
            ),
            (
                400,
                json!({"error": {"code": "charge_already_refunded", "message": "Charge ch_1 has already been refunded.", "type": "invalid_request_error"}}),
            ),
        ];
        let exchanges = cases
            .iter()
            .map(|(status, body)| {
                Exchange::new("GET", path, *status, body.clone()).header("retry-after", "300")
            })
            .collect();
        let fixture = Fixture::start(exchanges).await;
        let mut got = Vec::new();
        for _ in &cases {
            let answer = Stripe
                .call(&cx(&fixture), "read_payment", json!({"payment": "ch_1"}))
                .await;
            got.push(answer.unwrap_err());
        }
        assert_eq!(got[0], Error::Unauthorized);
        assert!(matches!(got[1], Error::Forbidden(_)));
        assert_eq!(got[2], Error::NotFound("No such charge: 'ch_1'".into()));
        assert!(matches!(got[3], Error::Unavailable(_)));
        assert!(
            matches!(&got[4], Error::Rejected(message) if message.contains("already been refunded"))
        );
        let bad = Stripe
            .call(
                &cx(&fixture),
                "create_refund",
                json!({"payment": "cus_NffrFeUfNV2Hib"}),
            )
            .await;
        assert!(matches!(bad, Err(Error::Invalid(_))));
        let unknown = Stripe
            .call(&cx(&fixture), "balance", json!({"currency": "usd"}))
            .await;
        assert!(matches!(unknown, Err(Error::Invalid(_))));
        assert_eq!(fixture.finish().await.len(), cases.len());
    }

    #[tokio::test]
    async fn poll_starts_at_the_newest_event_then_reports_newer_ones_oldest_first() {
        let types =
            "types%5B%5D=payment_intent.succeeded&types%5B%5D=payment_intent.payment_failed";
        let succeeded = recorded("event");
        let mut failed = succeeded.clone();
        failed["id"] = json!("evt_3");
        failed["type"] = json!(FAILED);
        failed["data"]["object"]["last_payment_error"] =
            json!({"message": "Your card was declined."});
        let mut older = succeeded.clone();
        older["id"] = json!("evt_2");
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                &format!("/api.stripe.com/v1/events?{types}&limit=1"),
                200,
                list(json!([{"id": "evt_1"}]), true),
            ),
            Exchange::new(
                "GET",
                &format!("/api.stripe.com/v1/events?{types}&limit=50&ending_before=evt_1"),
                200,
                list(json!([failed, older]), false),
            ),
            Exchange::new(
                "GET",
                &format!("/api.stripe.com/v1/events?{types}&limit=50&ending_before=evt_3"),
                200,
                list(json!([]), false),
            ),
        ])
        .await;
        let baseline = Stripe.poll(&cx(&fixture), None).await.unwrap();
        assert!(baseline.events.is_empty());
        assert_eq!(baseline.cursor.as_deref(), Some("evt_1"));
        let next = Stripe.poll(&cx(&fixture), Some("evt_1")).await.unwrap();
        let kinds: Vec<(&str, &str)> = next
            .events
            .iter()
            .map(|event| (event.kind, event.id.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [("payment.succeeded", "evt_2"), ("payment.failed", "evt_3")]
        );
        assert_eq!(
            next.events[0].summary,
            "Payment pi_3MtwBwLkdIwHu7ix28a3tqPa succeeded: 2000 in the smallest usd unit"
        );
        assert_eq!(next.events[1].data["failure"], "Your card was declined.");
        assert_eq!(next.cursor.as_deref(), Some("evt_3"));
        let again = Stripe.poll(&cx(&fixture), Some("evt_3")).await.unwrap();
        assert!(again.events.is_empty());
        assert_eq!(again.cursor.as_deref(), Some("evt_3"));
        fixture.finish().await;
    }
}

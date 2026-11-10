//! WHOOP recovery, sleep, strain and workouts through the person's own WHOOP app
//! (https://developer.whoop.com/docs/developing/oauth, https://developer.whoop.com/api).
//! The API only reads; collections page 25 records at a time with an opaque `nextToken`.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Field, FieldKind, Level, Manifest,
    OAuth2, Risk, Scope, Step, ToolSpec,
};
use serde::Deserialize;
use serde_json::{Value, json};

const API: &str = "https://api.prod.whoop.com/developer/v2";
const RANGE: &str = r#"{"type":"object","properties":{"start":{"type":"string","description":"Records from this ISO 8601 time, such as 2026-11-01T00:00:00Z."},"end":{"type":"string","description":"Records before this ISO 8601 time; now when absent."},"limit":{"type":"integer","minimum":1,"maximum":25},"next":{"type":"string","description":"The next token of a previous answer."}},"additionalProperties":false}"#;

pub struct Whoop;

const fn scope(scope: &'static str) -> Scope {
    Scope {
        scope,
        product: None,
    }
}

static MANIFEST: Manifest = Manifest {
    id: "whoop",
    name: "WHOOP",
    description: "Read your WHOOP recovery, sleep, strain and workouts through your own WHOOP app.",
    category: Category::Health,
    auth: &[Auth::OAuth2(OAuth2 {
        authorize: "https://api.prod.whoop.com/oauth/oauth2/auth",
        token: "https://api.prod.whoop.com/oauth/oauth2/token",
        device: None,
        // WHOOP revokes with a bearer DELETE of /v2/user/access, not RFC 7009.
        revoke: None,
        scopes: &[
            // A refresh token; access tokens last an hour.
            scope("offline"),
            scope("read:profile"),
            scope("read:body_measurement"),
            scope("read:cycles"),
            scope("read:recovery"),
            scope("read:sleep"),
            scope("read:workout"),
        ],
        params: &[],
        fields: &[
            Field {
                key: "client_id",
                label: "Client ID",
                kind: FieldKind::Text,
                required: true,
                placeholder: "",
                help: "From your app in the WHOOP Developer Dashboard.",
            },
            Field {
                key: "client_secret",
                label: "Client secret",
                kind: FieldKind::Secret,
                required: true,
                placeholder: "",
                help: "Shown beside the client ID.",
            },
        ],
    })],
    config: &[],
    products: &[],
    tools: &[
        ToolSpec {
            name: "profile",
            description: "Your name and email, with your height, weight and maximum heart rate.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_cycles",
            description: "Physiological cycles (days) with their strain, energy and heart rate, newest first.",
            risk: Risk::Read,
            product: None,
            input: RANGE,
        },
        ToolSpec {
            name: "list_recoveries",
            description: "Daily recovery scores with resting heart rate, HRV, SpO2 and skin temperature, newest first.",
            risk: Risk::Read,
            product: None,
            input: RANGE,
        },
        ToolSpec {
            name: "list_sleeps",
            description: "Sleeps and naps with their stages, performance, efficiency and need, newest first.",
            risk: Risk::Read,
            product: None,
            input: RANGE,
        },
        ToolSpec {
            name: "list_workouts",
            description: "Workouts with their sport, strain, heart rate zones and distance, newest first.",
            risk: Risk::Read,
            product: None,
            input: RANGE,
        },
    ],
    events: &[],
    level: Level::Read,
    guide: &[
        Step {
            title: "Create a WHOOP app",
            body: "Sign in to the WHOOP Developer Dashboard with your WHOOP account and create an app. Tick every read scope below; offline is requested at sign-in and needs no box.",
            link: Some("https://developer-dashboard.whoop.com/"),
            copy: Some(
                "read:profile read:body_measurement read:cycles read:recovery read:sleep read:workout",
            ),
        },
        Step {
            title: "Add the redirect address",
            body: "Under the app's Redirect URLs add {redirect_uri} exactly and save.",
            link: None,
            copy: Some("{redirect_uri}"),
        },
        Step {
            title: "Copy the client and sign in",
            body: "Copy the app's client ID and client secret below, and sign in with the WHOOP account to connect.",
            link: None,
            copy: None,
        },
    ],
    warning: None,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Range {
    start: Option<String>,
    end: Option<String>,
    limit: Option<u8>,
    next: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Nothing {}

impl Connector for Whoop {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let profile = get(cx, "/user/profile/basic", Vec::new()).await?;
            let name = profile["email"].as_str().unwrap_or("WHOOP").to_owned();
            Ok(Account { name })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "profile" => profile(cx, input).await,
                "list_cycles" => records(cx, "/cycle", input).await,
                "list_recoveries" => records(cx, "/recovery", input).await,
                "list_sleeps" => records(cx, "/activity/sleep", input).await,
                "list_workouts" => records(cx, "/activity/workout", input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }
}

async fn get(cx: &Context, path: &str, query: Vec<(&str, String)>) -> Result<Value, Error> {
    let request = cx.get(&format!("{API}{path}"))?.query(&query);
    cx.json(request).await
}

async fn profile(cx: &Context, input: Value) -> Result<Value, Error> {
    crate::input::<Nothing>(input)?;
    let mut profile = get(cx, "/user/profile/basic", Vec::new()).await?;
    let body = get(cx, "/user/measurement/body", Vec::new()).await?;
    for key in ["height_meter", "weight_kilogram", "max_heart_rate"] {
        profile[key] = body[key].clone();
    }
    Ok(profile)
}

async fn records(cx: &Context, path: &str, input: Value) -> Result<Value, Error> {
    let range: Range = crate::input(input)?;
    let mut query = vec![("limit", range.limit.unwrap_or(10).clamp(1, 25).to_string())];
    query.extend(range.start.map(|start| ("start", start)));
    query.extend(range.end.map(|end| ("end", end)));
    query.extend(range.next.map(|next| ("nextToken", next)));
    let found = get(cx, path, query).await?;
    Ok(json!({"records": found["records"], "next": found["next_token"]}))
}

#[cfg(test)]
mod tests {
    //! Answers are trimmed from the examples in WHOOP's API reference.
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    fn cx(fixture: &Fixture) -> Context {
        let credential = Credential::Token {
            token: "whoop-test-token".into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    fn recorded(name: &str) -> Value {
        let path = format!("{}/fixtures/whoop/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn profile_joins_the_body_measurement() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api.prod.whoop.com/developer/v2/user/profile/basic",
                200,
                json!({"user_id": 10129, "email": "jsmith123@whoop.com", "first_name": "John", "last_name": "Smith"}),
            ),
            Exchange::new(
                "GET",
                "/api.prod.whoop.com/developer/v2/user/measurement/body",
                200,
                json!({"height_meter": 1.8288, "weight_kilogram": 90.7185, "max_heart_rate": 200}),
            ),
        ])
        .await;
        let profile = Whoop
            .call(&cx(&fixture), "profile", json!({}))
            .await
            .unwrap();
        assert_eq!(
            profile,
            json!({"user_id": 10129, "email": "jsmith123@whoop.com", "first_name": "John", "last_name": "Smith", "height_meter": 1.8288, "weight_kilogram": 90.7185, "max_heart_rate": 200})
        );
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].header("authorization"),
            Some("Bearer whoop-test-token")
        );
    }

    #[tokio::test]
    async fn collections_filter_by_time_and_page_with_the_next_token() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api.prod.whoop.com/developer/v2/recovery?limit=25&start=2026-11-01T00%3A00%3A00Z&nextToken=MTIzOjEyMzEyMw",
                200,
                recorded("recovery"),
            ),
            Exchange::new(
                "GET",
                "/api.prod.whoop.com/developer/v2/activity/sleep?limit=10",
                200,
                json!({"records": [], "next_token": null}),
            ),
        ])
        .await;
        let cx = cx(&fixture);
        let input = json!({"start": "2026-11-01T00:00:00Z", "limit": 40, "next": "MTIzOjEyMzEyMw"});
        let listed = Whoop.call(&cx, "list_recoveries", input).await.unwrap();
        assert_eq!(listed["next"], "MTIzOjEyMzEyMw2");
        assert_eq!(listed["records"][0]["score"]["recovery_score"], 44);
        let sleeps = Whoop.call(&cx, "list_sleeps", json!({})).await.unwrap();
        assert_eq!(sleeps, json!({"records": [], "next": null}));
        let refused = Whoop
            .call(&cx, "list_workouts", json!({"sport": "running"}))
            .await;
        assert!(matches!(refused, Err(Error::Invalid(_))));
        fixture.finish().await;
    }
}

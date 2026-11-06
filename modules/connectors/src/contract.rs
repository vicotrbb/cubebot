//! Rules every registered connector must keep, so a new one learns them from a failing test.
//! Each also needs `fixtures/<id>/check.json`: a recorded `check` with the account it names.
use crate::{
    Auth, Connector, Context, Credential, Error, FieldKind, Manifest, Risk, ToolSpec, all,
    testing::{Exchange, Fixture, Script},
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;

/// Tools are offered as `<slug>__<name>` within providers' 64 characters, with slugs up to 32.
const LONGEST_TOOL: usize = 64 - 32 - 2;

fn named(text: &str, extra: u8, longest: usize) -> bool {
    (1..=longest).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == extra)
}

fn problems(manifest: &Manifest) -> Vec<String> {
    let mut found = Vec::new();
    if !named(manifest.id, b'-', 24) {
        found.push("an id is lowercase letters, digits and hyphens".into());
    }
    if manifest.name.is_empty() || !(1..=100).contains(&manifest.description.len()) {
        found.push("a name and a description of at most 100 characters".into());
    }
    if manifest.auth.is_empty() || manifest.tools.is_empty() {
        found.push("at least one way to sign in and one tool".into());
    }
    found.extend(
        manifest
            .tools
            .iter()
            .flat_map(|tool| tool_problems(manifest, tool)),
    );
    found.extend(field_problems(manifest));
    found.extend(
        manifest
            .auth
            .iter()
            .flat_map(|auth| auth_problems(manifest, auth)),
    );
    for event in manifest.events {
        if !event.kind.contains('.') || !known(manifest, event.product) {
            found.push(format!(
                "event {} is `noun.verb` with a known product",
                event.kind
            ));
        }
    }
    let mut tools = HashSet::new();
    for tool in manifest.tools {
        if !tools.insert(tool.name) {
            found.push(format!("tool {} is named twice", tool.name));
        }
    }
    found
        .into_iter()
        .map(|problem| format!("{}: {problem}", manifest.id))
        .collect()
}

fn known(manifest: &Manifest, product: Option<&str>) -> bool {
    product.is_none_or(|product| manifest.products.iter().any(|known| known.id == product))
}

fn tool_problems(manifest: &Manifest, tool: &ToolSpec) -> Vec<String> {
    let mut found = Vec::new();
    if !named(tool.name, b'_', LONGEST_TOOL) {
        found.push(format!("tool {} needs a snake_case name", tool.name));
    }
    if tool.description.len() < 20 || !known(manifest, tool.product) {
        found.push(format!(
            "tool {} needs a description and a known product",
            tool.name
        ));
    }
    if let Err(problem) = schema(tool.input) {
        found.push(format!("tool {}: {problem}", tool.name));
    }
    let sends = ["send", "reply", "post", "comment", "forward", "invite"]
        .iter()
        .any(|verb| tool.name.starts_with(verb));
    if sends && tool.risk != Risk::Send {
        found.push(format!(
            "tool {} reaches people, so its risk is Send",
            tool.name
        ));
    }
    found
}

fn field_problems(manifest: &Manifest) -> Vec<String> {
    let fields = manifest
        .config
        .iter()
        .chain(manifest.auth.iter().flat_map(Auth::fields));
    fields
        .filter(|field| {
            let shared_secret = matches!(field.kind, FieldKind::Secret)
                && manifest.config.iter().any(|shared| shared.key == field.key);
            !named(field.key, b'_', 32) || field.label.is_empty() || shared_secret
        })
        .map(|field| {
            format!(
                "field {} needs a key, a label, and secrets only in auth",
                field.key
            )
        })
        .collect()
}

fn auth_problems(manifest: &Manifest, auth: &Auth) -> Vec<String> {
    let mut found = Vec::new();
    let guided = matches!(
        auth,
        Auth::OAuth2(_) | Auth::GithubApp { .. } | Auth::ClientCredentials(_)
    );
    if guided && manifest.guide.is_empty() {
        found.push("a sign-in with the person's own app needs a setup guide".into());
    }
    if let Auth::OAuth2(oauth) = auth {
        if oauth
            .scopes
            .iter()
            .any(|scope| !known(manifest, scope.product))
        {
            found.push("every scope's product is known".into());
        }
        if !oauth.fields.iter().any(|field| field.key == "client_id") {
            found.push("OAuth uses the person's own client, so it asks for a client_id".into());
        }
    }
    if let Auth::ClientCredentials(app) = auth {
        let keys: Vec<&str> = app.fields.iter().map(|field| field.key).collect();
        let secret = app
            .fields
            .iter()
            .any(|field| field.key == "client_secret" && matches!(field.kind, FieldKind::Secret));
        let named = app.params.iter().all(|(_, value)| {
            value
                .strip_prefix('{')
                .and_then(|value| value.strip_suffix('}'))
                .is_none_or(|key| keys.contains(&key))
        });
        if !keys.contains(&"client_id") || !secret || !named {
            found.push(
                "a client-credentials sign-in asks for a client_id, a secret client_secret and every setting its params name"
                    .into(),
            );
        }
    }
    if let Auth::Linked { connectors, .. } = auth
        && connectors.iter().any(|id| crate::find(id).is_none())
    {
        found.push("a linked sign-in names registered connectors".into());
    }
    found
}

/// A tool's input is an object schema whose required properties exist.
fn schema(input: &str) -> Result<(), String> {
    let schema: Value = serde_json::from_str(input).map_err(|error| error.to_string())?;
    let properties = schema["properties"].as_object();
    if schema["type"] != "object" || properties.is_none() {
        return Err("the input schema is an object with properties".into());
    }
    let required = schema["required"].as_array().into_iter().flatten();
    for name in required {
        let name = name.as_str().unwrap_or_default();
        if !properties.is_some_and(|properties| properties.contains_key(name)) {
            return Err(format!("required {name} is not a property"));
        }
    }
    Ok(())
}

#[test]
fn every_manifest_keeps_the_rules() {
    let mut ids = HashSet::new();
    let mut found: Vec<String> = Vec::new();
    for connector in all() {
        let manifest = connector.manifest();
        if !ids.insert(manifest.id) {
            found.push(format!("{}: ids are unique", manifest.id));
        }
        found.extend(problems(manifest));
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn the_rules_catch_a_send_tool_marked_as_a_write() {
    static TOOLS: [crate::ToolSpec; 1] = [crate::ToolSpec {
        name: "send_message",
        description: "Sends a message to someone.",
        risk: Risk::Write,
        product: None,
        input: r#"{"type":"object","properties":{},"required":["to"]}"#,
    }];
    let manifest = Manifest {
        tools: &TOOLS,
        ..*crate::notion::Notion.manifest()
    };
    let found = problems(&manifest).join("\n");
    assert!(found.contains("risk is Send"), "{found}");
    assert!(found.contains("required to"), "{found}");
}

#[test]
fn the_rules_catch_a_client_credentials_app_without_a_secret_or_a_named_setting() {
    static AUTH: [Auth; 1] = [Auth::ClientCredentials(crate::ClientCredentials {
        token: "https://vendor.example/oauth/token",
        grant: "client_credentials",
        params: &[("account_id", "{account_id}")],
        fields: &[crate::Field {
            key: "client_id",
            label: "Client ID",
            kind: FieldKind::Text,
            required: true,
            placeholder: "",
            help: "",
        }],
    })];
    let manifest = Manifest {
        auth: &AUTH,
        guide: &[],
        ..*crate::notion::Notion.manifest()
    };
    let found = problems(&manifest).join("\n");
    assert!(found.contains("secret client_secret"), "{found}");
    assert!(found.contains("needs a setup guide"), "{found}");
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recorded {
    config: Value,
    credential: Credential,
    account: String,
    #[serde(default)]
    exchanges: Vec<Exchange>,
    #[serde(default)]
    script: Vec<String>,
}

fn recorded(id: &str) -> Recorded {
    let path = format!("{}/fixtures/{id}/check.json", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{path} is missing"));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// `{origin}` and `{port}` in settings stand for the fixture server.
fn placed(config: &Value, origin: &str, port: u16) -> Value {
    let text = config
        .to_string()
        .replace("{origin}", origin)
        .replace("\"{port}\"", &port.to_string());
    serde_json::from_str(&text).unwrap()
}

async fn check(connector: &dyn Connector) -> Result<String, Error> {
    let recorded = recorded(connector.manifest().id);
    let fixture = Fixture::start(recorded.exchanges).await;
    let script = Script::start(recorded.script.clone()).await;
    let config = placed(&recorded.config, &fixture.origin(), script.port());
    let cx = Context::new(config, recorded.credential).with_origin(&fixture.origin());
    let account = connector.check(&cx).await?;
    fixture.finish().await;
    if !recorded.script.is_empty() {
        script.finish().await;
    }
    assert_eq!(
        account.name,
        recorded.account,
        "{}",
        connector.manifest().id
    );
    Ok(account.name)
}

#[tokio::test]
async fn every_connector_checks_against_its_recorded_fixture() {
    for connector in all() {
        let id = connector.manifest().id;
        let account = check(*connector).await;
        assert!(account.is_ok(), "{id}: {account:?}");
    }
}

#[tokio::test]
async fn every_connector_refuses_an_unknown_tool_without_reaching_its_vendor() {
    for connector in all() {
        let recorded = recorded(connector.manifest().id);
        let cx =
            Context::new(recorded.config, recorded.credential).with_origin("http://127.0.0.1:9/");
        let refused = connector.call(&cx, "no_such_tool", Value::Null).await;
        assert_eq!(
            refused,
            Err(Error::UnknownTool),
            "{}",
            connector.manifest().id
        );
    }
}

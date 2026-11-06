//! What a person typed for a connection, checked against its manifest before anything is saved.
use crate::{Auth, FieldKind, Manifest};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// A connection's slug: it prefixes tools as `<slug>__<tool>`, so it holds no underscores.
pub fn slug_valid(slug: &str) -> bool {
    let bytes = slug.as_bytes();
    (1..=32).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// The first problem with `config` and `secrets` for signing in with `auth`, worded for the person.
/// Settings are the manifest's shared fields plus the method's own; secrets only its secret fields.
pub fn problem(
    manifest: &Manifest,
    auth: &Auth,
    config: &Map<String, Value>,
    secrets: &BTreeMap<String, String>,
) -> Option<String> {
    let fields: Vec<_> = manifest.config.iter().chain(auth.fields()).collect();
    for (key, value) in config {
        if key == "products" {
            if !products_valid(manifest, value) {
                return Some("choose products this connector offers".into());
            }
            continue;
        }
        let Some(field) = fields.iter().find(|field| field.key == key) else {
            return Some(format!("{key} is not a setting of {}", manifest.name));
        };
        if matches!(field.kind, FieldKind::Secret) || !value_valid(&field.kind, value) {
            return Some(format!("{} is not valid", field.label));
        }
    }
    for (key, value) in secrets {
        let secret = fields
            .iter()
            .any(|field| field.key == key && matches!(field.kind, FieldKind::Secret));
        if !secret || value.is_empty() || value.len() > 16 * 1024 || value.contains('\0') {
            return Some(format!("{key} is not a secret of {}", manifest.name));
        }
    }
    let missing = fields.iter().find(|field| {
        let given = match field.kind {
            FieldKind::Secret => secrets.contains_key(field.key),
            _ => config.get(field.key).is_some_and(|value| !value.is_null()),
        };
        field.required && !given
    });
    missing.map(|field| format!("{} is required", field.label))
}

fn products_valid(manifest: &Manifest, value: &Value) -> bool {
    value.as_array().is_some_and(|chosen| {
        !chosen.is_empty()
            && chosen
                .iter()
                .all(|item| manifest.products.iter().any(|product| item == product.id))
    })
}

fn text_valid(text: &str) -> bool {
    text.len() <= 2048 && !text.chars().any(char::is_control)
}

fn value_valid(kind: &FieldKind, value: &Value) -> bool {
    match (kind, value) {
        (_, Value::Null) => true,
        (FieldKind::Text, Value::String(text)) => text_valid(text),
        (FieldKind::Url, Value::String(url)) => {
            text_valid(url)
                && reqwest::Url::parse(url).is_ok_and(|url| {
                    matches!(url.scheme(), "https" | "http") && url.host().is_some()
                })
        }
        (FieldKind::Number, Value::Number(number)) => number.as_u64().is_some_and(|n| n <= 65535),
        (FieldKind::Choice(choices), Value::String(chosen)) => {
            choices.iter().any(|choice| choice.value == chosen)
        }
        (FieldKind::Lines, Value::Array(lines)) => {
            lines.len() <= 64
                && lines.iter().all(|line| {
                    line.as_str()
                        .is_some_and(|line| text_valid(line) && !line.is_empty())
                })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn slugs_are_short_lowercase_and_never_split_tool_names() {
        assert!(slug_valid("gmail-work") && slug_valid("n0"));
        for bad in ["", "-a", "Gmail", "a__b", "a_b", &"a".repeat(33)] {
            assert!(!slug_valid(bad), "{bad}");
        }
    }

    #[test]
    fn settings_and_secrets_follow_the_manifest() {
        let notion = crate::notion::Notion;
        let manifest = crate::Connector::manifest(&notion);
        let auth = &manifest.auth[0];
        let secret = |value: &str| BTreeMap::from([("token".to_owned(), value.to_owned())]);
        assert_eq!(problem(manifest, auth, &Map::new(), &secret("ntn_x")), None);
        assert_eq!(
            problem(manifest, auth, &Map::new(), &BTreeMap::new()).as_deref(),
            Some("Internal integration secret is required")
        );
        let unknown = map(json!({"host": "x"}));
        assert!(problem(manifest, auth, &unknown, &secret("t")).is_some());
        let smuggled = map(json!({"token": "in the clear"}));
        assert!(problem(manifest, auth, &smuggled, &secret("t")).is_some());
        let products = map(json!({"products": ["nope"]}));
        assert!(problem(manifest, auth, &products, &secret("t")).is_some());
    }

    #[test]
    fn values_match_their_field_kind() {
        assert!(value_valid(
            &FieldKind::Url,
            &json!("https://dav.example.com/")
        ));
        assert!(!value_valid(&FieldKind::Url, &json!("javascript:alert(1)")));
        assert!(!value_valid(&FieldKind::Text, &json!("two\nlines")));
        assert!(value_valid(&FieldKind::Lines, &json!(["/v1/", "/v2/"])));
        assert!(!value_valid(&FieldKind::Lines, &json!([""])));
        assert!(value_valid(&FieldKind::Number, &json!(993)));
        assert!(!value_valid(&FieldKind::Number, &json!(70000)));
    }
}

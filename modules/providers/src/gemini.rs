//! The native API is only for listing, because only it reports each model's context window.
use crate::{
    Capabilities, Error, Event, Limits, Model, Provider, Request, Tool,
    completions::{self, Completions},
};
use futures_util::Stream;
use reqwest::{Client, Url};
use serde_json::{Map, Value, json};

/// The part of JSON Schema Gemini's function declarations accept; anything else is refused.
const SCHEMA_KEYS: [&str; 14] = [
    "type",
    "format",
    "title",
    "description",
    "nullable",
    "enum",
    "items",
    "properties",
    "required",
    "anyOf",
    "minimum",
    "maximum",
    "minItems",
    "maxItems",
];

pub struct Gemini {
    http: Client,
    base: Url,
    pricing: Url,
}

impl Gemini {
    pub fn new() -> Result<Self, Error> {
        let parse = |url| Url::parse(url).map_err(|error| Error::Configuration(Some(error.into())));
        Self::base(
            parse("https://generativelanguage.googleapis.com/v1beta/")?,
            parse("https://aistudio.google.com/docs/pricing.md")?,
        )
    }

    pub fn for_loopback_tests(origin: &str) -> Result<Self, Error> {
        let base = crate::loopback(origin)?;
        let pricing = base
            .join("pricing.md")
            .map_err(|error| Error::Configuration(Some(error.into())))?;
        Self::base(base, pricing)
    }

    fn base(base: Url, pricing: Url) -> Result<Self, Error> {
        Ok(Self {
            http: crate::client()?,
            base,
            pricing,
        })
    }

    fn url(&self, path: &str) -> Result<Url, Error> {
        self.base
            .join(path)
            .map_err(|error| Error::Configuration(Some(error.into())))
    }

    #[tracing::instrument(name = "provider_request", skip_all, fields(provider = "gemini"))]
    pub async fn prices(&self) -> Result<Vec<crate::Price>, Error> {
        let page = crate::prices::page(&self.http, self.pricing.clone()).await?;
        let prices = from_page(&page);
        if prices.is_empty() {
            return Err(Error::InvalidResponse(None));
        }
        Ok(prices)
    }
}

impl Provider for Gemini {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tool_calls: true,
            model_limits: false,
        }
    }

    async fn limits(&self, _model: &str, _key: &str) -> Result<Limits, Error> {
        Ok(Limits::default())
    }

    /// shortcut: reads one page of 1000, more than Gemini offers; follow `nextPageToken` if not.
    #[tracing::instrument(name = "provider_request", skip_all, fields(provider = "gemini"))]
    async fn models(&self, key: &str) -> Result<Vec<Model>, Error> {
        let request = self
            .http
            .get(self.url("models?pageSize=1000")?)
            .header("x-goog-api-key", key);
        let body = crate::json(crate::send(request).await?).await?;
        let mut models: Vec<Model> = body["models"]
            .as_array()
            .ok_or(Error::InvalidResponse(None))?
            .iter()
            .filter_map(listed)
            .collect();
        // The listing has no dates; highest versions read first, which is nearly newest.
        models.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(models)
    }

    fn stream<'a>(
        &'a self,
        request: &'a Request<'a>,
    ) -> impl Stream<Item = Result<Event, Error>> + Send + 'a {
        crate::events::<Completions>("gemini", request.model, async move {
            let mut body = completions::body(request, function);
            body["stream_options"] = json!({"include_usage":true});
            let builder = self.http.post(self.url("openai/chat/completions")?);
            crate::send(builder.bearer_auth(request.key).json(&body)).await
        })
    }
}

fn listed(entry: &Value) -> Option<Model> {
    let generates = entry["supportedGenerationMethods"]
        .as_array()?
        .iter()
        .any(|method| method == "generateContent");
    let id = entry["name"].as_str()?.trim_start_matches("models/");
    let media = ["tts", "image", "embedding", "aqa", "transcribe", "banana"]
        .iter()
        .any(|word| id.contains(word));
    (generates && !media).then(|| Model {
        id: id.to_owned(),
        context_window: entry["inputTokenLimit"].as_u64(),
        tools: true,
    })
}

fn function(tool: &Tool) -> Value {
    let mut function = completions::function(tool);
    let declared = &mut function["function"];
    let schema = schema(&declared["parameters"], 0);
    // An object without properties is refused; a tool without arguments declares none.
    if free_form(&schema) {
        declared
            .as_object_mut()
            .map(|declared| declared.remove("parameters"));
    } else {
        declared["parameters"] = schema;
    }
    function
}

/// `["string","null"]` is `nullable`, and unknown keys, such as `additionalProperties` in every strict tool, go.
fn schema(value: &Value, depth: u32) -> Value {
    let Some(object) = value.as_object() else {
        return value.clone();
    };
    let mut kept = Map::new();
    for (key, part) in object
        .iter()
        .filter(|(key, _)| SCHEMA_KEYS.contains(&key.as_str()))
    {
        let part = match (key.as_str(), part) {
            ("type", Value::Array(kinds)) => {
                if kinds.iter().any(|kind| kind == "null") {
                    kept.insert("nullable".into(), json!(true));
                }
                kinds
                    .iter()
                    .find(|kind| *kind != "null")
                    .cloned()
                    .unwrap_or(Value::Null)
            }
            ("properties", Value::Object(properties)) => Value::Object(
                properties
                    .iter()
                    .map(|(name, inner)| (name.clone(), schema(inner, depth + 1)))
                    .collect(),
            ),
            ("anyOf", Value::Array(options)) => options
                .iter()
                .map(|inner| schema(inner, depth + 1))
                .collect(),
            ("items", inner) => schema(inner, depth + 1),
            _ => part.clone(),
        };
        kept.insert(key.clone(), part);
    }
    // shortcut: a nested free-form object or an array of anything travels as a string, which a tool may refuse; Gemini would otherwise refuse the whole request.
    if free_form(&Value::Object(kept.clone())) && depth > 0 {
        return json!({"type":"string","description":"A JSON object."});
    }
    if kept.get("type").is_some_and(|kind| kind == "array") && !kept.contains_key("items") {
        kept.insert("items".into(), json!({"type":"string"}));
    }
    Value::Object(kept)
}

fn free_form(schema: &Value) -> bool {
    schema["type"] == "object" && schema["properties"].as_object().is_none_or(Map::is_empty)
}

/// shortcut: a cell's first price is taken, so a promotion or the lowest context tier reads as the price; store the tiers if that skews estimates.
fn from_page(page: &str) -> Vec<crate::Price> {
    page.split("\n## ").skip(1).filter_map(section).collect()
}

fn section(text: &str) -> Option<crate::Price> {
    let model = text.split_once("{% id=\"")?.1.split_once('"')?.0.to_owned();
    let standard = text.split_once("{% tab title=\"Standard\" %}")?.1;
    let table = standard.split_once("{% /table %}")?.0;
    let rate = |label: &str| {
        let row = table
            .split("\n  ---")
            .find(|row| row.trim_start().starts_with(&format!("* {label}")))?;
        // The paid tier is the row's last cell; a free tier's own price never comes first.
        let paid = row.rsplit("\n  * ").next()?;
        let dollars = paid.split_once('$')?.1;
        let end = dollars
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(dollars.len());
        Some(dollars[..end].parse::<f64>().ok()? / 1_000_000.0)
    };
    let input = rate("Input price")?;
    Some(crate::Price {
        model,
        input_per_token: input,
        cached_per_token: rate("Context caching price").unwrap_or(input),
        output_per_token: rate("Output price")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_schemas_narrow_to_what_gemini_accepts() {
        let tool = Tool::strict(
            "t",
            "d",
            json!({"name":{"type":["string","null"],"description":"n"},"inner":{"type":"object","properties":{"a":{"type":"string","pattern":"x"}},"required":["a"],"additionalProperties":false}}),
        );
        let function = function(&tool);
        let parameters = &function["function"]["parameters"];
        assert!(parameters.get("additionalProperties").is_none());
        assert_eq!(parameters["properties"]["name"]["type"], "string");
        assert_eq!(parameters["properties"]["name"]["nullable"], true);
        let inner = &parameters["properties"]["inner"];
        assert!(inner.get("additionalProperties").is_none());
        assert!(inner["properties"]["a"].get("pattern").is_none());
    }

    #[test]
    fn a_tool_without_arguments_declares_no_parameters() {
        let function = function(&Tool::strict("t", "d", json!({})));
        assert!(function["function"].get("parameters").is_none());
        let bare = Tool {
            parameters: json!({"type":"object"}),
            ..Tool::strict("t", "d", json!({}))
        };
        assert!(
            super::function(&bare)["function"]
                .get("parameters")
                .is_none()
        );
    }

    #[test]
    fn what_cannot_be_declared_becomes_something_that_can() {
        let declared = schema(
            &json!({"type":"object","properties":{"free":{"type":"object"},"list":{"type":"array"}}}),
            0,
        );
        assert_eq!(declared["properties"]["free"]["type"], "string");
        assert_eq!(declared["properties"]["list"]["items"]["type"], "string");
    }

    #[test]
    fn the_listing_keeps_only_models_that_converse() {
        let entry = |name: &str, method: &str| json!({"name":name,"inputTokenLimit":1000,"supportedGenerationMethods":[method]});
        assert_eq!(
            listed(&entry("models/gemini-3.8-flash", "generateContent")),
            Some(Model {
                id: "gemini-3.8-flash".into(),
                context_window: Some(1000),
                tools: true
            })
        );
        assert_eq!(listed(&entry("models/embedding-1", "embedContent")), None);
        assert_eq!(
            listed(&entry("models/gemini-3.8-flash-tts", "generateContent")),
            None
        );
    }

    #[test]
    fn prices_read_from_the_paid_tier_of_the_standard_table() {
        let page = "# Pricing\n\n## Gemini X  {% id=\"gemini-x\" %}\n\n{% tabs %}\n  {% tab title=\"Standard\" %}\n  {% table %}\n  * Feature\n  * Free Tier\n  * Paid Tier, per 1M tokens in USD\n\n  ---\n\n  * Input price\n  * Free of charge\n  * $1.25, prompts <= 200k tokens    \n  $2.50, prompts > 200k tokens\n\n  ---\n\n  * Output price (including thinking tokens)\n  * Free of charge\n  * $10.00\n\n  ---\n\n  * Context caching price\n  * Not available\n  * $0.125, prompts <= 200k tokens\n  {% /table %}\n  {% /tab %}\n  {% tab title=\"Batch\" %}\n  {% table %}\n  * Input price\n  * $9\n  {% /table %}\n{% /tabs %}\n\n## Text only\n\nNothing here.\n";
        let prices = from_page(page);
        assert_eq!(prices.len(), 1);
        assert_eq!(prices[0].model, "gemini-x");
        assert_eq!(prices[0].input_per_token, 1.25e-6);
        assert_eq!(prices[0].output_per_token, 10e-6);
        assert_eq!(prices[0].cached_per_token, 0.125e-6);
    }

    #[tokio::test]
    #[ignore = "reads the live pricing page"]
    async fn live_page() {
        let prices = Gemini::new().unwrap().prices().await.unwrap();
        assert!(
            prices
                .iter()
                .any(|price| price.model.starts_with("gemini-"))
        );
    }
}

//! xAI Grok over Chat Completions.
//! shortcut: xAI labels this API legacy in favor of Responses, with no shutoff date; move to it
//! when one is announced, sharing the OpenAI adapter's decoder.
use crate::{
    Capabilities, Error, Event, Limits, Model, Provider, Request,
    completions::{self, Completions},
};
use futures_util::Stream;
use reqwest::{Client, Url};
use serde_json::{Value, json};

pub struct Grok {
    http: Client,
    /// Ends in `/`; `chat/completions` and `models` resolve against it.
    base: Url,
}

impl Grok {
    pub fn new() -> Result<Self, Error> {
        Self::base(
            Url::parse("https://api.x.ai/v1/")
                .map_err(|error| Error::Configuration(Some(error.into())))?,
        )
    }

    pub fn for_loopback_tests(origin: &str) -> Result<Self, Error> {
        Self::base(crate::loopback(origin)?)
    }

    fn base(base: Url) -> Result<Self, Error> {
        Ok(Self {
            http: crate::client()?,
            base,
        })
    }

    fn url(&self, path: &str) -> Result<Url, Error> {
        self.base
            .join(path)
            .map_err(|error| Error::Configuration(Some(error.into())))
    }
}

impl Provider for Grok {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tool_calls: true,
            // The window is configured from the listing.
            model_limits: false,
        }
    }

    async fn limits(&self, _model: &str, _key: &str) -> Result<Limits, Error> {
        Ok(Limits::default())
    }

    #[tracing::instrument(name = "provider_request", skip_all, fields(provider = "grok"))]
    async fn models(&self, key: &str) -> Result<Vec<Model>, Error> {
        let request = self.http.get(self.url("models")?).bearer_auth(key);
        let mut body = crate::json(crate::send(request).await?).await?;
        if let Some(data) = body["data"].as_array_mut() {
            data.retain(conversational);
            data.sort_by_key(|entry| std::cmp::Reverse(entry["created"].as_u64()));
        }
        crate::models(&body, |entry| entry["context_length"].as_u64())
    }

    fn stream<'a>(
        &'a self,
        request: &'a Request<'a>,
    ) -> impl Stream<Item = Result<Event, Error>> + Send + 'a {
        crate::events::<Completions>("grok", request.model, async move {
            let mut body = completions::body(request, completions::function);
            body["stream_options"] = json!({"include_usage":true});
            let builder = self.http.post(self.url("chat/completions")?);
            crate::send(builder.bearer_auth(request.key).json(&body)).await
        })
    }
}

/// The listing also holds image and video models, which bill no completion tokens. A missing price
/// keeps a model, so a change to the listing never empties it.
fn conversational(entry: &Value) -> bool {
    let id = entry["id"].as_str().unwrap_or_default();
    entry["completion_text_token_price"].as_u64() != Some(0)
        && !["imagine", "image", "video"]
            .iter()
            .any(|word| id.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_models_leave_the_listing() {
        assert!(conversational(
            &json!({"id":"grok-4.7","completion_text_token_price":60000})
        ));
        assert!(!conversational(
            &json!({"id":"grok-imagine","completion_text_token_price":0})
        ));
        assert!(!conversational(&json!({"id":"grok-imagine"})));
        assert!(conversational(&json!({"id":"grok-9"})));
    }
}

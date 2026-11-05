//! OpenRouter's docs require replaying `reasoning_details`, which the shared format does.
use crate::{
    Capabilities, Error, Event, Limits, Model, Provider, Request,
    completions::{self, Completions},
};
use futures_util::Stream;
use reqwest::{Client, RequestBuilder, Url};
use serde_json::{Value, json};

pub struct OpenRouter {
    http: Client,
    base: Url,
}

impl OpenRouter {
    pub fn new() -> Result<Self, Error> {
        Self::base(
            Url::parse("https://openrouter.ai/api/v1/")
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

    #[tracing::instrument(name = "provider_request", skip_all, fields(provider = "openrouter"))]
    pub async fn prices(&self) -> Result<Vec<crate::Price>, Error> {
        let response = crate::send(self.http.get(self.url("models")?)).await?;
        let body = crate::json(response).await?;
        let entries = body["data"]
            .as_array()
            .ok_or(Error::InvalidResponse(None))?;
        Ok(entries.iter().filter_map(price).collect())
    }

    fn authorized(&self, request: RequestBuilder, key: &str) -> RequestBuilder {
        request.bearer_auth(key).header("x-title", "Cubebot")
    }
}

impl Provider for OpenRouter {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tool_calls: true,
            // The window is configured from the listing; fetching all of it per turn is wasteful.
            model_limits: false,
        }
    }

    async fn limits(&self, _model: &str, _key: &str) -> Result<Limits, Error> {
        Ok(Limits::default())
    }

    #[tracing::instrument(name = "provider_request", skip_all, fields(provider = "openrouter"))]
    async fn models(&self, key: &str) -> Result<Vec<Model>, Error> {
        // The listing is public, so only this says whether the key works.
        crate::send(self.authorized(self.http.get(self.url("key")?), key)).await?;
        let response =
            crate::send(self.authorized(self.http.get(self.url("models")?), key)).await?;
        let mut body = crate::json(response).await?;
        let entries = body["data"]
            .as_array_mut()
            .ok_or(Error::InvalidResponse(None))?;
        entries.sort_by_key(|entry| std::cmp::Reverse(entry["created"].as_u64()));
        entries
            .iter()
            .map(|entry| {
                Ok(Model {
                    id: entry["id"]
                        .as_str()
                        .ok_or(Error::InvalidResponse(None))?
                        .to_owned(),
                    context_window: entry["context_length"].as_u64(),
                    tools: entry["supported_parameters"]
                        .as_array()
                        .is_some_and(|parameters| parameters.iter().any(|name| name == "tools")),
                })
            })
            .collect()
    }

    fn stream<'a>(
        &'a self,
        request: &'a Request<'a>,
    ) -> impl Stream<Item = Result<Event, Error>> + Send + 'a {
        crate::events::<Completions>("openrouter", request.model, async move {
            let builder = self.http.post(self.url("chat/completions")?);
            let mut body = completions::body(request, completions::function);
            body["usage"] = json!({"include":true});
            crate::send(self.authorized(builder, request.key).json(&body)).await
        })
    }
}

fn price(entry: &Value) -> Option<crate::Price> {
    let rate = |name: &str| entry["pricing"][name].as_str()?.parse::<f64>().ok();
    let (input, output) = (rate("prompt")?, rate("completion")?);
    // A negative rate is OpenRouter's marker for a router with no fixed price.
    (input >= 0.0 && output >= 0.0).then_some(crate::Price {
        model: entry["id"].as_str()?.to_owned(),
        input_per_token: input,
        cached_per_token: rate("input_cache_read").unwrap_or(input),
        output_per_token: output,
    })
}

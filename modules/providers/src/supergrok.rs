//! SuperGrok and X Premium plans through the Grok CLI's proxy, which serves the Responses API.
//! Opt-in: it impersonates the Grok CLI, whose version gate refuses every other client
//! (docs/adr/identity.md#personal-subscription-connections). Removing it removes this file.
use crate::{Error, Model, OpenAi, openai::Dialect};
use reqwest::Url;
use serde_json::Value;

/// The proxy answers 426 below this Grok CLI version.
const CLIENT_VERSION: &str = "1.0.13";

const SUPERGROK: Dialect = Dialect {
    instructions: false,
    native_computer: false,
    header: Some(("x-grok-client-version", CLIENT_VERSION)),
    catalog,
};

impl OpenAi {
    pub fn supergrok() -> Result<Self, Error> {
        let parse = |url| Url::parse(url).map_err(|error| Error::Configuration(Some(error.into())));
        let endpoint = parse("https://cli-chat-proxy.grok.com/v1/responses")?;
        Ok(Self::endpoint(endpoint.clone(), endpoint)?.with(SUPERGROK))
    }

    pub fn supergrok_for_loopback_tests(origin: &str) -> Result<Self, Error> {
        Ok(Self::for_loopback_tests(origin)?.with(SUPERGROK))
    }
}

fn catalog(body: Value) -> Result<Vec<Model>, Error> {
    crate::models(&body, |entry| entry["context_window"].as_u64())
}

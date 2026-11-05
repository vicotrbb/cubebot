use crate::{
    Capabilities, Decoder, Error, Event, Limits, Message, Model, Provider, Request, Usage, call,
    reply,
};
use futures_util::Stream;
use reqwest::{Client, Url};
use serde_json::{Value, json};
use std::collections::VecDeque;

const NOT_CONVERSATIONAL: [&str; 12] = [
    "embedding",
    "tts",
    "whisper",
    "transcribe",
    "dall-e",
    "image",
    "moderation",
    "audio",
    "realtime",
    "sora",
    "davinci",
    "babbage",
];

/// The standard rates: batch, flex and fast tiers sit under their own headings.
const PRICES: crate::prices::Columns = crate::prices::Columns {
    input: &["Short context input", "Input"],
    cached: &["Short context cached input", "Cached input"],
    output: &["Short context output", "Output"],
};

pub struct OpenAi {
    http: Client,
    endpoint: Url,
    pricing: Url,
    dialect: Dialect,
}

/// How a Responses endpoint reached with a plan's token differs from the API-key one.
#[derive(Clone, Copy)]
pub(crate) struct Dialect {
    /// The system prompt goes in `instructions`, and `max_output_tokens` is refused.
    pub instructions: bool,
    /// Hosted computer use is unsupported, so the computer tool is an ordinary function.
    pub native_computer: bool,
    pub header: Option<(&'static str, &'static str)>,
    pub catalog: fn(Value) -> Result<Vec<Model>, Error>,
}

const API: Dialect = Dialect {
    instructions: false,
    native_computer: true,
    header: None,
    catalog: api_catalog,
};

/// Sign in with ChatGPT's plan usage: https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations
const CHATGPT: Dialect = Dialect {
    instructions: true,
    native_computer: false,
    header: None,
    catalog: chatgpt_catalog,
};

impl OpenAi {
    pub fn new() -> Result<Self, Error> {
        let parse = |url| Url::parse(url).map_err(|error| Error::Configuration(Some(error.into())));
        Self::endpoint(
            parse("https://api.openai.com/v1/responses")?,
            parse("https://developers.openai.com/api/docs/pricing.md")?,
        )
    }

    pub fn for_loopback_tests(origin: &str) -> Result<Self, Error> {
        let join = |path| {
            crate::loopback(origin)?
                .join(path)
                .map_err(|error| Error::Configuration(Some(error.into())))
        };
        Self::endpoint(join("responses")?, join("pricing.md")?)
    }

    /// The same Responses API with a ChatGPT plan's access token; never ChatGPT's `backend-api`.
    pub fn chatgpt() -> Result<Self, Error> {
        Ok(Self::new()?.with(CHATGPT))
    }

    pub fn chatgpt_for_loopback_tests(origin: &str) -> Result<Self, Error> {
        Ok(Self::for_loopback_tests(origin)?.with(CHATGPT))
    }

    pub(crate) fn endpoint(endpoint: Url, pricing: Url) -> Result<Self, Error> {
        Ok(Self {
            http: crate::client()?,
            endpoint,
            pricing,
            dialect: API,
        })
    }

    pub(crate) fn with(self, dialect: Dialect) -> Self {
        Self { dialect, ..self }
    }

    #[tracing::instrument(name = "provider_request", skip_all, fields(provider = "openai"))]
    pub async fn prices(&self) -> Result<Vec<crate::Price>, Error> {
        let page = crate::prices::page(&self.http, self.pricing.clone()).await?;
        crate::prices::from_markdown(&page, "### Standard pricing data", &PRICES, str::to_owned)
    }
}

impl Provider for OpenAi {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tool_calls: true,
            model_limits: false,
        }
    }

    async fn limits(&self, _model: &str, _key: &str) -> Result<Limits, Error> {
        // The Models API reports no context window.
        Ok(Limits::default())
    }

    #[tracing::instrument(name = "provider_request", skip_all, fields(provider = "openai"))]
    async fn models(&self, key: &str) -> Result<Vec<Model>, Error> {
        let url = self
            .endpoint
            .join("models")
            .map_err(|error| Error::Configuration(Some(error.into())))?;
        let response = crate::send(self.headed(self.http.get(url)).bearer_auth(key)).await?;
        (self.dialect.catalog)(crate::json(response).await?)
    }

    fn stream<'a>(
        &'a self,
        request: &'a Request<'a>,
    ) -> impl Stream<Item = Result<Event, Error>> + Send + 'a {
        crate::events::<Responses>(
            "openai",
            request.model,
            crate::send(
                self.headed(self.http.post(self.endpoint.clone()))
                    .bearer_auth(request.key)
                    .json(&body(request, self.dialect)),
            ),
        )
    }
}

impl OpenAi {
    fn headed(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.dialect.header {
            Some((name, value)) => builder.header(name, value),
            None => builder,
        }
    }
}

fn api_catalog(mut body: Value) -> Result<Vec<Model>, Error> {
    if let Some(data) = body["data"].as_array_mut() {
        data.retain(|entry| {
            entry["id"]
                .as_str()
                .is_some_and(|id| !NOT_CONVERSATIONAL.iter().any(|word| id.contains(word)))
        });
        data.sort_by_key(|entry| std::cmp::Reverse(entry["created"].as_u64()));
    }
    crate::models(&body, |_| None)
}

/// The account's own catalog, in the server's order: listed models by `slug`.
fn chatgpt_catalog(body: Value) -> Result<Vec<Model>, Error> {
    let models = body["models"]
        .as_array()
        .ok_or(Error::InvalidResponse(None))?;
    models
        .iter()
        .filter(|entry| entry["visibility"] == "list" && entry["supported_in_api"] != false)
        .map(|entry| {
            Ok(Model {
                id: entry["slug"]
                    .as_str()
                    .ok_or(Error::InvalidResponse(None))?
                    .to_owned(),
                context_window: entry["context_window"].as_u64(),
                tools: true,
            })
        })
        .collect()
}

fn body(request: &Request, dialect: Dialect) -> Value {
    let computer = crate::computer_calls(request.messages, |item| {
        (item["type"] == "computer_call").then(|| item["call_id"].as_str())?
    });
    let mut input = system(request, dialect);
    for message in request.messages {
        match message {
            Message::User(text) => input.push(json!({"role":"user","content":text})),
            Message::Assistant(reply) => match &reply.native {
                Value::Array(items) => input.extend(items.iter().cloned()),
                _ => input.push(json!({"role":"assistant","content":reply.text})),
            },
            Message::ToolResult {
                call_id,
                output,
                image,
            } => match image {
                // A computer call's output is only its screenshot, so a refusal, such as while a person holds the desktop, follows as a note.
                Some(image) if computer.contains(&&**call_id) => {
                    input.push(json!({"type":"computer_call_output","call_id":call_id,"output":{"type":"computer_screenshot","image_url":format!("data:image/png;base64,{image}"),"detail":"original"}}));
                    if output != "OK" {
                        let note = format!("The computer tool reported: {output}");
                        input.push(json!({"role":"developer","content":note}));
                    }
                }
                Some(image) => input.push(
                    json!({"type":"function_call_output","call_id":call_id,"output":[
                        {"type":"input_text","text":output},
                        {"type":"input_image","image_url":format!("data:image/png;base64,{image}")},
                    ]}),
                ),
                None => input
                    .push(json!({"type":"function_call_output","call_id":call_id,"output":output})),
            },
        }
    }
    let mut body = json!({
        "model":request.model,"input":input,"store":false,"stream":true,
        "include":["reasoning.encrypted_content"]
    });
    prompted(&mut body, request, dialect);
    if !request.tools.is_empty() {
        body["tools"] = request
            .tools
            .iter()
            .map(|tool| declared(tool, dialect))
            .collect();
        if request.tools_disabled {
            body["tool_choice"] = json!("none");
        }
    }
    body
}

/// A plan takes the system prompt as `instructions`, never as a system message.
fn system(request: &Request, dialect: Dialect) -> Vec<Value> {
    if dialect.instructions {
        return Vec::new();
    }
    vec![json!({"role":"system","content":request.system})]
}

/// A plan refuses `max_output_tokens`.
fn prompted(body: &mut Value, request: &Request, dialect: Dialect) {
    if dialect.instructions {
        body["instructions"] = json!(request.system);
    } else if let Some(limit) = request.max_output_tokens {
        body["max_output_tokens"] = json!(limit);
    }
}

fn declared(tool: &crate::Tool, dialect: Dialect) -> Value {
    if tool.computer && dialect.native_computer {
        json!({"type":"computer"})
    } else if tool.computer {
        let mut function = crate::completions::function(tool)["function"].take();
        function["type"] = json!("function");
        function
    } else {
        json!({"type":"function","name":tool.name,"description":tool.description,"strict":tool.strict,"parameters":tool.parameters})
    }
}

#[derive(Default)]
struct Responses;
impl Decoder for Responses {
    fn decode(&mut self, data: Value, events: &mut VecDeque<Event>) -> Result<bool, Error> {
        match data["type"].as_str() {
            Some("response.output_text.delta") => {
                let delta = data["delta"].as_str().ok_or(Error::InvalidResponse(None))?;
                events.push_back(Event::Text(delta.into()));
                Ok(false)
            }
            Some("response.completed") => {
                events.push_back(completed(&data["response"])?);
                Ok(true)
            }
            Some("response.incomplete") => Err(Error::InvalidResponse(None)),
            Some("response.failed") => Err(failure(&data["response"]["error"]["code"])),
            // Plan usage reports its code under `error`; the API puts it at the top.
            Some("error") => Err(failure(
                data["error"]
                    .get("code")
                    .filter(|code| !code.is_null())
                    .unwrap_or(&data["code"]),
            )),
            _ => Ok(false),
        }
    }
}

/// Only rate limits and server faults are worth retrying; anything else fails the same again.
fn failure(code: &Value) -> Error {
    match code.as_str() {
        Some(
            "server_error"
            | "rate_limit_exceeded"
            | "subscription_sharing_usage_unavailable"
            | "subscription_sharing_user_unavailable",
        ) => Error::Transient(None),
        _ if usage_limit(code) => Error::UsageLimit,
        _ => Error::Rejected,
    }
}

pub(crate) fn usage_limit(code: &Value) -> bool {
    code == "subscription_sharing_usage_limit_exceeded"
}

/// The final response is authoritative; deltas were only progress.
fn completed(response: &Value) -> Result<Event, Error> {
    if response["status"] != "completed" {
        return Err(Error::InvalidResponse(None));
    }
    let output = response["output"]
        .as_array()
        .ok_or(Error::InvalidResponse(None))?;
    let mut text = String::new();
    let mut calls = Vec::new();
    for item in output {
        match item["type"].as_str() {
            Some("reasoning") => (),
            Some("message") => append_content(&mut text, &item["content"])?,
            Some("function_call") => calls.push(function_call(item)?),
            Some("computer_call") => calls.push(computer_call(item)?),
            _ => return Err(Error::InvalidResponse(None)),
        }
    }
    let usage = &response["usage"];
    let usage = usage["input_tokens"]
        .as_u64()
        .zip(usage["output_tokens"].as_u64())
        .map(|(input_tokens, output_tokens)| Usage {
            input_tokens,
            output_tokens,
            cached_input_tokens: usage["input_tokens_details"]["cached_tokens"]
                .as_u64()
                .unwrap_or(0),
            cost_usd: None,
        });
    Ok(Event::Done {
        reply: reply(text, calls, Value::Array(output.clone()))?,
        usage,
    })
}

fn append_content(text: &mut String, value: &Value) -> Result<(), Error> {
    for part in value.as_array().ok_or(Error::InvalidResponse(None))? {
        if part["type"] != "output_text" {
            return Err(Error::InvalidResponse(None));
        }
        text.push_str(part["text"].as_str().ok_or(Error::InvalidResponse(None))?);
    }
    Ok(())
}

fn function_call(item: &Value) -> Result<crate::FunctionCall, Error> {
    let field = |name: &str| item[name].as_str().ok_or(Error::InvalidResponse(None));
    call(
        field("call_id")?,
        field("name")?,
        field("arguments")?.into(),
    )
}

fn computer_call(item: &Value) -> Result<crate::FunctionCall, Error> {
    let call_id = item["call_id"]
        .as_str()
        .ok_or(Error::InvalidResponse(None))?;
    let actions = json!({"actions":item["actions"]});
    call(call_id, "computer", actions.to_string())
}

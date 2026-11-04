//! OpenAI Responses over SSE, stateless (`store: false`) with encrypted reasoning replay.
use crate::{
    Capabilities, Decoder, Error, Event, Limits, Message, Model, Provider, Request, Usage, call,
    reply,
};
use futures_util::Stream;
use reqwest::{Client, Url};
use serde_json::{Value, json};
use std::collections::VecDeque;

/// Model families the Models API lists that cannot hold a conversation.
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
}

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

    fn endpoint(endpoint: Url, pricing: Url) -> Result<Self, Error> {
        Ok(Self {
            http: crate::client()?,
            endpoint,
            pricing,
        })
    }

    /// The standard-tier prices on OpenAI's pricing page.
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
        let response = crate::send(self.http.get(url).bearer_auth(key)).await?;
        let mut body = crate::json(response).await?;
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

    fn stream<'a>(
        &'a self,
        request: &'a Request<'a>,
    ) -> impl Stream<Item = Result<Event, Error>> + Send + 'a {
        crate::events::<Responses>(
            "openai",
            request.model,
            crate::send(
                self.http
                    .post(self.endpoint.clone())
                    .bearer_auth(request.key)
                    .json(&body(request)),
            ),
        )
    }
}

fn body(request: &Request) -> Value {
    let computer = crate::computer_calls(request.messages, |item| {
        (item["type"] == "computer_call").then(|| item["call_id"].as_str())?
    });
    let mut input = vec![json!({"role":"system","content":request.system})];
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
                // A computer call's output is only its screenshot, so a refusal, such as while a
                // person holds the desktop, follows it as a note the model can read.
                Some(image) if computer.contains(&&**call_id) => {
                    input.push(json!({"type":"computer_call_output","call_id":call_id,"output":{"type":"computer_screenshot","image_url":format!("data:image/png;base64,{image}"),"detail":"original"}}));
                    if output != "OK" {
                        let note = format!("The computer tool reported: {output}");
                        input.push(json!({"role":"developer","content":note}));
                    }
                }
                // Another tool's image, such as a browser screenshot, follows its text.
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
    if let Some(limit) = request.max_output_tokens {
        body["max_output_tokens"] = json!(limit);
    }
    if !request.tools.is_empty() {
        body["tools"] = request
            .tools
            .iter()
            .map(|tool| {
                if tool.computer {
                    json!({"type":"computer"})
                } else {
                    json!({"type":"function","name":tool.name,"description":tool.description,"strict":tool.strict,"parameters":tool.parameters})
                }
            })
            .collect();
        if request.tools_disabled {
            body["tool_choice"] = json!("none");
        }
    }
    body
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
            // Truncated output is not a reply.
            Some("response.incomplete") => Err(Error::InvalidResponse(None)),
            Some("response.failed") => Err(failure(&data["response"]["error"]["code"])),
            Some("error") => Err(failure(&data["code"])),
            _ => Ok(false),
        }
    }
}

/// Only rate limits and server faults are worth retrying; anything else fails the same again.
fn failure(code: &Value) -> Error {
    match code.as_str() {
        Some("server_error" | "rate_limit_exceeded") => Error::Transient(None),
        _ => Error::Rejected,
    }
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

/// The call's batch of actions, as one `computer` call.
fn computer_call(item: &Value) -> Result<crate::FunctionCall, Error> {
    let call_id = item["call_id"]
        .as_str()
        .ok_or(Error::InvalidResponse(None))?;
    let actions = json!({"actions":item["actions"]});
    call(call_id, "computer", actions.to_string())
}

//! OpenRouter Chat Completions over SSE, the one API every model it routes to speaks. Assistant
//! messages are replayed whole so `reasoning_details` survive tool calls, as its docs require.
use crate::{
    Capabilities, Decoder, Error, Event, FunctionCall, Limits, Message, Model, Provider, Request,
    Tool, Usage, call, reply,
};
use futures_util::Stream;
use reqwest::{Client, RequestBuilder, Url};
use serde_json::{Value, json};
use std::collections::VecDeque;

pub struct OpenRouter {
    http: Client,
    /// Ends in `/`; `chat/completions`, `models` and `key` resolve against it.
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

    /// What every model on OpenRouter costs per token, from its public listing.
    #[tracing::instrument(name = "provider_request", skip_all, fields(provider = "openrouter"))]
    pub async fn prices(&self) -> Result<Vec<crate::Price>, Error> {
        let response = crate::send(self.http.get(self.url("models")?)).await?;
        let body = crate::json(response).await?;
        let entries = body["data"]
            .as_array()
            .ok_or(Error::InvalidResponse(None))?;
        Ok(entries.iter().filter_map(price).collect())
    }

    /// Names the app on OpenRouter's own usage pages.
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
            crate::send(self.authorized(builder, request.key).json(&body(request))).await
        })
    }
}

fn body(request: &Request) -> Value {
    let mut messages = vec![json!({"role":"system","content":request.system})];
    // A person-role image may only follow the whole run of tool results it belongs to.
    let mut images = Vec::new();
    for message in request.messages {
        if !matches!(message, Message::ToolResult { .. }) {
            flush(&mut messages, &mut images);
        }
        match message {
            Message::User(text) => messages.push(json!({"role":"user","content":text})),
            Message::Assistant(reply) => match &reply.native {
                Value::Object(_) => messages.push(reply.native.clone()),
                _ => messages.push(json!({"role":"assistant","content":reply.text})),
            },
            Message::ToolResult {
                call_id,
                output,
                image,
            } => {
                messages.push(json!({"role":"tool","tool_call_id":call_id,"content":output}));
                images.extend(image.iter());
            }
        }
    }
    flush(&mut messages, &mut images);
    let mut body = json!({
        "model":request.model,"messages":messages,"stream":true,"usage":{"include":true}
    });
    if let Some(limit) = request.max_output_tokens {
        body["max_tokens"] = json!(limit);
    }
    if !request.tools.is_empty() {
        body["tools"] = request.tools.iter().map(function).collect();
        if request.tools_disabled {
            body["tool_choice"] = json!("none");
        }
    }
    body
}

fn flush(messages: &mut Vec<Value>, images: &mut Vec<&String>) {
    if images.is_empty() {
        return;
    }
    let content: Vec<Value> = images
        .drain(..)
        .map(|image| json!({"type":"image_url","image_url":{"url":format!("data:image/png;base64,{image}")}}))
        .collect();
    messages.push(json!({"role":"user","content":content}));
}

/// Models behind OpenRouter have no provider computer tool, so it is an ordinary function taking
/// the same single actions the agent runs for Anthropic.
fn function(tool: &Tool) -> Value {
    if !tool.computer {
        return json!({"type":"function","function":{"name":tool.name,"description":tool.description,"parameters":tool.parameters}});
    }
    json!({"type":"function","function":{
        "name":tool.name,
        "description":"See and control the desktop, 1280 by 800 pixels. Take a screenshot before acting and after to see the result.",
        "parameters":{"type":"object","required":["action"],"properties":{
            "action":{"type":"string","enum":["screenshot","left_click","right_click","middle_click","double_click","triple_click","mouse_move","left_click_drag","scroll","type","key","wait"]},
            "coordinate":{"type":"array","items":{"type":"integer"},"description":"[x, y]"},
            "start_coordinate":{"type":"array","items":{"type":"integer"},"description":"Where a drag starts: [x, y]"},
            "text":{"type":"string","description":"What to type, or the key such as Return or ctrl+a"},
            "scroll_direction":{"type":"string","enum":["up","down","left","right"]},
            "scroll_amount":{"type":"integer"},
            "duration":{"type":"number","description":"Seconds to wait"},
        }},
    }})
}

#[derive(Default)]
struct PendingCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
struct Completions {
    text: String,
    calls: Vec<PendingCall>,
    reasoning: Vec<Value>,
    finish: Option<String>,
    usage: Option<Usage>,
}

impl Decoder for Completions {
    fn decode(&mut self, data: Value, events: &mut VecDeque<Event>) -> Result<bool, Error> {
        if data == crate::sse::DONE {
            events.push_back(self.finished()?);
            return Ok(true);
        }
        if !data["error"].is_null() {
            return Err(failure(&data["error"]["code"]));
        }
        if let Some(usage) = usage(&data["usage"]) {
            self.usage = Some(usage);
        }
        // The usage chunk repeats the finish reason with an empty delta.
        let choice = &data["choices"][0];
        if let Some(finish) = choice["finish_reason"].as_str() {
            self.finish = Some(finish.to_owned());
        }
        let delta = &choice["delta"];
        if let Some(text) = delta["content"].as_str().filter(|text| !text.is_empty()) {
            self.text.push_str(text);
            events.push_back(Event::Text(text.to_owned()));
        }
        if let Some(details) = delta["reasoning_details"].as_array() {
            self.reasoning.extend(details.iter().cloned());
        }
        for part in delta["tool_calls"].as_array().into_iter().flatten() {
            self.tool_call(part)?;
        }
        Ok(false)
    }
}

impl Completions {
    /// Calls arrive in pieces addressed by index; the first piece names the call.
    fn tool_call(&mut self, part: &Value) -> Result<(), Error> {
        let index = part["index"].as_u64().ok_or(Error::InvalidResponse(None))? as usize;
        // Bounds what a stream can make this allocate; no reply makes this many calls.
        if index > 1024 {
            return Err(Error::InvalidResponse(None));
        }
        if self.calls.len() <= index {
            self.calls.resize_with(index + 1, PendingCall::default);
        }
        let pending = &mut self.calls[index];
        if let Some(id) = part["id"].as_str() {
            pending.id = id.to_owned();
        }
        if let Some(name) = part["function"]["name"].as_str() {
            pending.name.push_str(name);
        }
        if let Some(arguments) = part["function"]["arguments"].as_str() {
            pending.arguments.push_str(arguments);
        }
        Ok(())
    }

    fn finished(&mut self) -> Result<Event, Error> {
        // Truncated output is not a reply; a stream without a reason ended early.
        if !matches!(self.finish.as_deref(), Some("stop" | "tool_calls")) {
            return Err(Error::InvalidResponse(None));
        }
        let calls = self
            .calls
            .iter()
            // A call without arguments streams none, which is not JSON.
            .map(|pending| {
                let arguments = match pending.arguments.as_str() {
                    "" => "{}".to_owned(),
                    arguments => arguments.to_owned(),
                };
                call(&pending.id, &pending.name, arguments)
            })
            .collect::<Result<Vec<FunctionCall>, Error>>()?;
        let mut native = json!({"role":"assistant","content":self.text});
        if !calls.is_empty() {
            native["tool_calls"] = calls
                .iter()
                .map(|call| json!({"id":call.call_id,"type":"function","function":{"name":call.name,"arguments":call.arguments}}))
                .collect();
        }
        if !self.reasoning.is_empty() {
            native["reasoning_details"] = Value::Array(self.reasoning.clone());
        }
        Ok(Event::Done {
            reply: reply(std::mem::take(&mut self.text), calls, native)?,
            usage: self.usage,
        })
    }
}

/// One listing entry's rates, written as decimal strings; an entry without them has no price.
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

fn usage(value: &Value) -> Option<Usage> {
    Some(Usage {
        input_tokens: value["prompt_tokens"].as_u64()?,
        output_tokens: value["completion_tokens"].as_u64()?,
        cached_input_tokens: value["prompt_tokens_details"]["cached_tokens"]
            .as_u64()
            .unwrap_or(0),
        cost_usd: value["cost"].as_f64(),
    })
}

/// An error after the response started, carrying the HTTP status it would have had.
fn failure(code: &Value) -> Error {
    match code.as_u64() {
        Some(429 | 500..=599) => Error::Transient(None),
        _ => Error::Rejected,
    }
}

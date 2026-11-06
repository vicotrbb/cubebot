use crate::{
    Capabilities, Decoder, Error, Event, Limits, Message, Model, Provider, Request, Usage, call,
    reply,
};
use futures_util::Stream;
use reqwest::{Client, RequestBuilder, Url};
use serde_json::{Value, json};
use std::collections::VecDeque;

const VERSION: &str = "2026-10-21";

const PRICES: crate::prices::Columns = crate::prices::Columns {
    input: &["Base input tokens"],
    cached: &["Cache hits and refreshes"],
    output: &["Output tokens"],
};

/// `Claude Opus 5.5` as the API's `claude-opus-5.5`; dots and dates are matched up when stored.
fn model_id(name: &str) -> String {
    name.to_lowercase().replace(' ', "-")
}

pub struct Anthropic {
    http: Client,
    pub(crate) base: Url,
    pricing: Url,
    dialect: Dialect,
}

/// How an Anthropic-compatible endpoint differs from Anthropic's own.
#[derive(Clone, Copy)]
pub(crate) struct Dialect {
    pub provider: &'static str,
    /// Anthropic's own extensions: the computer toolset, strict tools, the cache breakpoint,
    /// `tool_choice: none` and limits read from the Models API.
    pub native: bool,
    /// The context window and output limit of a non-native endpoint, which reports neither.
    pub limits: (u64, u64),
    pub prices: fn(&str) -> Result<Vec<crate::Price>, Error>,
}

const API: Dialect = Dialect {
    provider: "anthropic",
    native: true,
    limits: (0, 0),
    prices: |page| crate::prices::from_markdown(page, "## Model pricing", &PRICES, model_id),
};

impl Anthropic {
    pub fn new() -> Result<Self, Error> {
        let parse = |url| Url::parse(url).map_err(|error| Error::Configuration(Some(error.into())));
        Self::base(
            parse("https://api.anthropic.com/v1/")?,
            parse("https://platform.claude.com/docs/en/about-claude/pricing.md")?,
        )
    }

    pub fn for_loopback_tests(origin: &str) -> Result<Self, Error> {
        let base = crate::loopback(origin)?;
        let pricing = base
            .join("pricing.md")
            .map_err(|error| Error::Configuration(Some(error.into())))?;
        Self::base(base, pricing)
    }

    pub(crate) fn base(base: Url, pricing: Url) -> Result<Self, Error> {
        Ok(Self {
            http: crate::client()?,
            base,
            pricing,
            dialect: API,
        })
    }

    pub(crate) fn with(self, dialect: Dialect) -> Self {
        Self { dialect, ..self }
    }

    #[tracing::instrument(name = "provider_request", skip_all, fields(provider = self.dialect.provider))]
    pub async fn prices(&self) -> Result<Vec<crate::Price>, Error> {
        let page = crate::prices::page(&self.http, self.pricing.clone()).await?;
        (self.dialect.prices)(&page)
    }

    fn authorized(&self, request: RequestBuilder, key: &str) -> RequestBuilder {
        request
            .header("x-api-key", key)
            .header("anthropic-version", VERSION)
    }
}

impl Provider for Anthropic {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            tool_calls: true,
            model_limits: true,
        }
    }

    #[tracing::instrument(name = "provider_request", skip_all, fields(provider = self.dialect.provider, model = model))]
    async fn limits(&self, model: &str, key: &str) -> Result<Limits, Error> {
        if !self.dialect.native {
            let (window, output) = self.dialect.limits;
            return Ok(Limits {
                context_window: Some(window),
                max_output_tokens: Some(output),
            });
        }
        let mut url = self
            .base
            .join("models/")
            .map_err(|error| Error::Configuration(Some(error.into())))?;
        // A pushed segment is percent-encoded, so a model ID cannot change the path.
        url.path_segments_mut()
            .map_err(|()| Error::Configuration(None))?
            .pop_if_empty()
            .push(model);
        let response = crate::send(self.authorized(self.http.get(url), key)).await?;
        let model = crate::json(response).await?;
        Ok(Limits {
            context_window: model["max_input_tokens"].as_u64(),
            max_output_tokens: model["max_tokens"].as_u64(),
        })
    }

    #[tracing::instrument(name = "provider_request", skip_all, fields(provider = self.dialect.provider))]
    async fn models(&self, key: &str) -> Result<Vec<Model>, Error> {
        let mut url = self
            .base
            .join("models")
            .map_err(|error| Error::Configuration(Some(error.into())))?;
        url.query_pairs_mut().append_pair("limit", "1000");
        let response = crate::send(self.authorized(self.http.get(url), key)).await?;
        let body = crate::json(response).await?;
        let fixed = (!self.dialect.native).then_some(self.dialect.limits.0);
        crate::models(&body, |entry| entry["max_input_tokens"].as_u64().or(fixed))
    }

    fn stream<'a>(
        &'a self,
        request: &'a Request<'a>,
    ) -> impl Stream<Item = Result<Event, Error>> + Send + 'a {
        crate::events::<Messages>(self.dialect.provider, request.model, async move {
            let url = self
                .base
                .join("messages")
                .map_err(|error| Error::Configuration(Some(error.into())))?;
            let body = body(request, self.dialect.native)?;
            crate::send(
                self.authorized(self.http.post(url), request.key)
                    .json(&body),
            )
            .await
        })
    }
}

fn body(request: &Request, native: bool) -> Result<Value, Error> {
    // The Messages API requires an output limit; callers pass the model's own.
    let limit = request
        .max_output_tokens
        .ok_or(Error::Configuration(None))?;
    let computer = crate::computer_calls(request.messages, |block| {
        (block["toolset_name"] == "computer").then(|| block["id"].as_str())?
    });
    let mut messages: Vec<Value> = Vec::new();
    for message in request.messages {
        let value = match message {
            Message::User(text) => json!({"role":"user","content":text}),
            Message::Assistant(reply) if reply.native.is_array() => {
                json!({"role":"assistant","content":reply.native})
            }
            Message::Assistant(reply) => json!({"role":"assistant","content":reply.text}),
            Message::ToolResult {
                call_id,
                output,
                image,
            } => {
                let result = tool_result(
                    call_id,
                    output,
                    image.as_deref(),
                    computer.contains(&&**call_id),
                );
                // Parallel results answer one assistant message, so they share one user message.
                if let Some(Value::Array(content)) = messages
                    .last_mut()
                    .filter(|last| last["content"][0]["type"] == "tool_result")
                    .map(|last| &mut last["content"])
                {
                    content.push(result);
                    continue;
                }
                json!({"role":"user","content":[result]})
            }
        };
        messages.push(value);
    }
    let mut body =
        json!({"model":request.model,"max_tokens":limit,"messages":messages,"stream":true});
    if native {
        // One breakpoint moves to the end of the history each turn, so the resent prefix is read from cache.
        body["cache_control"] = json!({"type":"ephemeral"});
    }
    if !request.system.is_empty() {
        body["system"] = json!(request.system);
    }
    if !request.tools.is_empty() {
        body["tools"] = if native {
            native_tools(request.tools)
        } else {
            plain_tools(request.tools)
        };
        // A compatible endpoint may accept only `auto`; a call made anyway is an ordinary stray call.
        body["tool_choice"] = if request.tools_disabled && native {
            json!({"type":"none"})
        } else {
            json!({"type":"auto"})
        };
    }
    Ok(body)
}

fn native_tools(tools: &[crate::Tool]) -> Value {
    tools
        .iter()
        .zip(strict_within_limits(tools))
        .map(|(tool, strict)| {
            if tool.computer {
                json!({"type":"computer_toolset_20260801","configs":unserved()})
            } else {
                json!({"name":tool.name,"description":tool.description,"input_schema":tool.parameters,"strict":strict})
            }
        })
        .collect()
}

/// Without the computer toolset the computer is the ordinary function Chat Completions providers
/// get, and its calls still arrive as `{"action": member, ...input}`.
fn plain_tools(tools: &[crate::Tool]) -> Value {
    tools
        .iter()
        .map(|tool| {
            let function = crate::completions::function(tool);
            let function = &function["function"];
            json!({"name":function["name"],"description":function["description"],"input_schema":function["parameters"]})
        })
        .collect()
}

/// The API refuses a request with more than 20 strict tools, or 24 optional or 16 union
/// parameters across their schemas; tools past those limits are sent advisory.
fn strict_within_limits(tools: &[crate::Tool]) -> Vec<bool> {
    let mut spent = Cost::default();
    tools
        .iter()
        .map(|tool| {
            if !tool.strict || tool.computer {
                return false;
            }
            let next = spent.add(&cost(&tool.parameters));
            let fits = next.tools <= 20 && next.optional <= 24 && next.unions <= 16;
            if fits {
                spent = next;
            }
            fits
        })
        .collect()
}

#[derive(Default)]
struct Cost {
    tools: usize,
    optional: usize,
    unions: usize,
}

impl Cost {
    fn add(&self, other: &Cost) -> Cost {
        Cost {
            tools: self.tools + 1,
            optional: self.optional + other.optional,
            unions: self.unions + other.unions,
        }
    }
    fn merge(&mut self, other: Cost) {
        self.optional += other.optional;
        self.unions += other.unions;
    }
}

fn cost(schema: &Value) -> Cost {
    let mut total = Cost::default();
    match schema {
        Value::Object(node) => {
            if node.get("anyOf").is_some() || node.get("type").is_some_and(Value::is_array) {
                total.unions += 1;
            }
            if let Some(Value::Object(properties)) = node.get("properties") {
                let required = node.get("required").and_then(Value::as_array);
                total.optional += properties
                    .keys()
                    .filter(|name| !required.is_some_and(|names| names.iter().any(|n| n == *name)))
                    .count();
            }
            node.values().for_each(|child| total.merge(cost(child)));
        }
        Value::Array(items) => items.iter().for_each(|child| total.merge(cost(child))),
        _ => {}
    }
    total
}

fn tool_use(block: &Value) -> Result<crate::FunctionCall, Error> {
    let field = |name: &str| block[name].as_str().ok_or(Error::InvalidResponse(None));
    if block["toolset_name"] != "computer" {
        return call(field("id")?, field("name")?, block["input"].to_string());
    }
    let mut input = block["input"].clone();
    let input = input.as_object_mut().ok_or(Error::InvalidResponse(None))?;
    input.insert("action".into(), json!(field("name")?));
    call(
        field("id")?,
        "computer",
        Value::Object(input.clone()).to_string(),
    )
}

fn unserved() -> Value {
    [
        "zoom",
        "hold_key",
        "left_mouse_down",
        "left_mouse_up",
        "cursor_position",
    ]
    .into_iter()
    .map(|member| (member.to_owned(), json!({"enabled":false})))
    .collect::<serde_json::Map<_, _>>()
    .into()
}

fn tool_result(call_id: &str, output: &str, image: Option<&str>, computer: bool) -> Value {
    let mut result = json!({"type":"tool_result","tool_use_id":call_id,"content":output});
    if let Some(image) = image {
        result["content"] = json!([
            {"type":"text","text":output},
            {"type":"image","source":{"type":"base64","media_type":"image/png","data":image}}
        ]);
    }
    if computer {
        result["toolset_name"] = json!("computer");
    }
    result
}

#[derive(Default)]
struct Messages {
    blocks: Vec<Value>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cached_input_tokens: u64,
    stop: Option<String>,
}
impl Decoder for Messages {
    fn decode(&mut self, data: Value, events: &mut VecDeque<Event>) -> Result<bool, Error> {
        match data["type"].as_str() {
            Some("message_start") => self.usage(&data["message"]["usage"]),
            Some("content_block_start") => self.start(&data)?,
            Some("content_block_delta") => self.delta(&data, events)?,
            Some("content_block_stop") => self.stop(&data)?,
            Some("message_delta") => {
                self.stop = data["delta"]["stop_reason"].as_str().map(Into::into);
                self.usage(&data["usage"]);
            }
            Some("message_stop") => {
                events.push_back(self.finish()?);
                return Ok(true);
            }
            Some("error") => {
                return Err(match data["error"]["type"].as_str() {
                    Some("overloaded_error" | "api_error" | "rate_limit_error") => {
                        Error::Transient(None)
                    }
                    _ => Error::Rejected,
                });
            }
            _ => (),
        }
        Ok(false)
    }
}

impl Messages {
    /// Counts are cumulative; input includes cache reads and writes.
    fn usage(&mut self, usage: &Value) {
        if let Some(input) = usage["input_tokens"].as_u64() {
            let cached = ["cache_creation_input_tokens", "cache_read_input_tokens"]
                .iter()
                .filter_map(|field| usage[*field].as_u64())
                .sum::<u64>();
            self.input_tokens = Some(input + cached);
            // shortcut: cache writes count as ordinary input, slightly under their rate.
            self.cached_input_tokens = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
        }
        if let Some(output) = usage["output_tokens"].as_u64() {
            self.output_tokens = Some(output);
        }
    }

    fn block(&mut self, data: &Value) -> Result<&mut Value, Error> {
        data["index"]
            .as_u64()
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| self.blocks.get_mut(index))
            .ok_or(Error::InvalidResponse(None))
    }

    fn start(&mut self, data: &Value) -> Result<(), Error> {
        if data["index"].as_u64() != Some(self.blocks.len() as u64) {
            return Err(Error::InvalidResponse(None));
        }
        let mut block = data["content_block"].clone();
        if block["type"] == "tool_use" {
            // Arguments arrive as JSON fragments, parsed when the block stops.
            block["input"] = json!("");
        }
        self.blocks.push(block);
        Ok(())
    }

    fn delta(&mut self, data: &Value, events: &mut VecDeque<Event>) -> Result<(), Error> {
        let delta = &data["delta"];
        let (field, piece) = match delta["type"].as_str() {
            Some("text_delta") => ("text", &delta["text"]),
            Some("input_json_delta") => ("input", &delta["partial_json"]),
            Some("thinking_delta") => ("thinking", &delta["thinking"]),
            Some("signature_delta") => ("signature", &delta["signature"]),
            _ => return Ok(()),
        };
        let piece = piece.as_str().ok_or(Error::InvalidResponse(None))?;
        match &mut self.block(data)?[field] {
            Value::String(current) => current.push_str(piece),
            slot => *slot = json!(piece),
        }
        if field == "text" {
            events.push_back(Event::Text(piece.into()));
        }
        Ok(())
    }

    fn stop(&mut self, data: &Value) -> Result<(), Error> {
        let block = self.block(data)?;
        if block["type"] == "tool_use" {
            let arguments = block["input"]
                .as_str()
                .ok_or(Error::InvalidResponse(None))?;
            block["input"] = match arguments {
                "" => json!({}),
                arguments => serde_json::from_str(arguments)
                    .map_err(|error| Error::InvalidResponse(Some(error.into())))?,
            };
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<Event, Error> {
        match self.stop.as_deref() {
            Some("end_turn" | "tool_use" | "stop_sequence") => (),
            Some("refusal") => return Err(Error::Rejected),
            // Truncated output (`max_tokens`) or an unknown stop is not a reply.
            _ => return Err(Error::InvalidResponse(None)),
        }
        let mut text = String::new();
        let mut calls = Vec::new();
        for block in &self.blocks {
            match block["type"].as_str() {
                Some("text") => text.push_str(block["text"].as_str().unwrap_or_default()),
                Some("tool_use") => calls.push(tool_use(block)?),
                _ => (),
            }
        }
        let usage =
            self.input_tokens
                .zip(self.output_tokens)
                .map(|(input_tokens, output_tokens)| Usage {
                    input_tokens,
                    output_tokens,
                    cached_input_tokens: self.cached_input_tokens,
                    cost_usd: None,
                });
        Ok(Event::Done {
            reply: reply(text, calls, Value::Array(std::mem::take(&mut self.blocks)))?,
            usage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tool;

    fn tool(name: String, parameters: Value) -> Tool {
        Tool {
            name,
            description: String::new(),
            parameters,
            strict: true,
            computer: false,
        }
    }

    fn sent(tools: &[Tool]) -> Vec<Value> {
        let request = Request {
            model: "claude-sonnet-5-5",
            key: "",
            system: "",
            messages: &[],
            tools,
            tools_disabled: false,
            max_output_tokens: Some(1),
        };
        body(&request, true).unwrap()["tools"]
            .as_array()
            .unwrap()
            .clone()
    }

    #[test]
    fn strict_tools_stay_within_the_api_limits() {
        let union = json!({"type":"object","properties":{"a":{"type":["string","null"]}},"required":["a"],"additionalProperties":false});
        let optional = json!({"type":"object","properties":{"a":{"type":"string"},"b":{"type":"string"}},"additionalProperties":false});
        let tools: Vec<Tool> = (0..46)
            .map(|index| match index % 3 {
                0 => tool(format!("union{index}"), union.clone()),
                1 => tool(format!("optional{index}"), optional.clone()),
                _ => tool(format!("plain{index}"), json!({"type":"object"})),
            })
            .collect();
        let sent = sent(&tools);
        assert_eq!(sent.len(), 46);
        let strict: Vec<&Value> = sent.iter().filter(|tool| tool["strict"] == true).collect();
        assert_eq!(strict.len(), 20);
        let named = |prefix: &str| {
            strict
                .iter()
                .filter(|tool| tool["name"].as_str().unwrap().starts_with(prefix))
                .count()
        };
        assert!(named("union") <= 16);
        assert!(named("optional") * 2 <= 24);
        assert_eq!(sent[0]["strict"], true);
    }

    #[test]
    fn a_few_strict_tools_are_all_sent_strict() {
        let tools = vec![tool("one".into(), json!({"type":"object"}))];
        assert_eq!(sent(&tools)[0]["strict"], true);
    }
}

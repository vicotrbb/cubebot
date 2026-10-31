//! Anthropic Messages over SSE. Content blocks, including signed thinking, are kept for replay.
use crate::{
    Capabilities, Decoder, Error, Event, Limits, Message, Provider, Request, Usage, call, reply,
};
use futures_util::Stream;
use reqwest::{Client, RequestBuilder, Url};
use serde_json::{Value, json};
use std::collections::VecDeque;

const VERSION: &str = "2026-10-21";

pub struct Anthropic {
    http: Client,
    /// Ends in `/`; `messages` and `models/{id}` resolve against it.
    base: Url,
}

impl Anthropic {
    pub fn new() -> Result<Self, Error> {
        Self::base(
            Url::parse("https://api.anthropic.com/v1/")
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

    #[tracing::instrument(name = "provider_request", skip_all, fields(provider = "anthropic", model = model))]
    async fn limits(&self, model: &str, key: &str) -> Result<Limits, Error> {
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

    fn stream<'a>(
        &'a self,
        request: &'a Request<'a>,
    ) -> impl Stream<Item = Result<Event, Error>> + Send + 'a {
        crate::events::<Messages>("anthropic", request.model, async move {
            let url = self
                .base
                .join("messages")
                .map_err(|error| Error::Configuration(Some(error.into())))?;
            let body = body(request)?;
            crate::send(
                self.authorized(self.http.post(url), request.key)
                    .json(&body),
            )
            .await
        })
    }
}

fn body(request: &Request) -> Result<Value, Error> {
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
                let result = tool_result(call_id, output, image, computer.contains(&&**call_id));
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
    // Automatic caching moves one breakpoint to the end of the history each turn, so the
    // resent prefix is read from cache instead of billed in full.
    let mut body = json!({"model":request.model,"max_tokens":limit,"messages":messages,
        "stream":true,"cache_control":{"type":"ephemeral"}});
    if !request.system.is_empty() {
        body["system"] = json!(request.system);
    }
    if !request.tools.is_empty() {
        body["tools"] = request
            .tools
            .iter()
            .map(|tool| match tool.computer {
                true => json!({"type":"computer_toolset_20260801","configs":unserved()}),
                false => json!({"name":tool.name,"description":tool.description,"input_schema":tool.parameters,"strict":tool.strict}),
            })
            .collect();
        body["tool_choice"] = if request.tools_disabled {
            json!({"type":"none"})
        } else {
            json!({"type":"auto"})
        };
    }
    Ok(body)
}

/// A computer toolset call becomes a `computer` call whose action is the member's name.
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

/// Turns off the toolset members the desktop arbiter does not serve.
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

/// A result with its image after the text; a computer toolset result echoes its toolset.
fn tool_result(call_id: &str, output: &str, image: &Option<String>, computer: bool) -> Value {
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
        }
        if let Some(output) = usage["output_tokens"].as_u64() {
            self.output_tokens = Some(output);
        }
    }

    fn block(&mut self, data: &Value) -> Result<&mut Value, Error> {
        let index = data["index"].as_u64().ok_or(Error::InvalidResponse(None))? as usize;
        self.blocks
            .get_mut(index)
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
                });
        Ok(Event::Done {
            reply: reply(text, calls, Value::Array(std::mem::take(&mut self.blocks)))?,
            usage,
        })
    }
}

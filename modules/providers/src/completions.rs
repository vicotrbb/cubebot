//! Assistant messages are replayed whole, so reasoning details and thought signatures survive tool calls.
use crate::{Decoder, Error, Event, FunctionCall, Message, Request, Tool, Usage, call, reply};
use serde_json::{Value, json};
use std::collections::VecDeque;

pub(crate) fn body(request: &Request, tool: fn(&Tool) -> Value) -> Value {
    let mut messages = vec![json!({"role":"system","content":request.system})];
    // A person-role image may only follow the whole run of tool results it belongs to.
    let mut images = Vec::new();
    for message in request.messages {
        if !matches!(message, Message::ToolResult { .. }) {
            flush(&mut messages, &mut images);
        }
        match message {
            Message::User(text) => messages.push(json!({"role":"user","content":text})),
            Message::Image { media_type, data } => messages.push(json!({"role":"user","content":[
                {"type":"image_url","image_url":{"url":format!("data:{media_type};base64,{data}")}}
            ]})),
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
    let mut body = json!({"model":request.model,"messages":messages,"stream":true});
    if let Some(limit) = request.max_output_tokens {
        body["max_tokens"] = json!(limit);
    }
    if !request.tools.is_empty() {
        body["tools"] = request.tools.iter().map(tool).collect();
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

/// These providers have no computer tool of their own, so it is an ordinary function.
pub(crate) fn function(tool: &Tool) -> Value {
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
    /// Gemini's `extra_content`: its thought signature must come back with the call.
    extra: Option<Value>,
}

#[derive(Default)]
pub(crate) struct Completions {
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
    /// Calls arrive in pieces addressed by index, but Gemini may omit the index or repeat 0 for parallel calls, each then whole with its own id.
    fn slot(&self, part: &Value) -> usize {
        let id = part["id"].as_str().filter(|id| !id.is_empty());
        let last = self.calls.len().saturating_sub(1);
        match (part["index"].as_u64(), id) {
            (Some(index), Some(id))
                if self
                    .calls
                    .get(index as usize)
                    .is_some_and(|call| !call.id.is_empty() && call.id != id) =>
            {
                self.calls.len()
            }
            (Some(index), _) => index as usize,
            (None, _) if part["function"]["name"].is_string() => self.calls.len(),
            (None, Some(_)) => self.calls.len(),
            (None, None) => last,
        }
    }

    fn tool_call(&mut self, part: &Value) -> Result<(), Error> {
        let index = self.slot(part);
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
        if !part["extra_content"].is_null() {
            pending.extra = Some(part["extra_content"].clone());
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
            .enumerate()
            // A call without arguments streams none, which is not JSON. Gemini may leave the id empty, and results must still pair.
            .map(|(position, pending)| {
                let arguments = match pending.arguments.as_str() {
                    "" => "{}".to_owned(),
                    arguments => arguments.to_owned(),
                };
                let generated = format!("call_{position}");
                let id = Some(pending.id.as_str())
                    .filter(|id| !id.is_empty())
                    .unwrap_or(&generated);
                call(id, &pending.name, arguments)
            })
            .collect::<Result<Vec<FunctionCall>, Error>>()?;
        let mut native = json!({"role":"assistant","content":self.text});
        if !calls.is_empty() {
            native["tool_calls"] = calls
                .iter()
                .zip(&self.calls)
                .map(|(call, pending)| {
                    let mut replayed = json!({"id":call.call_id,"type":"function","function":{"name":call.name,"arguments":call.arguments}});
                    if let Some(extra) = &pending.extra {
                        replayed["extra_content"] = extra.clone();
                    }
                    replayed
                })
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

fn usage(value: &Value) -> Option<Usage> {
    Some(Usage {
        input_tokens: value["prompt_tokens"].as_u64()?,
        output_tokens: value["completion_tokens"].as_u64()?,
        cached_input_tokens: value["prompt_tokens_details"]["cached_tokens"]
            .as_u64()
            .unwrap_or(0),
        // OpenRouter reports dollars, xAI ticks of a ten-billionth of one.
        cost_usd: value["cost"]
            .as_f64()
            .or_else(|| Some(value["cost_in_usd_ticks"].as_f64()? / 1e10)),
    })
}

fn failure(code: &Value) -> Error {
    match code.as_u64() {
        Some(429 | 500..=599) => Error::Transient(None),
        _ => Error::Rejected,
    }
}

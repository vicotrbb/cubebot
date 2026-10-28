//! OpenAI Responses over SSE, stateless (`store: false`) with encrypted reasoning replay.
use crate::{
    Capabilities, Decoder, Error, Event, Limits, Message, Provider, Request, Usage, call, reply,
};
use futures_util::Stream;
use reqwest::{Client, Url};
use serde_json::{Value, json};
use std::collections::VecDeque;

pub struct OpenAi {
    http: Client,
    endpoint: Url,
}

impl OpenAi {
    pub fn new() -> Result<Self, Error> {
        Self::endpoint(
            Url::parse("https://api.openai.com/v1/responses").map_err(|_| Error::Configuration)?,
        )
    }

    pub fn for_loopback_tests(origin: &str) -> Result<Self, Error> {
        Self::endpoint(
            crate::loopback(origin)?
                .join("responses")
                .map_err(|_| Error::Configuration)?,
        )
    }

    fn endpoint(endpoint: Url) -> Result<Self, Error> {
        Ok(Self {
            http: crate::client()?,
            endpoint,
        })
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

    fn stream<'a>(
        &'a self,
        request: &'a Request<'a>,
    ) -> impl Stream<Item = Result<Event, Error>> + Send + 'a {
        crate::events::<Responses>(crate::send(
            self.http
                .post(self.endpoint.clone())
                .bearer_auth(request.key)
                .json(&body(request)),
        ))
    }
}

fn body(request: &Request) -> Value {
    let mut input = vec![json!({"role":"system","content":request.system})];
    for message in request.messages {
        match message {
            Message::User(text) => input.push(json!({"role":"user","content":text})),
            Message::Assistant(reply) => match &reply.native {
                Value::Array(items) => input.extend(items.iter().cloned()),
                _ => input.push(json!({"role":"assistant","content":reply.text})),
            },
            Message::ToolResult { call_id, output } => {
                input.push(json!({"type":"function_call_output","call_id":call_id,"output":output}))
            }
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
            .map(|tool| json!({"type":"function","name":tool.name,"description":tool.description,"strict":true,"parameters":tool.parameters}))
            .collect();
        body["parallel_tool_calls"] = json!(false);
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
                let delta = data["delta"].as_str().ok_or(Error::InvalidResponse)?;
                events.push_back(Event::Text(delta.into()));
                Ok(false)
            }
            Some("response.completed") => {
                events.push_back(completed(&data["response"])?);
                Ok(true)
            }
            // Truncated output is not a reply.
            Some("response.incomplete") => Err(Error::InvalidResponse),
            Some("response.failed" | "error") => Err(Error::Transient(None)),
            _ => Ok(false),
        }
    }
}

/// The final response is authoritative; deltas were only progress.
fn completed(response: &Value) -> Result<Event, Error> {
    if response["status"] != "completed" {
        return Err(Error::InvalidResponse);
    }
    let output = response["output"]
        .as_array()
        .ok_or(Error::InvalidResponse)?;
    let mut text = String::new();
    let mut function = None;
    for item in output {
        match item["type"].as_str() {
            Some("reasoning") => (),
            Some("message") => append_content(&mut text, &item["content"])?,
            Some("function_call") if function.is_none() => function = Some(function_call(item)?),
            _ => return Err(Error::InvalidResponse),
        }
    }
    let usage = &response["usage"];
    let usage = usage["input_tokens"]
        .as_u64()
        .zip(usage["output_tokens"].as_u64())
        .map(|(input_tokens, output_tokens)| Usage {
            input_tokens,
            output_tokens,
        });
    Ok(Event::Done {
        reply: reply(text, function, Value::Array(output.clone()))?,
        usage,
    })
}

fn append_content(text: &mut String, value: &Value) -> Result<(), Error> {
    for part in value.as_array().ok_or(Error::InvalidResponse)? {
        if part["type"] != "output_text" {
            return Err(Error::InvalidResponse);
        }
        text.push_str(part["text"].as_str().ok_or(Error::InvalidResponse)?);
    }
    Ok(())
}

fn function_call(item: &Value) -> Result<crate::FunctionCall, Error> {
    let field = |name: &str| item[name].as_str().ok_or(Error::InvalidResponse);
    call(
        field("call_id")?,
        field("name")?,
        field("arguments")?.into(),
    )
}

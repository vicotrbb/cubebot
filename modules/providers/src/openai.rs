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
            Url::parse("https://api.openai.com/v1/responses")
                .map_err(|error| Error::Configuration(Some(error.into())))?,
        )
    }

    pub fn for_loopback_tests(origin: &str) -> Result<Self, Error> {
        Self::endpoint(
            crate::loopback(origin)?
                .join("responses")
                .map_err(|error| Error::Configuration(Some(error.into())))?,
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
            .map(|tool| match tool.computer {
                true => json!({"type":"computer"}),
                false => json!({"type":"function","name":tool.name,"description":tool.description,"strict":tool.strict,"parameters":tool.parameters}),
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
            Some("response.failed" | "error") => Err(Error::Transient(None)),
            _ => Ok(false),
        }
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

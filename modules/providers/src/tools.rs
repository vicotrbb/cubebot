//! Stateless provider tool-call transport. Tool schemas and policy belong to the caller.
use crate::{Error, OpenAi, append_content};
use serde_json::{Value, json};

pub struct FunctionCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}
pub struct Step {
    pub output: Vec<Value>,
    pub call: Option<FunctionCall>,
    pub text: String,
    /// Input and output tokens, zero when the provider omits them.
    pub usage: (u64, u64),
}
impl OpenAi {
    pub async fn tool_step(
        &self,
        model: &str,
        key: &str,
        input: &[Value],
        tools: &[Value],
    ) -> Result<Step, Error> {
        let mut body = json!({
            "model":model,"input":input,"store":false,"max_output_tokens":2048,
            "include":["reasoning.encrypted_content"]
        });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
            body["parallel_tool_calls"] = json!(false);
        }
        parse(self.send(key, body).await?)
    }
}
fn parse(value: Value) -> Result<Step, Error> {
    if value["status"] != "completed" {
        return Err(Error::InvalidResponse);
    }
    let output = value["output"]
        .as_array()
        .ok_or(Error::InvalidResponse)?
        .clone();
    let mut step = Step {
        output,
        call: None,
        text: String::new(),
        usage: (
            value["usage"]["input_tokens"].as_u64().unwrap_or(0),
            value["usage"]["output_tokens"].as_u64().unwrap_or(0),
        ),
    };
    for item in &step.output {
        match item["type"].as_str() {
            Some("reasoning") => (),
            Some("message") => append_content(&mut step.text, &item["content"])?,
            Some("function_call") if step.call.is_none() => step.call = Some(call(item)?),
            _ => return Err(Error::InvalidResponse),
        }
    }
    if (step.call.is_none() && step.text.is_empty())
        || step.text.len() > 32768
        || step.text.contains('\0')
    {
        return Err(Error::InvalidResponse);
    }
    Ok(step)
}
fn call(item: &Value) -> Result<FunctionCall, Error> {
    let name = item["name"].as_str().ok_or(Error::InvalidResponse)?;
    let call_id = item["call_id"].as_str().ok_or(Error::InvalidResponse)?;
    let arguments = item["arguments"].as_str().ok_or(Error::InvalidResponse)?;
    if call_id.is_empty()
        || call_id.len() > 256
        || name.is_empty()
        || name.len() > 128
        || arguments.len() > 16384
    {
        return Err(Error::InvalidResponse);
    }
    Ok(FunctionCall {
        name: name.into(),
        call_id: call_id.into(),
        arguments: arguments.into(),
    })
}

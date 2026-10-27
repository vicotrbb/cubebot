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
}
impl OpenAi {
    pub async fn tool_step(
        &self,
        model: &str,
        key: &str,
        input: &[Value],
        tools: &[Value],
    ) -> Result<Step, Error> {
        let value = self
            .send(
                key,
                json!({
                    "model":model,"input":input,"store":false,"max_output_tokens":2048,
                    "parallel_tool_calls":false,"include":["reasoning.encrypted_content"],
                    "tools":tools
                }),
            )
            .await?;
        parse(value)
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

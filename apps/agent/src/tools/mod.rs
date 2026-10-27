//! Model arguments are data. Only the trusted runtime supplies invocation authority.
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) fn terminal_definition() -> Value {
    json!({"type":"function","name":"terminal",
        "description":"Execute a bounded command using /bin/bash --noprofile --norc in the persistent workspace. The result includes the command exit status as code; run programs directly without appending echo $? which masks that status. Commands are serialized. Captured combined output is limited to 8192 UTF-8 bytes. Check exit status and verify results. Cancellation cannot undo effects. Never blindly repeat uncertain effects.",
        "strict":true,"parameters":{"type":"object","properties":{
            "command":{"type":"string"},"seconds":{"type":"integer","minimum":1,"maximum":30}},
            "required":["command","seconds"],"additionalProperties":false}})
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TerminalArguments {
    pub command: String,
    pub seconds: u64,
}
pub(crate) fn terminal_arguments(
    name: &str,
    arguments: &str,
) -> Result<TerminalArguments, &'static str> {
    if name != "terminal" {
        return Err("unavailable tool");
    }
    let arguments: TerminalArguments =
        serde_json::from_str(arguments).map_err(|_| "invalid terminal arguments")?;
    if !(1..=30).contains(&arguments.seconds)
        || arguments.command.is_empty()
        || arguments.command.len() > 8192
        || arguments.command.contains('\0')
    {
        return Err("unbounded terminal arguments");
    }
    Ok(arguments)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_arguments_never_add_authority_or_select_another_tool() {
        assert!(terminal_arguments("terminal", r#"{"command":"true","seconds":1}"#).is_ok());
        assert!(terminal_arguments("terminal", r#"{"command":"true","seconds":31}"#).is_err());
        assert!(
            terminal_arguments(
                "terminal",
                r#"{"command":"true","seconds":1,"authority":"mine"}"#
            )
            .is_err()
        );
        assert!(terminal_arguments("other", r#"{"command":"true","seconds":1}"#).is_err());
    }
}

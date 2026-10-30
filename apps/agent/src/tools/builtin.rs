//! Built-in tools. Each one is a fixed program run through the sandbox as the tool user; only
//! `edit_file` needs code of its own, run by re-executing the agent binary as that user.
use super::{Job, Running, Sandbox, Tool};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::process::ExitCode;
use uuid::Uuid;

const BASH: [&str; 3] = ["/bin/bash", "--noprofile", "--norc"];
/// Unquoted pathname expansion with word splitting disabled never evaluates the pattern. Bash
/// before 4 (macOS development hosts) lacks globstar, so `**` is refused there rather than
/// silently matching one level.
const GLOB: &str = r#"shopt -s globstar 2>/dev/null || [[ $1 != *'**'* ]] || { echo 'This bash lacks globstar; ** is unsupported.'; exit 2; }; shopt -s nullglob; IFS=; matches=($1); [[ ${#matches[@]} -gt 0 ]] && printf '%s\n' "${matches[@]}""#;
const WRITE: &str = r#"mkdir -p -- "$(dirname -- "$1")" && cat > "$1" && printf 'Wrote %s\n' "$1""#;
const RETAINED: &str = "Combined output is kept in the workspace file named in the result; longer output is shown as its head and tail.";

struct Builtin {
    definition: cubebot_providers::Tool,
    job: fn(&Value) -> Result<Job, &'static str>,
}

impl Tool for Builtin {
    fn definition(&self) -> &cubebot_providers::Tool {
        &self.definition
    }

    fn call<'a>(
        &'a self,
        sandbox: &'a Sandbox,
        id: Uuid,
        input: &Value,
    ) -> Result<Running<'a>, &'static str> {
        let job = (self.job)(input)?;
        Ok(Box::pin(sandbox.run(id, job)))
    }
}

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    let tool = |name: &str, description: String, properties: Value, job| -> Box<dyn Tool> {
        let required: Vec<&String> = properties
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, _)| key)
            .collect();
        Box::new(Builtin {
            definition: cubebot_providers::Tool {
                name: name.into(),
                description,
                parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
                strict: true,
            },
            job,
        })
    };
    let text = json!({"type":"string"});
    vec![
        tool("bash", format!("Execute a command using /bin/bash --noprofile --norc in the persistent workspace. The result includes the exit status as code; run programs directly without appending echo $? which masks that status. {RETAINED} Calls in one response run in parallel. Check exit status and verify results. Cancellation cannot undo effects. Never blindly repeat uncertain effects."),
            json!({"command":text}), bash),
        tool("read_file", format!("Read a file. Relative paths start at the workspace. {RETAINED}"),
            json!({"path":text}), read_file),
        tool("write_file", "Create or replace a file with the given content, creating parent directories.".into(),
            json!({"path":text,"content":text}), write_file),
        tool("edit_file", "Replace old_string with new_string in a file. old_string must occur exactly once unless replace_all is true.".into(),
            json!({"path":text,"old_string":text,"new_string":text,"replace_all":{"type":"boolean"}}), edit_job),
        tool("glob", format!("List paths matching a bash glob pattern relative to the workspace; ** matches directories recursively. Exit code 1 means no match. {RETAINED}"),
            json!({"pattern":text}), glob),
        tool("grep", format!("Search files under path recursively for an extended regular expression, printing file:line:text. Exit code 1 means no match. {RETAINED}"),
            json!({"pattern":text,"path":text}), grep),
    ]
}

/// A script run by bash with positional arguments, which bash never evaluates as code.
pub(super) fn shell(script: &str, arguments: &[&str]) -> Job {
    let mut argv: Vec<String> = BASH.map(Into::into).into();
    argv.extend(["-c".into(), script.into(), "tool".into()]);
    argv.extend(arguments.iter().map(|argument| argument.to_string()));
    Job { argv, stdin: None }
}

/// Typed arguments with unknown fields refused, so model input can never add options.
fn parse<T: DeserializeOwned>(input: &Value) -> Result<T, &'static str> {
    T::deserialize(input).map_err(|_| "invalid tool arguments")
}

/// Program arguments cannot hold NUL bytes.
fn argument(text: &str) -> Result<&str, &'static str> {
    (!text.is_empty() && !text.contains('\0'))
        .then_some(text)
        .ok_or("invalid tool arguments")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    command: String,
}
fn bash(input: &Value) -> Result<Job, &'static str> {
    let Command { command } = parse(input)?;
    Ok(shell(argument(&command)?, &[]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Path {
    path: String,
}
fn read_file(input: &Value) -> Result<Job, &'static str> {
    let Path { path } = parse(input)?;
    Ok(Job {
        argv: vec!["cat".into(), "--".into(), argument(&path)?.into()],
        stdin: None,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Write {
    path: String,
    content: String,
}
fn write_file(input: &Value) -> Result<Job, &'static str> {
    let Write { path, content } = parse(input)?;
    Ok(write(argument(&path)?, content.into_bytes()))
}

pub(super) fn write(path: &str, content: Vec<u8>) -> Job {
    Job {
        stdin: Some(content),
        ..shell(WRITE, &[path])
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    path: String,
    old_string: String,
    new_string: String,
    replace_all: bool,
}
fn edit_job(input: &Value) -> Result<Job, &'static str> {
    let edit: Edit = parse(input)?;
    argument(&edit.path)?;
    let agent = std::env::current_exe().map_err(|_| "unavailable tool")?;
    Ok(Job {
        argv: vec![agent.to_string_lossy().into(), "edit-file".into()],
        stdin: Some(input.to_string().into_bytes()),
    })
}

/// `cubebot-agent edit-file`: applies the edit on standard input as the tool user.
pub fn edit_file() -> ExitCode {
    let mut input = String::new();
    let result = std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)
        .map_err(|error| error.to_string())
        .and_then(|_| edit(&input));
    match result {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            println!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn edit(input: &str) -> Result<String, String> {
    let edit: Edit = serde_json::from_str(input).map_err(|error| error.to_string())?;
    if edit.old_string.is_empty() {
        return Err("old_string must not be empty".into());
    }
    let text = std::fs::read_to_string(&edit.path).map_err(|error| error.to_string())?;
    let count = text.matches(&edit.old_string).count();
    let text = match (count, edit.replace_all) {
        (0, _) => return Err(format!("old_string not found in {}", edit.path)),
        (1, _) | (_, true) => text.replace(&edit.old_string, &edit.new_string),
        (count, false) => {
            return Err(format!(
                "old_string occurs {count} times in {}; add context to make it unique or set replace_all",
                edit.path
            ));
        }
    };
    std::fs::write(&edit.path, text).map_err(|error| error.to_string())?;
    Ok(format!("Replaced {count} occurrence(s) in {}", edit.path))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pattern {
    pattern: String,
}
fn glob(input: &Value) -> Result<Job, &'static str> {
    let Pattern { pattern } = parse(input)?;
    Ok(shell(GLOB, &[argument(&pattern)?]))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    pattern: String,
    path: String,
}
fn grep(input: &Value) -> Result<Job, &'static str> {
    let Search { pattern, path } = parse(input)?;
    // Retained output would otherwise match every earlier search.
    let exclude = format!("--exclude-dir={}", super::OUTPUT);
    let argv = [
        "grep",
        "-rnIE",
        &exclude,
        "-e",
        argument(&pattern)?,
        "--",
        argument(&path)?,
    ];
    Ok(Job {
        argv: argv.map(Into::into).into(),
        stdin: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cubebot_protocol::terminal::Outcome;

    async fn run(sandbox: &Sandbox, name: &str, input: Value) -> (Option<i32>, String) {
        let tools = tools();
        let tool = tools
            .iter()
            .find(|tool| tool.definition().name == name)
            .unwrap();
        match tool.call(sandbox, Uuid::new_v4(), &input).unwrap().await {
            Outcome::Exited { code, output } => (code, output),
            other => panic!("unexpected outcome {other:?}"),
        }
    }

    #[test]
    fn model_arguments_never_add_options() {
        let tools = tools();
        let bash = &tools[0];
        let sandbox = super::super::tests::sandbox();
        assert!(
            bash.call(&sandbox, Uuid::nil(), &json!({"command":"true"}))
                .is_ok()
        );
        for input in [
            json!({"command":"true","authority":"mine"}),
            json!({"command":""}),
            json!({"command":"a\0b"}),
            json!("true"),
        ] {
            assert!(bash.call(&sandbox, Uuid::nil(), &input).is_err(), "{input}");
        }
        std::fs::remove_dir_all(&sandbox.workspace).unwrap();
    }

    #[tokio::test]
    async fn file_tools_write_read_glob_and_grep_in_the_workspace() {
        let sandbox = super::super::tests::sandbox();
        let (code, _) = run(
            &sandbox,
            "write_file",
            json!({"path":"src/a.txt","content":"alpha\nbeta $(touch pwned)\n"}),
        )
        .await;
        assert_eq!(code, Some(0));
        let (code, output) = run(&sandbox, "read_file", json!({"path":"src/a.txt"})).await;
        assert_eq!(code, Some(0));
        assert!(
            output.starts_with("alpha\nbeta $(touch pwned)\n"),
            "{output}"
        );
        let (code, output) = run(&sandbox, "glob", json!({"pattern":"src/*.txt"})).await;
        assert_eq!(code, Some(0));
        assert!(output.starts_with("src/a.txt\n"), "{output}");
        let (code, output) = run(&sandbox, "glob", json!({"pattern":"**/*.txt"})).await;
        match code {
            Some(0) => assert!(output.starts_with("src/a.txt\n"), "{output}"),
            Some(2) => assert!(output.starts_with("This bash lacks globstar"), "{output}"),
            _ => panic!("{output}"),
        }
        let (code, _) = run(&sandbox, "glob", json!({"pattern":"$(touch pwned)*"})).await;
        assert_eq!(code, Some(1));
        let (code, output) = run(&sandbox, "grep", json!({"pattern":"^be+ta","path":"."})).await;
        assert_eq!(code, Some(0));
        assert!(output.starts_with("./src/a.txt:2:beta"), "{output}");
        assert!(!sandbox.workspace.join("pwned").exists());
        std::fs::remove_dir_all(&sandbox.workspace).unwrap();
    }

    #[test]
    fn edits_replace_one_unique_match_or_every_match_when_asked() {
        let path = std::env::temp_dir().join(format!("cubebot-edit-{}", Uuid::new_v4()));
        std::fs::write(&path, "a b a").unwrap();
        let request = |old: &str, replace_all: bool| {
            json!({"path":path,"old_string":old,"new_string":"c","replace_all":replace_all})
                .to_string()
        };
        assert!(
            edit(&request("a", false))
                .unwrap_err()
                .contains("occurs 2 times")
        );
        assert!(
            edit(&request("z", false))
                .unwrap_err()
                .contains("not found")
        );
        assert!(edit(&request("", true)).is_err());
        edit(&request("b", false)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a c a");
        edit(&request("a", true)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "c c c");
        std::fs::remove_file(path).unwrap();
    }
}

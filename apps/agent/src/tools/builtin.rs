use super::{Job, Running, Sandbox, Tool};
use cubebot_protocol::terminal::Outcome;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{process::ExitCode, time::Duration};
use tokio::sync::Mutex;
use uuid::Uuid;

const BASH: [&str; 3] = ["/bin/bash", "--noprofile", "--norc"];
const WRITE: &str = r#"mkdir -p -- "$(dirname -- "$1")" && { [ -e "$1" ] && n=Replaced || n=Created; } && cat > "$1" && printf '%s %s (%s lines)\n' "$n" "$1" "$(wc -l < "$1" | tr -d ' ')""#;
const READ: &str = r#"[ -d "$1" ] && { echo "$1 is a directory; list it with glob or bash."; exit 1; }
[ -r "$1" ] || { echo "$1 does not exist or cannot be read."; exit 1; }
[ -s "$1" ] && ! grep -qI '' "$1" && { echo "$1 is a binary file; inspect it with bash, such as file or xxd."; exit 1; }
awk -v s="$2" -v n="$3" -v budget=14000 '
NR >= s && NR < s + n && !full {
  l = $0; if (length(l) > 2000) l = substr(l, 1, 2000) " [line cut]"
  b += length(l) + 8
  if (b > budget && shown) { full = NR; next }
  printf "%6d\t%s\n", NR, l; shown = NR
}
END {
  if (NR == 0) print "(empty file)"
  else if (!shown) printf "[the file has %d lines; offset %d is past its end]\n", NR, s
  else if (shown < NR) printf "[lines %d-%d of %d; continue with offset %d]\n", s, shown, NR, shown + 1
}' < "$1""#;
const RG: [&str; 8] = [
    "rg",
    "--no-require-git",
    "--hidden",
    "--color=never",
    "--glob=!.git",
    "--glob=!.cubebot-tool-output",
    "--max-columns=300",
    "--max-columns-preview",
];
const RETAINED: &str = "Combined output is kept for at least a week in the workspace file named in the result; longer output is shown as its head and tail.";
const DEFAULT_SECONDS: u64 = 600;
const MOST_SECONDS: u64 = 3600;
const READ_LINES: u64 = 2000;

static FILES: Mutex<()> = Mutex::const_new(());

struct Planned {
    job: Job,
    limit: Duration,
}

impl From<Job> for Planned {
    fn from(job: Job) -> Self {
        Self {
            job,
            limit: Duration::from_secs(MOST_SECONDS),
        }
    }
}

struct Builtin {
    definition: cubebot_providers::Tool,
    job: fn(&Value) -> Result<Planned, &'static str>,
    writes: bool,
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
        let Planned { job, limit } = (self.job)(input)?;
        let writes = self.writes;
        Ok(Box::pin(async move {
            let _one_writer = if writes {
                Some(FILES.lock().await)
            } else {
                None
            };
            sandbox.run_within(limit, id, job).await
        }))
    }
}

struct Plan {
    definition: cubebot_providers::Tool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Todos {
    todos: Vec<Todo>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Todo {
    content: String,
    status: Status,
}

#[derive(Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Status {
    Pending,
    InProgress,
    Completed,
}

impl Tool for Plan {
    fn definition(&self) -> &cubebot_providers::Tool {
        &self.definition
    }

    fn call<'a>(
        &'a self,
        _: &'a Sandbox,
        _: Uuid,
        input: &Value,
    ) -> Result<Running<'a>, &'static str> {
        let output = planned(parse(input)?)?;
        Ok(Box::pin(async move {
            Outcome::Exited {
                code: Some(0),
                output,
            }
        }))
    }
}

fn planned(Todos { todos }: Todos) -> Result<String, &'static str> {
    let fits = |todo: &Todo| !todo.content.trim().is_empty() && todo.content.len() <= 500;
    if todos.len() > 50 || !todos.iter().all(fits) {
        return Err("a plan has at most 50 items of 1 to 500 bytes");
    }
    let working = todos
        .iter()
        .filter(|t| t.status == Status::InProgress)
        .count();
    if working > 1 {
        return Err("keep exactly one item in_progress at a time");
    }
    let done = todos
        .iter()
        .filter(|t| t.status == Status::Completed)
        .count();
    let next = todos.iter().find(|t| t.status != Status::Completed);
    Ok(match next {
        None if !todos.is_empty() => format!("Plan complete: {done} of {done} done."),
        None => "Plan cleared.".into(),
        Some(next) => format!(
            "Plan updated: {done} of {} done. Now: {}",
            todos.len(),
            next.content.trim()
        ),
    })
}

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    let tool = |name: &str, description: String, properties: Value, job, writes| -> Box<dyn Tool> {
        Box::new(Builtin {
            definition: cubebot_providers::Tool::strict(name, description, properties),
            job,
            writes,
        })
    };
    let text = json!({"type":"string"});
    let maybe = |description: &str| json!({"type":["string","null"],"description":description});
    let count = |description: &str| json!({"type":["integer","null"],"description":description});
    let todo = json!({
        "type":"object",
        "properties":{
            "content":{"type":"string","description":"One concrete step, in the imperative."},
            "status":{"type":"string","enum":["pending","in_progress","completed"]},
        },
        "required":["content","status"],
        "additionalProperties":false,
    });
    vec![
        tool("bash", format!("Execute a command using /bin/bash --noprofile --norc in the persistent workspace. The result includes the exit status as code; run programs directly without appending echo $? which masks that status. {RETAINED} Each call starts in the workspace with a fresh shell, so cd and variables do not carry over; chain them with && in one call. A call stops after timeout seconds, {DEFAULT_SECONDS} unless you set it, at most {MOST_SECONDS}; background jobs stop when it ends, so start a server with `setsid server > server.log 2>&1 &`. Calls in one response run in parallel. Prefer read_file, edit_file, grep and glob over cat, sed, grep and find. Check exit status and verify results. Cancellation cannot undo effects. Never blindly repeat uncertain effects."),
            json!({"command":text,"timeout":count("Seconds before the call stops; null for the default.")}), bash, false),
        tool("read_file", format!("Read a text file with line numbers, as `number<TAB>line`. Relative paths start at the workspace. Reads up to {READ_LINES} lines from offset (1 when null), and stops sooner at about 14 KB, saying where to continue. Read a file before you edit it, and read only the part you need of a long one. The line numbers are not part of the file."),
            json!({"path":text,"offset":count("The first line to read, from 1; null for the start."),"limit":count("How many lines to read; null for as many as fit.")}), read_file, false),
        tool("write_file", "Create a file, or replace one entirely, creating parent directories. To change part of an existing file use edit_file instead; read a file before replacing it.".into(),
            json!({"path":text,"content":text}), write_file, true),
        tool("edit_file", "Replace old_string with new_string in a file and show the change as a diff with line numbers. Copy old_string exactly from read_file output, without the line numbers, with enough surrounding lines to occur exactly once, unless replace_all is true. Edits to files never overlap: they run one at a time.".into(),
            json!({"path":text,"old_string":text,"new_string":text,"replace_all":{"type":"boolean"}}), edit_job, true),
        tool("glob", format!("List files whose paths match a gitignore-style glob such as **/*.rs or src/**/test_*.py, newest first, skipping .git and what .gitignore ignores; use bash to list ignored files. Exit code 1 means no match. {RETAINED}"),
            json!({"pattern":text,"path":maybe("The directory to search; null for the workspace.")}), glob, false),
        tool("grep", format!("Search file contents with ripgrep for a regular expression, printing path:line:text, skipping .git, binary files and what .gitignore ignores. Exit code 1 means no match. {RETAINED}"),
            json!({"pattern":text,"path":maybe("A file or directory to search; null for the workspace."),"glob":maybe("Only files matching this glob, such as *.ts or src/**; null for all."),"ignore_case":{"type":"boolean"}}), grep, false),
        Box::new(Plan {
            definition: cubebot_providers::Tool::strict(
                "todo_write",
                "Keep a visible plan for work of three or more steps. Send the whole list every time: add the steps when you start, mark one in_progress before working on it, mark it completed as soon as it is done, and add steps you discover. People watch this list in the console.",
                json!({"todos":{"type":"array","items":todo}}),
            ),
        }),
    ]
}

/// A script run by bash with positional arguments, which bash never evaluates as code.
pub(super) fn shell(script: &str, arguments: &[&str]) -> Job {
    let mut argv: Vec<String> = BASH.map(Into::into).into();
    argv.extend(["-c".into(), script.into(), "tool".into()]);
    argv.extend(arguments.iter().map(ToString::to_string));
    Job { argv, stdin: None }
}

/// Typed arguments with unknown fields refused, so model input can never add options.
pub(crate) fn parse<T: DeserializeOwned>(input: &Value) -> Result<T, &'static str> {
    T::deserialize(input).map_err(|_| "invalid tool arguments")
}

fn argument(text: &str) -> Result<&str, &'static str> {
    (!text.is_empty() && !text.contains('\0'))
        .then_some(text)
        .ok_or("invalid tool arguments")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    command: String,
    #[serde(default)]
    timeout: Option<u64>,
}
fn bash(input: &Value) -> Result<Planned, &'static str> {
    let Command { command, timeout } = parse(input)?;
    let seconds = timeout.unwrap_or(DEFAULT_SECONDS);
    if !(1..=MOST_SECONDS).contains(&seconds) {
        return Err("timeout is 1 to 3600 seconds");
    }
    Ok(Planned {
        job: shell(argument(&command)?, &[]),
        limit: Duration::from_secs(seconds),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    path: String,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    limit: Option<u64>,
}
fn read_file(input: &Value) -> Result<Planned, &'static str> {
    let Read {
        path,
        offset,
        limit,
    } = parse(input)?;
    let (offset, limit) = (offset.unwrap_or(1), limit.unwrap_or(READ_LINES));
    if offset == 0 || limit == 0 {
        return Err("offset and limit start at 1");
    }
    let (offset, limit) = (offset.to_string(), limit.min(READ_LINES).to_string());
    Ok(shell(READ, &[argument(&path)?, &offset, &limit]).into())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Write {
    path: String,
    content: String,
}
fn write_file(input: &Value) -> Result<Planned, &'static str> {
    let Write { path, content } = parse(input)?;
    Ok(write(argument(&path)?, content.into_bytes()).into())
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
fn edit_job(input: &Value) -> Result<Planned, &'static str> {
    let edit: Edit = parse(input)?;
    argument(&edit.path)?;
    let agent = std::env::current_exe().map_err(|_| "unavailable tool")?;
    Ok(Job {
        argv: vec![agent.to_string_lossy().into(), "edit-file".into()],
        stdin: Some(input.to_string().into_bytes()),
    }
    .into())
}

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
        return Err("old_string must not be empty; use write_file to create a file".into());
    }
    if edit.old_string == edit.new_string {
        return Err("old_string and new_string are the same; nothing to change".into());
    }
    let before = std::fs::read_to_string(&edit.path).map_err(|error| error.to_string())?;
    let count = before.matches(&edit.old_string).count();
    let after = match (count, edit.replace_all) {
        (0, _) => {
            return Err(format!(
                "old_string not found in {}; read the file again and copy the text exactly, without line numbers",
                edit.path
            ));
        }
        (1, _) | (_, true) => before.replace(&edit.old_string, &edit.new_string),
        (count, false) => {
            return Err(format!(
                "old_string occurs {count} times in {}; add context to make it unique or set replace_all",
                edit.path
            ));
        }
    };
    replace(std::path::Path::new(&edit.path), &after).map_err(|error| error.to_string())?;
    let noun = if count == 1 {
        "occurrence"
    } else {
        "occurrences"
    };
    Ok(format!(
        "Replaced {count} {noun} in {}\n{}",
        edit.path,
        diff(&before, &after)
    ))
}

fn replace(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let path = &std::fs::canonicalize(path)?;
    let permissions = std::fs::metadata(path)?.permissions();
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let part = path.with_file_name(format!(".{name}.cubebot-edit"));
    std::fs::write(&part, text)?;
    std::fs::set_permissions(&part, permissions)?;
    std::fs::rename(&part, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&part);
    })
}

const CONTEXT: usize = 3;
const SHOWN: usize = 80;

fn diff(before: &str, after: &str) -> String {
    let (old, new): (Vec<&str>, Vec<&str>) = (before.lines().collect(), after.lines().collect());
    let same = |a: &[&str], b: &[&str]| a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let head = same(&old, &new);
    let tail = same(
        &old[head..].iter().rev().copied().collect::<Vec<_>>(),
        &new[head..].iter().rev().copied().collect::<Vec<_>>(),
    );
    let start = head.saturating_sub(CONTEXT);
    let (old_end, new_end) = (old.len() - tail, new.len() - tail);
    let after_old = (old_end + CONTEXT).min(old.len());
    let after_new = (new_end + CONTEXT).min(new.len());
    let mut lines = vec![format!(
        "@@ -{},{} +{},{} @@",
        start + 1,
        after_old - start,
        start + 1,
        after_new - start
    )];
    lines.extend(old[start..head].iter().map(|line| format!(" {line}")));
    lines.extend(old[head..old_end].iter().map(|line| format!("-{line}")));
    lines.extend(new[head..new_end].iter().map(|line| format!("+{line}")));
    lines.extend(
        new[new_end..after_new]
            .iter()
            .map(|line| format!(" {line}")),
    );
    if lines.len() > SHOWN {
        let hidden = lines.len() - SHOWN;
        lines.truncate(SHOWN);
        lines.push(format!(
            "[{hidden} more diff lines; read the file to see them]"
        ));
    }
    lines.join("\n")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pattern {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
}
fn glob(input: &Value) -> Result<Planned, &'static str> {
    let Pattern { pattern, path } = parse(input)?;
    let pattern = format!("--glob={}", argument(&pattern)?);
    let mut argv: Vec<String> = RG.map(Into::into).into();
    argv.extend(["--files".into(), "--sortr=modified".into(), pattern]);
    searched(&mut argv, path.as_deref())?;
    Ok(Job { argv, stdin: None }.into())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    glob: Option<String>,
    #[serde(default)]
    ignore_case: bool,
}
fn grep(input: &Value) -> Result<Planned, &'static str> {
    let search: Search = parse(input)?;
    let mut argv: Vec<String> = RG.map(Into::into).into();
    argv.extend(["--line-number", "--no-heading", "--sort=path"].map(Into::into));
    if search.ignore_case {
        argv.push("--ignore-case".into());
    }
    if let Some(glob) = &search.glob {
        argv.push(format!("--glob={}", argument(glob)?));
    }
    argv.extend(["-e".into(), argument(&search.pattern)?.into()]);
    searched(&mut argv, search.path.as_deref())?;
    Ok(Job { argv, stdin: None }.into())
}

fn searched(argv: &mut Vec<String>, path: Option<&str>) -> Result<(), &'static str> {
    argv.push("--".into());
    if let Some(path) = path {
        argv.push(argument(path)?.into());
    }
    Ok(())
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
            bash.call(
                &sandbox,
                Uuid::nil(),
                &json!({"command":"true","timeout":null})
            )
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
        let read =
            |offset: Value, limit: Value| json!({"path":"src/a.txt","offset":offset,"limit":limit});
        let (code, output) = run(&sandbox, "read_file", read(Value::Null, Value::Null)).await;
        assert_eq!(code, Some(0));
        assert!(
            output.starts_with("     1\talpha\n     2\tbeta $(touch pwned)\n"),
            "{output}"
        );
        let (code, output) = run(&sandbox, "read_file", read(json!(2), json!(1))).await;
        assert_eq!(code, Some(0));
        assert!(output.starts_with("     2\tbeta"), "{output}");
        let (code, output) = run(&sandbox, "read_file", read(json!(1), json!(1))).await;
        assert_eq!(code, Some(0));
        assert!(
            output.contains("[lines 1-1 of 2; continue with offset 2]"),
            "{output}"
        );
        let (code, output) = run(
            &sandbox,
            "read_file",
            json!({"path":"src","offset":null,"limit":null}),
        )
        .await;
        assert_eq!(code, Some(1));
        assert!(output.contains("is a directory"), "{output}");
        assert!(!sandbox.workspace.join("pwned").exists());
        std::fs::remove_dir_all(&sandbox.workspace).unwrap();
    }

    fn ripgrep_on_the_tool_path() -> bool {
        ["/usr/local/bin/rg", "/usr/bin/rg", "/bin/rg"]
            .iter()
            .any(|path| std::path::Path::new(path).exists())
    }

    #[tokio::test]
    async fn searches_use_ripgrep_and_skip_what_gitignore_ignores() {
        if !ripgrep_on_the_tool_path() {
            eprintln!("ripgrep is not on the tool PATH; the agent image and CI install it");
            return;
        }
        let sandbox = super::super::tests::sandbox();
        std::fs::create_dir_all(sandbox.workspace.join("src")).unwrap();
        std::fs::create_dir_all(sandbox.workspace.join("target")).unwrap();
        std::fs::write(sandbox.workspace.join("src/a.txt"), "alpha\nbeta\n").unwrap();
        std::fs::write(sandbox.workspace.join("target/b.txt"), "beta\n").unwrap();
        std::fs::write(sandbox.workspace.join(".gitignore"), "target/\n").unwrap();
        let glob = |pattern: &str| json!({"pattern":pattern,"path":null});
        let (code, output) = run(&sandbox, "glob", glob("src/*.txt")).await;
        assert_eq!(code, Some(0));
        assert!(output.starts_with("src/a.txt\n"), "{output}");
        let (code, output) = run(&sandbox, "glob", glob("**/*.txt")).await;
        assert_eq!(code, Some(0));
        assert!(output.starts_with("src/a.txt\n"), "{output}");
        let (code, _) = run(&sandbox, "glob", glob("$(touch pwned)*")).await;
        assert_eq!(code, Some(1));
        let search = |pattern: &str, path: Value, glob: Value| json!({"pattern":pattern,"path":path,"glob":glob,"ignore_case":false});
        let (code, output) = run(&sandbox, "grep", search("^be+ta", json!("."), Value::Null)).await;
        assert_eq!(code, Some(0));
        assert!(output.starts_with("./src/a.txt:2:beta"), "{output}");
        let (code, output) = run(
            &sandbox,
            "grep",
            search("beta", Value::Null, json!("src/**")),
        )
        .await;
        assert_eq!(
            (code, output.lines().next()),
            (Some(0), Some("src/a.txt:2:beta"))
        );
        let (code, _) = run(&sandbox, "grep", search("gamma", Value::Null, Value::Null)).await;
        assert_eq!(code, Some(1));
        let (code, output) = run(&sandbox, "grep", search("BETA", Value::Null, Value::Null)).await;
        assert_eq!(code, Some(1), "{output}");
        let insensitive = json!({"pattern":"BETA","path":null,"glob":null,"ignore_case":true});
        let (code, output) = run(&sandbox, "grep", insensitive).await;
        assert_eq!(
            (code, output.lines().next()),
            (Some(0), Some("src/a.txt:2:beta"))
        );
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
        assert!(
            edit(&request("b", false))
                .unwrap()
                .ends_with("@@ -1,1 +1,1 @@\n-a b a\n+a c a")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a c a");
        edit(&request("a", true)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "c c c");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_diff_shows_the_changed_lines_with_three_lines_around_them() {
        let before = (1..=10).map(|n| format!("line {n}\n")).collect::<String>();
        let after = before.replace("line 5\n", "five\nand a half\n");
        assert_eq!(
            diff(&before, &after),
            "@@ -2,7 +2,8 @@\n line 2\n line 3\n line 4\n-line 5\n+five\n+and a half\n line 6\n line 7\n line 8"
        );
        assert_eq!(diff("a\n", "a\nb\n"), "@@ -1,1 +1,2 @@\n a\n+b");
    }

    #[cfg(unix)]
    #[test]
    fn an_edit_keeps_permissions_and_writes_through_a_symlink() {
        use std::os::unix::fs::PermissionsExt;
        let folder = std::env::temp_dir().join(format!("cubebot-edit-{}", Uuid::new_v4()));
        std::fs::create_dir(&folder).unwrap();
        let (script, link) = (folder.join("run.sh"), folder.join("link.sh"));
        std::fs::write(&script, "echo old\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o750)).unwrap();
        std::os::unix::fs::symlink(&script, &link).unwrap();
        let request =
            json!({"path":link,"old_string":"old","new_string":"new","replace_all":false});
        edit(&request.to_string()).unwrap();
        assert_eq!(std::fs::read_to_string(&script).unwrap(), "echo new\n");
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let mode = std::fs::metadata(&script).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o750);
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 2);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_plan_names_its_progress_and_keeps_one_step_in_progress() {
        let plan = |todos: Value| planned(parse(&json!({ "todos": todos })).unwrap());
        let step = |content: &str, status: &str| json!({"content":content,"status":status});
        assert_eq!(
            plan(json!([
                step("Read the code", "completed"),
                step("Fix it", "in_progress"),
                step("Test it", "pending")
            ])),
            Ok("Plan updated: 1 of 3 done. Now: Fix it".into())
        );
        assert_eq!(
            plan(json!([step("Ship", "completed")])),
            Ok("Plan complete: 1 of 1 done.".into())
        );
        assert!(plan(json!([step("A", "in_progress"), step("B", "in_progress")])).is_err());
        assert!(plan(json!([step(" ", "pending")])).is_err());
    }

    #[tokio::test]
    async fn a_command_stops_at_its_own_timeout() {
        let sandbox = super::super::tests::sandbox();
        let tools = tools();
        let bash = tools
            .iter()
            .find(|tool| tool.definition().name == "bash")
            .unwrap();
        assert!(
            bash.call(
                &sandbox,
                Uuid::nil(),
                &json!({"command":"true","timeout":0})
            )
            .is_err()
        );
        assert!(
            bash.call(
                &sandbox,
                Uuid::nil(),
                &json!({"command":"true","timeout":3601})
            )
            .is_err()
        );
        let running = bash
            .call(
                &sandbox,
                Uuid::new_v4(),
                &json!({"command":"echo started; sleep 30","timeout":1}),
            )
            .unwrap();
        match running.await {
            Outcome::Stopped { reason, output } => {
                assert_eq!(reason, "timed out");
                assert!(output.starts_with("started"), "{output}");
            }
            other => panic!("unexpected outcome {other:?}"),
        }
        std::fs::remove_dir_all(&sandbox.workspace).unwrap();
    }
}

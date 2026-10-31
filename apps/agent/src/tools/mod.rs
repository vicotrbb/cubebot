//! The agent's tools. Model arguments are data; every tool runs as a child process of the agent
//! under the sandbox's unprivileged tool user, so the kernel, not path checks, bounds what a call
//! can touch. Full output stays in the workspace and the model sees a head-and-tail preview.
mod browser;
mod builtin;
mod computer;
mod mcp;

pub use builtin::edit_file;
use cubebot_protocol::terminal::Outcome;
use serde_json::Value;
use std::{
    io::{self, Read, Seek, SeekFrom},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::PathBuf,
    pin::Pin,
    process::Stdio,
};
use tokio::{io::AsyncWriteExt, process::Command};
use uuid::Uuid;

/// Output shown from each end of a longer result.
const HEAD: u64 = 8 * 1024;
const TAIL: u64 = 8 * 1024;
const OUTPUT: &str = ".cubebot-tool-output";

/// A started call. Dropping it stops the call and its process group.
pub type Running<'a> = Pin<Box<dyn Future<Output = Outcome> + Send + 'a>>;

pub trait Tool: Send + Sync {
    fn definition(&self) -> &cubebot_providers::Tool;
    /// Validates model arguments before the call is journaled; the call starts when polled.
    fn call<'a>(
        &'a self,
        sandbox: &'a Sandbox,
        id: Uuid,
        input: &Value,
    ) -> Result<Running<'a>, &'static str>;
}

/// Program arguments and optional standard input for one call.
pub struct Job {
    pub argv: Vec<String>,
    pub stdin: Option<Vec<u8>>,
}

pub struct Sandbox {
    workspace: PathBuf,
    uid: u32,
    gid: u32,
}

impl Sandbox {
    /// `CUBEBOT_TOOL_USER` is `uid:gid` and never root, so tools cannot reach agent state.
    pub fn from_environment() -> Result<Self, &'static str> {
        let workspace = std::env::var_os("CUBEBOT_WORKSPACE_DIRECTORY")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or("tool workspace not configured")?;
        let user = std::env::var("CUBEBOT_TOOL_USER").map_err(|_| "tool user not configured")?;
        let (uid, gid) = user
            .split_once(':')
            .and_then(|(uid, gid)| Some((uid.parse().ok()?, gid.parse().ok()?)))
            .filter(|&(uid, gid)| uid != 0 && gid != 0)
            .ok_or("tool user must be a non-root uid:gid")?;
        Ok(Self {
            workspace,
            uid,
            gid,
        })
    }

    /// A child as the tool user in its own process group, in the workspace, without the agent's
    /// environment. Standard library spawning drops supplementary groups when the agent is root.
    pub fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .env("HOME", &self.workspace)
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("LANG", "C.UTF-8")
            .current_dir(&self.workspace)
            .uid(self.uid)
            .gid(self.gid)
            .process_group(0)
            .kill_on_drop(true);
        command
    }

    fn output(&self, id: Uuid) -> PathBuf {
        self.workspace.join(OUTPUT).join(format!("{id}.log"))
    }

    /// Runs a job with its combined output written by the tool user to the workspace.
    pub async fn run(&self, id: Uuid, job: Job) -> Outcome {
        let mut command = self.command("/bin/sh");
        command
            .args([
                "-c",
                r#"mkdir -p -- "$1" && out="$1/$2.log" && shift 2 && exec "$@" > "$out" 2>&1"#,
            ])
            .args(["tool", OUTPUT, &id.to_string()])
            .args(&job.argv)
            .stdin(if job.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let code = match self.wait(command, job.stdin).await {
            Ok(code) => code,
            Err(error) => {
                return Outcome::Exited {
                    code: None,
                    output: format!("The tool did not start: {error}"),
                };
            }
        };
        Outcome::Exited {
            code,
            output: self.preview(id),
        }
    }

    async fn wait(&self, mut command: Command, stdin: Option<Vec<u8>>) -> io::Result<Option<i32>> {
        let mut child = command.spawn()?;
        let _group = Group(child.id());
        if let (Some(mut pipe), Some(bytes)) = (child.stdin.take(), stdin) {
            // A program that exits without reading its input is not a failed start.
            let _ = pipe.write_all(&bytes).await;
        }
        Ok(child.wait().await?.code())
    }

    /// The shared truncation policy: the whole result when short, otherwise its head and tail
    /// with the omitted size, always followed by the retained file.
    pub fn preview(&self, id: Uuid) -> String {
        let path = self.output(id);
        let shown = self.read_preview(&path).unwrap_or_else(|_| {
            "Output preview unavailable; inspect the retained workspace file.".into()
        });
        format!("{shown}\n\nFull output: {}", path.display())
    }

    /// Opens only a regular file the tool user owns, so a planted link or hard link cannot make
    /// the agent read its own state.
    fn read_preview(&self, path: &std::path::Path) -> io::Result<String> {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.uid() != self.uid {
            return Err(io::Error::other("output file not owned by the tool user"));
        }
        let length = metadata.len();
        if length <= HEAD + TAIL {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            return Ok(String::from_utf8_lossy(&bytes).into_owned());
        }
        let mut head = vec![0; HEAD as usize];
        file.read_exact(&mut head)?;
        file.seek(SeekFrom::End(-(TAIL as i64)))?;
        let mut tail = Vec::new();
        file.read_to_end(&mut tail)?;
        Ok(format!(
            "{}\n[... {} bytes omitted ...]\n{}",
            String::from_utf8_lossy(&head),
            length - HEAD - TAIL,
            String::from_utf8_lossy(&tail)
        ))
    }
}

/// Stops a call's whole process group, including background jobs, when the call ends or is
/// cancelled. shortcut: a descendant that calls setsid survives until the sandbox stops; kill
/// the tool user's processes once the kvm guest owns the session.
struct Group(Option<u32>);
impl Drop for Group {
    fn drop(&mut self) {
        if let Some(id) = self.0 {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(id as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }
}

/// Built-in tools followed by the tools of each configured MCP server.
pub struct Registry {
    sandbox: Sandbox,
    tools: Vec<Box<dyn Tool>>,
    definitions: Vec<cubebot_providers::Tool>,
}

impl Registry {
    /// `None` when tools are disabled. An enabled but invalid configuration is an error, never
    /// a fallback to running tools as the agent.
    pub async fn load() -> Result<Option<Self>, &'static str> {
        if std::env::var("CUBEBOT_TERMINAL_ENABLED").as_deref() != Ok("1") {
            return Ok(None);
        }
        let sandbox = Sandbox::from_environment()?;
        let mut tools = builtin::tools();
        // The provider's computer-use tool, for models that support it (ADR 0085), and the
        // browser tools on the desktop's Chromium (ADRs 0036 and 0037).
        if std::env::var("CUBEBOT_COMPUTER_ENABLED").as_deref() == Ok("1") {
            tools.push(computer::tool());
            tools.extend(browser::tools());
        }
        tools.extend(mcp::tools(&sandbox).await?);
        Ok(Some(Self::new(sandbox, tools)))
    }

    fn new(sandbox: Sandbox, tools: Vec<Box<dyn Tool>>) -> Self {
        let definitions = tools.iter().map(|tool| tool.definition().clone()).collect();
        Self {
            sandbox,
            tools,
            definitions,
        }
    }

    pub fn definitions(&self) -> &[cubebot_providers::Tool] {
        &self.definitions
    }

    pub fn call(&self, name: &str, id: Uuid, input: &Value) -> Result<Running<'_>, &'static str> {
        self.tools
            .iter()
            .find(|tool| tool.definition().name == name)
            .ok_or("no tool has this name")?
            .call(&self.sandbox, id, input)
    }

    /// Writes a workspace file as the tool user, creating parent directories, so the agent's
    /// own writes have the same reach as a tool call's.
    pub async fn write(&self, path: &str, content: &str) -> bool {
        let job = builtin::write(path, content.as_bytes().to_vec());
        let mut command = self.sandbox.command(&job.argv[0]);
        command
            .args(&job.argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        matches!(self.sandbox.wait(command, job.stdin).await, Ok(Some(0)))
    }

    /// What a stopped call left in its retained output.
    pub fn preview(&self, id: Uuid) -> String {
        self.sandbox.preview(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sandbox whose tool user is the test's own user, as on the development backend.
    pub(super) fn sandbox() -> Sandbox {
        let workspace = std::env::temp_dir().join(format!("cubebot-tools-{}", Uuid::new_v4()));
        std::fs::create_dir(&workspace).unwrap();
        let owner = std::fs::metadata(&workspace).unwrap();
        Sandbox {
            workspace,
            uid: owner.uid(),
            gid: owner.gid(),
        }
    }

    fn output(outcome: Outcome) -> (Option<i32>, String) {
        match outcome {
            Outcome::Exited { code, output } => (code, output),
            other => panic!("unexpected outcome {other:?}"),
        }
    }

    #[tokio::test]
    async fn long_output_keeps_head_and_tail_and_the_whole_file_in_the_workspace() {
        let sandbox = sandbox();
        let id = Uuid::new_v4();
        let script = "printf start; head -c 100000 /dev/zero | tr '\\0' x; printf end; exit 3";
        let (code, shown) = output(sandbox.run(id, builtin::shell(script, &[])).await);
        assert_eq!(code, Some(3));
        assert!(shown.starts_with("startxx"), "{shown}");
        assert!(shown.contains("[... 83624 bytes omitted ...]"), "{shown}");
        assert!(shown.contains("xxend\n\nFull output: "), "{shown}");
        assert_eq!(std::fs::metadata(sandbox.output(id)).unwrap().len(), 100008);
        std::fs::remove_dir_all(&sandbox.workspace).unwrap();
    }

    #[tokio::test]
    async fn commands_run_in_the_workspace_without_the_agent_environment() {
        let sandbox = sandbox();
        let job = builtin::shell(
            "pwd; printf '%s' \"${CUBEBOT_JOURNAL_DIRECTORY:-clean}\"",
            &[],
        );
        let (code, shown) = output(sandbox.run(Uuid::new_v4(), job).await);
        assert_eq!(code, Some(0));
        assert!(
            shown.contains("clean") && shown.contains("cubebot-tools-"),
            "{shown}"
        );
        std::fs::remove_dir_all(&sandbox.workspace).unwrap();
    }

    #[test]
    fn previews_refuse_links_planted_in_the_output_directory() {
        let sandbox = sandbox();
        let id = Uuid::new_v4();
        std::fs::create_dir(sandbox.workspace.join(OUTPUT)).unwrap();
        std::os::unix::fs::symlink("/etc/hosts", sandbox.output(id)).unwrap();
        assert!(
            sandbox
                .preview(id)
                .starts_with("Output preview unavailable")
        );
        std::fs::remove_dir_all(&sandbox.workspace).unwrap();
    }

    #[tokio::test]
    async fn the_registry_writes_files_as_the_tool_user() {
        let registry = Registry::new(sandbox(), Vec::new());
        let path = ".cubebot-skills/disk-triage/1/scripts/usage.sh";
        assert!(registry.write(path, "du -sh *").await);
        let workspace = &registry.sandbox.workspace;
        let written = std::fs::read_to_string(workspace.join(path)).unwrap();
        assert_eq!(written, "du -sh *");
        std::fs::remove_dir_all(workspace).unwrap();
    }

    #[tokio::test]
    async fn dropping_a_call_kills_its_process_group() {
        let sandbox = sandbox();
        let marker = sandbox.workspace.join("survived");
        let job = builtin::shell("(sleep 1; touch survived) & sleep 30", &[]);
        let call = sandbox.run(Uuid::new_v4(), job);
        let _ = tokio::time::timeout(std::time::Duration::from_millis(200), call).await;
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        assert!(!marker.exists());
        std::fs::remove_dir_all(&sandbox.workspace).unwrap();
    }
}

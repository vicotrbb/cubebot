//! The agent's tools. Model arguments are data; every tool runs as a child process of the agent
//! under the sandbox's unprivileged tool user, so the kernel, not path checks, bounds what a call
//! can touch. Full output stays in the workspace and the model sees a head-and-tail preview.
mod browser;
mod builtin;
mod computer;
mod mcp;

pub use builtin::edit_file;
pub(crate) use builtin::parse;
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

/// Callers waiting for a reply by request id, shared with the task that routes replies.
type Waiting = std::sync::Arc<
    std::sync::Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<Value>>>,
>;

/// The waiting callers. A panic while one held the map leaves it consistent: every change is
/// a single insert, remove or clear.
fn lock(
    waiting: &Waiting,
) -> std::sync::MutexGuard<'_, std::collections::HashMap<u64, tokio::sync::oneshot::Sender<Value>>>
{
    waiting
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The tool user's search path: its own `~/.local/bin` first, then the system's.
pub(crate) fn user_path(home: &std::path::Path) -> String {
    format!("{}/.local/bin:/usr/local/bin:/usr/bin:/bin", home.display())
}

/// A `uid:gid` tool user, never root.
pub(crate) fn tool_user(user: &str) -> Option<(u32, u32)> {
    user.split_once(':')
        .and_then(|(uid, gid)| Some((uid.parse().ok()?, gid.parse().ok()?)))
        .filter(|&(uid, gid)| uid != 0 && gid != 0)
}

/// Output shown from each end of a longer result.
const HEAD: u64 = 8 * 1024;
const TAIL: u64 = 8 * 1024;
const OUTPUT: &str = ".cubebot-tool-output";
/// The guest image's default desktop wallpaper.
pub const WALLPAPER: &str = "/usr/share/cubebot/wallpaper.jpg";
/// Guest init creates this once it has drawn the default, in `/run`, which only root writes.
pub const DESKTOP_READY: &str = "/run/cubebot-desktop";

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
    /// The desktop is on, so programs a tool starts open their windows on it.
    desktop: bool,
}

impl Sandbox {
    /// `CUBEBOT_TOOL_USER` is `uid:gid` and never root, so tools cannot reach agent state.
    pub fn from_environment() -> Result<Self, crate::Caused> {
        let workspace = std::env::var_os("CUBEBOT_WORKSPACE_DIRECTORY")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or("tool workspace not configured")?;
        let user = std::env::var("CUBEBOT_TOOL_USER")
            .map_err(|error| crate::Caused::new("tool user not configured", error))?;
        let (uid, gid) = tool_user(&user).ok_or("tool user must be a non-root uid:gid")?;
        Ok(Self {
            workspace,
            uid,
            gid,
            desktop: std::env::var("CUBEBOT_COMPUTER_ENABLED").as_deref() == Ok("1"),
        })
    }

    /// A child as the tool user in its own process group, in the workspace, without the agent's
    /// environment. Standard library spawning drops supplementary groups when the agent is root.
    pub fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .env("HOME", &self.workspace)
            // Software the tool user installs with pip, npm or by hand lands in ~/.local/bin.
            .env("PATH", user_path(&self.workspace))
            .env("LANG", "C.UTF-8")
            .current_dir(&self.workspace)
            .uid(self.uid)
            .gid(self.gid)
            .process_group(0)
            .kill_on_drop(true);
        // `:1` is the tool user's display (docs/adr/sandbox.md#desktop-as-a-vmm-device).
        if self.desktop {
            command.env("DISPLAY", ":1");
        }
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
    /// The MCP servers the last turn named and their tools, swapped whole when that changes.
    mcp: std::sync::RwLock<Mcp>,
    /// One reconnection at a time, so concurrent lanes never start a server twice.
    connecting: tokio::sync::Mutex<()>,
    /// Lanes run at once but share one mouse and screen, so a turn holds this while it uses them.
    desktop: std::sync::Arc<tokio::sync::Mutex<()>>,
}

#[derive(Default)]
struct Mcp {
    servers: Vec<cubebot_protocol::runtime::McpServer>,
    tools: Vec<std::sync::Arc<dyn Tool>>,
    /// When a server last failed to connect; it is tried again after [`MCP_RETRY`].
    failed: Option<std::time::Instant>,
}

/// How long a turn goes without retrying an MCP server that failed to connect, so an
/// unreachable server does not hold up every turn for its connection timeout.
const MCP_RETRY: std::time::Duration = std::time::Duration::from_secs(300);

impl Registry {
    /// `None` when tools are disabled. An enabled but invalid configuration is an error, never
    /// a fallback to running tools as the agent.
    pub async fn load() -> Result<Option<Self>, crate::Caused> {
        if std::env::var("CUBEBOT_TERMINAL_ENABLED").as_deref() != Ok("1") {
            return Ok(None);
        }
        let sandbox = Sandbox::from_environment()?;
        let mut tools = builtin::tools();
        // The provider's computer-use tool, for models that support it (docs/adr/sandbox.md#desktop-as-a-vmm-device), and the
        // browser tools on the desktop's Chromium (docs/adr/sandbox.md#structured-browser-tools-with-visual-interaction and docs/adr/sandbox.md#main-browser-session-with-separate-task-sessions).
        if std::env::var("CUBEBOT_COMPUTER_ENABLED").as_deref() == Ok("1") {
            tools.push(computer::tool());
            tools.extend(browser::tools());
        }
        Ok(Some(Self::new(sandbox, tools)))
    }

    fn new(sandbox: Sandbox, tools: Vec<Box<dyn Tool>>) -> Self {
        let definitions = tools.iter().map(|tool| tool.definition().clone()).collect();
        Self {
            sandbox,
            tools,
            definitions,
            mcp: std::sync::RwLock::default(),
            connecting: tokio::sync::Mutex::new(()),
            desktop: std::sync::Arc::default(),
        }
    }

    fn mcp(&self) -> std::sync::RwLockReadGuard<'_, Mcp> {
        self.mcp
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Connects the MCP servers a turn names when they differ from the connected ones, or
    /// when one failed and its retry delay has passed. Servers dropped from the list stop.
    pub async fn connect_mcp(&self, servers: &[cubebot_protocol::runtime::McpServer]) {
        let _one = self.connecting.lock().await;
        let current = |mcp: &Mcp| {
            mcp.servers == servers && mcp.failed.is_none_or(|at| at.elapsed() < MCP_RETRY)
        };
        if current(&self.mcp()) {
            return;
        }
        let (tools, all) = mcp::tools(&self.sandbox, servers.to_vec())
            .await
            .unwrap_or_else(|error| {
                eprintln!("MCP servers refused: {error}");
                (Vec::new(), false)
            });
        *self
            .mcp
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Mcp {
            servers: servers.to_vec(),
            tools: tools.into_iter().map(std::sync::Arc::from).collect(),
            failed: (!all).then(std::time::Instant::now),
        };
    }

    /// Built-in tools, then the connected MCP servers' tools.
    pub fn definitions(&self) -> Vec<cubebot_providers::Tool> {
        let mcp = self.mcp();
        let servers = mcp.tools.iter().map(|tool| tool.definition().clone());
        self.definitions.iter().cloned().chain(servers).collect()
    }

    pub fn call(&self, name: &str, id: Uuid, input: &Value) -> Result<Running<'_>, &'static str> {
        if let Some(tool) = self
            .tools
            .iter()
            .find(|tool| tool.definition().name == name)
        {
            return tool.call(&self.sandbox, id, input);
        }
        let tool = self
            .mcp()
            .tools
            .iter()
            .find(|tool| tool.definition().name == name)
            .cloned()
            .ok_or("no tool has this name")?;
        // Calls start when polled, so validating here runs nothing; the future then owns its
        // tool, which a reconnection may drop from the registry while the call runs.
        drop(tool.call(&self.sandbox, id, input)?);
        let input = input.clone();
        Ok(Box::pin(async move {
            match tool.call(&self.sandbox, id, &input) {
                Ok(running) => running.await,
                Err(error) => Outcome::Exited {
                    code: None,
                    output: error.into(),
                },
            }
        }))
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

    /// A workspace file as the tool user reads it, on the child's standard output; its exit
    /// status says whether it was read whole.
    pub fn reader(&self, path: &str) -> io::Result<tokio::process::Child> {
        let mut command = self.sandbox.command("cat");
        command
            .args(["--", path])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command.spawn()
    }

    /// Writes its standard input to a workspace file as the tool user, creating parent
    /// directories, and replaces the file only once the input has ended. Kill it to abandon.
    pub fn writer(&self, path: &str) -> io::Result<tokio::process::Child> {
        const REPLACE: &str = r#"mkdir -p -- "$(dirname -- "$1")" && cat > "$1.cubebot-part" && mv -f -- "$1.cubebot-part" "$1""#;
        let mut command = self.sandbox.command("/bin/sh");
        command
            .args(["-c", REPLACE, "tool", path])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.spawn()
    }

    /// Draws `image` on the desktop as the tool user, or the default with `None`. True when
    /// there is no desktop to draw on.
    pub async fn wallpaper(&self, image: Option<Vec<u8>>) -> bool {
        if std::env::var("CUBEBOT_COMPUTER_ENABLED").as_deref() != Ok("1") {
            return true;
        }
        // Waits for guest init's default, so it never draws over this one.
        const DRAW: &str = r#"f=$(mktemp) && cat > "$f" || exit 1
for _ in $(seq 60); do [ -e "$2" ] && break; sleep 0.5; done
xwallpaper --zoom "${1:-$f}"; s=$?; rm -f "$f"; exit $s"#;
        let default = if image.is_some() { "" } else { WALLPAPER };
        let mut command = self.sandbox.command("/bin/sh");
        command
            .args(["-c", DRAW, "wallpaper", default, DESKTOP_READY])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let image = image.unwrap_or_default();
        matches!(self.sandbox.wait(command, Some(image)).await, Ok(Some(0)))
    }

    /// Waits until no other turn is using the desktop.
    pub async fn desktop(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.desktop.clone().lock_owned().await
    }

    #[cfg(test)]
    pub(crate) fn with_desktop() -> Self {
        Self::new(tests::sandbox(), vec![computer::tool()])
    }

    pub fn computer(&self, name: &str) -> bool {
        self.definitions
            .iter()
            .any(|tool| tool.computer && tool.name == name)
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
            desktop: false,
        }
    }

    #[tokio::test]
    async fn tool_programs_reach_the_desktop_only_when_it_is_on() {
        let mut sandbox = sandbox();
        let display = |sandbox: &Sandbox| {
            let mut command = sandbox.command("/bin/sh");
            command.args(["-c", "printf %s \"$DISPLAY\""]);
            async move { command.output().await.unwrap().stdout }
        };
        assert_eq!(display(&sandbox).await, b"");
        sandbox.desktop = true;
        assert_eq!(display(&sandbox).await, b":1");
    }

    #[tokio::test]
    async fn the_desktop_is_one_turn_at_a_time() {
        let registry = Registry::with_desktop();
        let held = registry.desktop().await;
        let waiting = registry.desktop();
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut waiting)
                .await
                .is_err(),
            "a second turn waits while the first holds it"
        );
        drop(held);
        waiting.await;
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

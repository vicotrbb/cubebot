//! Every tool runs as a child under the sandbox's unprivileged tool user, so the kernel, not path checks, bounds what a call can touch.
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

type Waiting = std::sync::Arc<
    std::sync::Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<Value>>>,
>;

/// A panic while one held the map leaves it consistent: every change is a single insert, remove or clear.
fn lock(
    waiting: &Waiting,
) -> std::sync::MutexGuard<'_, std::collections::HashMap<u64, tokio::sync::oneshot::Sender<Value>>>
{
    waiting
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn user_path(home: &std::path::Path) -> String {
    format!("{}/.local/bin:/usr/local/bin:/usr/bin:/bin", home.display())
}

pub(crate) fn tool_user(user: &str) -> Option<(u32, u32)> {
    user.split_once(':')
        .and_then(|(uid, gid)| Some((uid.parse().ok()?, gid.parse().ok()?)))
        .filter(|&(uid, gid)| uid != 0 && gid != 0)
}

const HEAD: u64 = 8 * 1024;
const TAIL: u64 = 8 * 1024;
const OUTPUT: &str = ".cubebot-tool-output";
const OUTPUT_MINUTES: &str = "+10080";
const CALL_LIMIT: std::time::Duration = std::time::Duration::from_secs(60 * 60);
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

pub struct Job {
    pub argv: Vec<String>,
    pub stdin: Option<Vec<u8>>,
}

pub struct Sandbox {
    workspace: PathBuf,
    uid: u32,
    gid: u32,
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

    /// Standard library spawning drops supplementary groups when the agent is root.
    pub fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .env("HOME", &self.workspace)
            .env("PATH", user_path(&self.workspace))
            .env("LANG", "C.UTF-8")
            .current_dir(&self.workspace)
            .uid(self.uid)
            .gid(self.gid)
            .process_group(0)
            .kill_on_drop(true);
        if self.desktop {
            command.env("DISPLAY", ":1");
        }
        command
    }

    fn output(&self, id: Uuid) -> PathBuf {
        self.workspace.join(OUTPUT).join(format!("{id}.log"))
    }

    async fn prune_output(&self) {
        let mut command = self.command("find");
        command
            .args([OUTPUT, "-maxdepth", "1", "-type", "f", "-name", "*.log"])
            .args(["-mmin", OUTPUT_MINUTES, "-delete"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Err(error) = command.status().await {
            eprintln!("tool output not pruned: {error}");
        }
    }

    pub async fn run(&self, id: Uuid, job: Job) -> Outcome {
        self.run_within(CALL_LIMIT, id, job).await
    }

    async fn run_within(&self, limit: std::time::Duration, id: Uuid, job: Job) -> Outcome {
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
        // Dropping the wait kills the process group before the output is read.
        let code = match tokio::time::timeout(limit, self.wait(command, job.stdin)).await {
            Err(_) => {
                return Outcome::Stopped {
                    reason: "timed out".into(),
                    output: self.preview(id),
                };
            }
            Ok(Ok(code)) => code,
            Ok(Err(error)) => {
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

    /// Keeps a result in the call's output file, as a program's output would be, and answers with
    /// its preview: `code` 1 marks a failure.
    pub async fn retain(&self, id: Uuid, text: String, code: i32) -> Outcome {
        let job = Job {
            argv: vec!["cat".into()],
            stdin: Some(text.into_bytes()),
        };
        match self.run(id, job).await {
            Outcome::Exited {
                code: Some(0),
                output,
            } => Outcome::Exited {
                code: Some(code),
                output,
            },
            other => other,
        }
    }

    pub fn preview(&self, id: Uuid) -> String {
        let path = self.output(id);
        let shown = self.read_preview(&path).unwrap_or_else(|_| {
            "Output preview unavailable; inspect the retained workspace file.".into()
        });
        format!("{shown}\n\nFull output: {}", path.display())
    }

    /// Opens only a regular file the tool user owns, so a planted link or hard link cannot make the agent read its own state.
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

/// shortcut: a descendant that calls setsid survives until the sandbox stops; kill the tool user's processes once the kvm guest owns the session.
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

pub struct Registry {
    sandbox: Sandbox,
    tools: Vec<Box<dyn Tool>>,
    definitions: Vec<cubebot_providers::Tool>,
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
    failed: Option<std::time::Instant>,
}

/// So an unreachable server does not hold up every turn for its connection timeout.
const MCP_RETRY: std::time::Duration = std::time::Duration::from_secs(300);

impl Registry {
    /// An enabled but invalid configuration is an error, never a fallback to running tools as the agent.
    pub async fn load() -> Result<Option<Self>, crate::Caused> {
        if std::env::var("CUBEBOT_TERMINAL_ENABLED").as_deref() != Ok("1") {
            return Ok(None);
        }
        let sandbox = Sandbox::from_environment()?;
        sandbox.prune_output().await;
        let mut tools = builtin::tools();
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
        // Calls start when polled, so validating here runs nothing; the future owns its tool, which a reconnection may drop while it runs.
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

    /// As the tool user, so the agent's own writes have the same reach as a tool call's.
    pub async fn write(&self, path: &str, content: &str) -> bool {
        self.write_bytes(path, content.as_bytes().to_vec()).await
    }

    pub async fn write_bytes(&self, path: &str, content: Vec<u8>) -> bool {
        let job = builtin::write(path, content);
        let mut command = self.sandbox.command(&job.argv[0]);
        command
            .args(&job.argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        matches!(self.sandbox.wait(command, job.stdin).await, Ok(Some(0)))
    }

    pub fn reader(&self, path: &str) -> io::Result<tokio::process::Child> {
        let mut command = self.sandbox.command("cat");
        command
            .args(["--", path])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command.spawn()
    }

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

    pub fn preview(&self, id: Uuid) -> String {
        self.sandbox.preview(id)
    }

    pub async fn retain(&self, id: Uuid, text: String, code: i32) -> Outcome {
        self.sandbox.retain(id, text, code).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    async fn a_call_past_its_limit_stops_with_its_output_and_process_group() {
        let sandbox = sandbox();
        let id = Uuid::new_v4();
        let script = "echo started; sleep 30 & echo $! > sleeper; wait";
        let job = Job {
            argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
            stdin: None,
        };
        let limit = std::time::Duration::from_millis(500);
        let outcome = sandbox.run_within(limit, id, job).await;
        assert!(outcome.validate().is_ok());
        let Outcome::Stopped { reason, output } = outcome else {
            panic!("unexpected outcome {outcome:?}");
        };
        assert_eq!(reason, "timed out");
        assert!(output.contains("started"), "{output}");
        let sleeper = std::fs::read_to_string(sandbox.workspace.join("sleeper")).unwrap();
        let sleeper = nix::unistd::Pid::from_raw(sleeper.trim().parse().unwrap());
        let gone = || nix::sys::signal::kill(sleeper, None).is_err();
        for _ in 0..200 {
            if gone() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(gone());
        std::fs::remove_dir_all(&sandbox.workspace).unwrap();
    }

    #[tokio::test]
    async fn pruning_deletes_only_old_output() {
        let sandbox = sandbox();
        let directory = sandbox.workspace.join(OUTPUT);
        std::fs::create_dir(&directory).unwrap();
        let now = std::time::SystemTime::now();
        for (name, days) in [("old.log", 8), ("recent.log", 6), ("old.keep", 8)] {
            let file = std::fs::File::create(directory.join(name)).unwrap();
            file.set_modified(now - std::time::Duration::from_secs(days * 86400))
                .unwrap();
        }
        sandbox.prune_output().await;
        assert!(!directory.join("old.log").exists());
        assert!(directory.join("recent.log").exists());
        assert!(directory.join("old.keep").exists());
        std::fs::remove_dir_all(&sandbox.workspace).unwrap();
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

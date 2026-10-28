//! Local servers: newline-delimited JSON-RPC over the pipes of a child run as the tool user.
use super::{Failure, answer, result};
use crate::tools::Sandbox;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    process::Stdio as Pipe,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
    sync::oneshot,
};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

pub(in crate::tools) struct Stdio {
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    pending: Pending,
    /// Set once the server's output ends; the next call starts it again.
    closed: Arc<AtomicBool>,
    _child: Child,
}

impl Stdio {
    pub fn spawn(
        sandbox: &Sandbox,
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
    ) -> Result<Self, String> {
        let mut child = sandbox
            .command(command)
            .args(args)
            .envs(env)
            .stdin(Pipe::piped())
            .stdout(Pipe::piped())
            .stderr(Pipe::inherit())
            .spawn()
            .map_err(|error| format!("did not start: {error}"))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err("pipes unavailable".into());
        };
        let stdin = Arc::new(tokio::sync::Mutex::new(stdin));
        let pending = Pending::default();
        let closed = Arc::new(AtomicBool::new(false));
        tokio::spawn(read(stdout, stdin.clone(), pending.clone(), closed.clone()));
        Ok(Self {
            stdin,
            pending,
            closed,
            _child: child,
        })
    }

    pub fn alive(&self) -> bool {
        !self.closed.load(Ordering::Acquire)
    }

    /// Stops waiting for a cancelled request's response.
    pub fn forget(&self, id: u64) {
        lock(&self.pending).remove(&id);
    }

    pub async fn request(&self, id: u64, method: &str, params: Value) -> Result<Value, Failure> {
        let (sender, response) = oneshot::channel();
        lock(&self.pending).insert(id, sender);
        let message = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        if let Err(failure) = self.send(&message).await {
            lock(&self.pending).remove(&id);
            return Err(failure);
        }
        result(response.await.map_err(|_| Failure::Lost)?)
    }

    pub async fn send(&self, message: &Value) -> Result<(), Failure> {
        write(&self.stdin, message)
            .await
            .map_err(|error| Failure::NotSent(error.to_string()))
    }
}

fn lock(pending: &Pending) -> std::sync::MutexGuard<'_, HashMap<u64, oneshot::Sender<Value>>> {
    pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn write(stdin: &tokio::sync::Mutex<ChildStdin>, message: &Value) -> std::io::Result<()> {
    let mut line = message.to_string();
    line.push('\n');
    stdin.lock().await.write_all(line.as_bytes()).await
}

/// Routes responses to their callers and answers server requests. When the server exits, every
/// waiting caller learns its result is lost.
async fn read(
    stdout: ChildStdout,
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    pending: Pending,
    closed: Arc<AtomicBool>,
) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = &message["id"];
        if message.get("method").is_some() {
            if !id.is_null() {
                let _ = write(&stdin, &answer(&message)).await;
            }
        } else if let Some(sender) = id.as_u64().and_then(|id| lock(&pending).remove(&id)) {
            let _ = sender.send(message);
        }
    }
    closed.store(true, Ordering::Release);
    lock(&pending).clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use cubebot_protocol::terminal::Outcome;
    use std::time::Duration;

    /// A line-oriented fake server: answers initialize, lists one tool and echoes calls.
    const SERVER: &str = r#"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"initialize"'*) printf '{"jsonrpc":"2.0","method":"ping","id":"s1"}\n{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18"}}\n' "$id" ;;
    *'"tools/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"Echo","inputSchema":{"type":"object"}}]}}\n' "$id" ;;
    *'"tools/call"'*) (sleep 0.2; printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"echoed %s"}]}}\n' "$id" "$id") & ;;
  esac
done"#;

    /// Counts its starts, exits after answering a quick call, never answers a slow one and
    /// records cancellations.
    const ONE_SHOT: &str = r#"
echo >> starts
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-11-25"}}\n' "$id" ;;
    *'"tools/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"run","inputSchema":{"type":"object"}}]}}\n' "$id" ;;
    *'notifications/cancelled'*) touch cancelled ;;
    *'"slow"'*) ;;
    *'"tools/call"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[]}}\n' "$id"; exit ;;
  esac
done"#;

    #[tokio::test]
    async fn a_stopped_server_starts_again_and_cancelled_calls_are_announced() {
        let sandbox = crate::tools::tests::sandbox();
        let config = super::super::Config::Stdio {
            name: "once".into(),
            command: "/bin/bash".into(),
            args: vec!["-c".into(), ONE_SHOT.into()],
            env: BTreeMap::new(),
        };
        let tools = super::super::connect(&sandbox, config).await.unwrap();
        let id = uuid::Uuid::new_v4();
        let quick = tools[0].call(&sandbox, id, &json!({})).unwrap().await;
        assert!(
            matches!(quick, Outcome::Exited { code: Some(0), .. }),
            "{quick:?}"
        );
        // Wait for the exit to be noticed, so the next call cannot race it into a lost result.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let slow = tools[0]
            .call(&sandbox, id, &json!({"mode":"slow"}))
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(500), slow)
                .await
                .is_err()
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        let starts = std::fs::read_to_string(sandbox.workspace.join("starts")).unwrap();
        assert_eq!(starts.lines().count(), 2);
        assert!(sandbox.workspace.join("cancelled").exists());
        std::fs::remove_dir_all(&sandbox.workspace).unwrap();
    }

    #[tokio::test]
    async fn stdio_servers_expose_tools_and_answer_parallel_calls_by_id() {
        let sandbox = crate::tools::tests::sandbox();
        let config = super::super::Config::Stdio {
            name: "fake".into(),
            command: "/bin/bash".into(),
            args: vec!["-c".into(), SERVER.into()],
            env: BTreeMap::new(),
        };
        let tools = super::super::connect(&sandbox, config).await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].definition().name, "fake__echo");
        let (first, second) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let started = std::time::Instant::now();
        let (a, b) = tokio::join!(
            tools[0].call(&sandbox, first, &json!({})).unwrap(),
            tools[0].call(&sandbox, second, &json!({})).unwrap(),
        );
        assert!(started.elapsed() < std::time::Duration::from_millis(390));
        for outcome in [a, b] {
            let Outcome::Exited { code, output } = outcome else {
                panic!("call did not finish");
            };
            assert_eq!(code, Some(0));
            assert!(output.starts_with("echoed "), "{output}");
        }
        std::fs::remove_dir_all(&sandbox.workspace).unwrap();
    }
}

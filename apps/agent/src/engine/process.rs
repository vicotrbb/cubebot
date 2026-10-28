//! Terminal commands run as children of the agent inside its sandbox.
use cubebot_protocol::terminal::{Invocation, Outcome};
use std::{
    io::{self, Read, Seek, SeekFrom},
    process::Stdio,
};
use tokio::process::{Child, Command};

pub(super) fn spawn(invocation: &Invocation, workspace: &str) -> io::Result<(Child, String)> {
    let output = format!("{workspace}/.cubebot-terminal-output/{}.log", invocation.id);
    let child = Command::new("/bin/sh")
        .args([
            "-c",
            r#"mkdir -p -- "$HOME/.cubebot-terminal-output" && cd "$HOME" && exec /bin/bash --noprofile --norc -c "$1" > "$2" 2>&1"#,
            "terminal",
            &invocation.command,
            &output,
        ])
        // Commands share the sandbox with the agent, but not its environment.
        .env_clear()
        .env("HOME", workspace)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()?;
    Ok((child, output))
}

/// `status` is `None` when authority ended before the command exited.
pub(super) async fn finish(mut child: Child, path: &str, status: Option<Option<i32>>) -> Outcome {
    // shortcut: only the command's process group is stopped; a descendant that calls setsid
    // survives until the sandbox stops. Kill the whole guest session once the kvm backend exists.
    if let Some(id) = child.id() {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(id as i32),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
    let _ = child.wait().await;
    let output = format!(
        "{}\n\nFull output: {path}",
        String::from_utf8_lossy(&preview(path))
    );
    match status {
        Some(code) => Outcome::Exited { code, output },
        None => Outcome::Stopped {
            reason: "authority_expired".into(),
            output,
        },
    }
}

/// A response preview, never a capture limit or reason to stop a command.
fn preview(path: &str) -> Vec<u8> {
    let tail = || -> io::Result<Vec<u8>> {
        let mut file = std::fs::File::open(path)?;
        let length = file.metadata()?.len();
        file.seek(SeekFrom::Start(length.saturating_sub(8192)))?;
        let mut bytes = Vec::new();
        file.take(8192).read_to_end(&mut bytes)?;
        Ok(bytes)
    };
    tail().unwrap_or_else(|_| {
        b"Output preview unavailable; inspect the retained workspace file.".to_vec()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn commands_run_in_the_workspace_without_the_agent_environment() {
        let workspace =
            std::env::temp_dir().join(format!("cubebot-process-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&workspace).unwrap();
        let invocation = Invocation {
            id: uuid::Uuid::new_v4(),
            authority: uuid::Uuid::new_v4(),
            command: "pwd; printf '%s' \"${CUBEBOT_JOURNAL_DIRECTORY:-clean}\"; exit 4".into(),
        };
        let (mut child, path) = spawn(&invocation, workspace.to_str().unwrap()).unwrap();
        let code = child.wait().await.unwrap().code();
        let Outcome::Exited { code, output } = finish(child, &path, Some(code)).await else {
            panic!("command did not exit");
        };
        assert_eq!(code, Some(4));
        assert!(output.contains("clean"), "{output}");
        assert!(output.contains("cubebot-process-"), "{output}");
        std::fs::remove_dir_all(workspace).unwrap();
    }
}

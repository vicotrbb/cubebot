use cubebot_protocol::terminal::{Invocation, Outcome};
use std::{io, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, net::UnixStream, process::Command};

pub(super) async fn execute(
    request: &Invocation,
    not_after_millis: u64,
    stream: &mut UnixStream,
) -> io::Result<Outcome> {
    let Some(mut lease) = super::lease::Lease::new(not_after_millis)? else {
        return Ok(stopped("authority_expired"));
    };
    if !caller_connected(stream)? {
        return Ok(stopped("cancelled"));
    }
    let path = output_path(request.id);
    let mut child = spawn_user(&request.command, &path)?;
    let outcome = loop {
        let deadline = lease.deadline;
        tokio::select! {
            status = child.wait() => break Outcome::Exited { code: status?.code(), output: String::new() },
            _ = tokio::time::sleep_until(deadline) => break stopped("authority_expired"),
            size = stream.read(&mut lease.bytes[lease.received..]) => {
                if !lease.receive(size?)? { break stopped("cancelled"); }
            }
        }
    };
    // Stop detached descendants before releasing the workspace or reading the output.
    kill_namespace().await?;
    let _ = child.wait().await;
    reap_namespace().await?;
    let preview = preview(&path).await;
    kill_namespace().await?;
    reap_namespace().await?;
    let output = format!(
        "{}\n\nFull output: {path}",
        String::from_utf8_lossy(&preview)
    );
    Ok(match outcome {
        Outcome::Exited { code, .. } => Outcome::Exited { code, output },
        Outcome::Stopped { reason, .. } => Outcome::Stopped { reason, output },
        Outcome::Uncertain => Outcome::Uncertain,
    })
}
fn stopped(reason: &str) -> Outcome {
    Outcome::Stopped {
        reason: reason.into(),
        output: String::new(),
    }
}
fn output_path(id: uuid::Uuid) -> String {
    format!("/home/agent/.cubebot-terminal-output/{id}.log")
}

fn spawn_user(command: &str, output: &str) -> io::Result<tokio::process::Child> {
    user_command().args([
        "/bin/sh", "-c",
        r#"cd /home/agent && mkdir -p -- .cubebot-terminal-output && exec /bin/bash --noprofile --norc -c "$1" > "$2" 2>&1"#,
        "terminal", command, output,
    ]).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
}
async fn preview(path: &str) -> Vec<u8> {
    // This is a response preview, never a capture limit or reason to stop a command.
    let result = user_command()
        .args([
            "/usr/bin/timeout",
            "--kill-after=1",
            "2",
            "/usr/bin/tail",
            "-c",
            "8192",
            "--",
            path,
        ])
        .output()
        .await;
    match result {
        Ok(output) if output.status.success() => output.stdout,
        _ => b"Output preview unavailable; inspect the retained workspace file.".to_vec(),
    }
}
fn user_command() -> Command {
    // Keep the process-isolation policy; it is independent of task duration and output.
    let mut command = Command::new("/usr/bin/prlimit");
    command
        .args([
            "--nproc=128:128",
            "--",
            "/usr/bin/setpriv",
            "--reuid=10002",
            "--regid=10002",
            "--clear-groups",
            "--no-new-privs",
            "--inh-caps=-all",
            "--ambient-caps=-all",
        ])
        .env_clear()
        .env("HOME", "/home/agent")
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .current_dir("/")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    command
}

async fn kill_namespace() -> io::Result<()> {
    if std::process::id() != 1 {
        return Err(io::Error::other("unsafe process namespace"));
    }
    use nix::{
        sys::signal::{Signal, kill},
        unistd::Pid,
    };
    match kill(Pid::from_raw(-1), Signal::SIGKILL) {
        Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
        Err(error) => Err(io::Error::other(error)),
    }
}
async fn reap_namespace() -> io::Result<()> {
    use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        match waitpid(None, Some(WaitPidFlag::WNOHANG)) {
            Err(nix::errno::Errno::ECHILD) => return Ok(()),
            Ok(WaitStatus::StillAlive) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(io::Error::other("terminal descendants not proven stopped"));
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok(_) => (),
            Err(error) => return Err(io::Error::other(error)),
        }
    }
}

fn caller_connected(stream: &UnixStream) -> io::Result<bool> {
    use nix::poll::{PollFd, PollFlags, poll};
    use std::os::fd::AsFd;
    let shutdown = PollFlags::POLLHUP | PollFlags::POLLERR;
    #[cfg(target_os = "linux")]
    let shutdown = shutdown | PollFlags::from_bits_retain(nix::libc::POLLRDHUP);
    let mut descriptors = [PollFd::new(stream.as_fd(), shutdown)];
    poll(&mut descriptors, 0u8).map_err(io::Error::other)?;
    Ok(!descriptors[0]
        .revents()
        .unwrap_or(PollFlags::POLLERR)
        .intersects(shutdown))
}
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    #[tokio::test]
    async fn buffered_renewals_are_not_mistaken_for_a_disconnected_caller() {
        let (mut caller, executor) = UnixStream::pair().unwrap();
        caller.write_u64(1).await.unwrap();
        assert!(caller_connected(&executor).unwrap());
        drop(caller);
        assert!(!caller_connected(&executor).unwrap());
    }
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_half_closed_caller_cannot_start_work_even_with_buffered_frames() {
        let (mut caller, executor) = UnixStream::pair().unwrap();
        caller.write_u64(1).await.unwrap();
        let caller = caller.into_std().unwrap();
        caller.shutdown(std::net::Shutdown::Write).unwrap();
        assert!(!caller_connected(&executor).unwrap());
    }
}

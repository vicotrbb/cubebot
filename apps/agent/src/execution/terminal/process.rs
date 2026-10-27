use cubebot_protocol::terminal::{Invocation, OUTPUT_LIMIT, Outcome};
use std::{io, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    net::UnixStream,
    process::Command,
};

pub(super) async fn execute(
    request: &Invocation,
    not_after_millis: u64,
    stream: &mut UnixStream,
) -> io::Result<Outcome> {
    if !caller_present(stream) {
        return Ok(stopped("cancelled"));
    }
    let Some(deadline) = execution_deadline(not_after_millis, request.seconds)? else {
        return Ok(stopped("cancelled"));
    };
    let (mut child, stdout, stderr) = spawn_user(&request.command)?;
    let (send, mut receive) = tokio::sync::mpsc::channel(8);
    let out = tokio::spawn(capture(stdout, send.clone()));
    let err = tokio::spawn(capture(stderr, send.clone()));
    drop(send);
    let timer = tokio::time::sleep_until(deadline);
    tokio::pin!(timer);
    let mut output = Vec::new();
    let mut signal = [0u8; 1];
    let outcome = loop {
        tokio::select! {
            status = child.wait() => break Outcome::Exited { code: status?.code(), output: String::new() },
            _ = &mut timer => break stopped("duration_limit"),
            _ = stream.read(&mut signal) => break stopped("cancelled"),
            Some(chunk) = receive.recv() => {
                let available = OUTPUT_LIMIT - output.len();
                output.extend_from_slice(&chunk[..chunk.len().min(available)]);
                if chunk.len() > available { break stopped("output_limit"); }
            }
        }
    };
    // PID namespaces make -1 cover descendants even after setsid/double-fork.
    // PID 1 is excluded by Linux. Failure terminates the trusted server.
    kill_namespace().await?;
    let _ = child.wait().await;
    reap_namespace().await?;
    // Drain both pipes after all writers are proven stopped.
    while let Some(chunk) = receive.recv().await {
        let available = OUTPUT_LIMIT - output.len();
        output.extend_from_slice(&chunk[..chunk.len().min(available)]);
    }
    out.await.map_err(io::Error::other)??;
    err.await.map_err(io::Error::other)??;
    Ok(with_output(outcome, output))
}
fn with_output(outcome: Outcome, bytes: Vec<u8>) -> Outcome {
    let mut output = String::from_utf8_lossy(&bytes).into_owned();
    let mut end = output.len().min(OUTPUT_LIMIT);
    while !output.is_char_boundary(end) {
        end -= 1;
    }
    output.truncate(end);
    match outcome {
        Outcome::Exited { code, .. } => Outcome::Exited { code, output },
        Outcome::Stopped { reason, .. } => Outcome::Stopped { reason, output },
        Outcome::Uncertain => Outcome::Uncertain,
    }
}

fn stopped(reason: &str) -> Outcome {
    Outcome::Stopped {
        reason: reason.into(),
        output: String::new(),
    }
}
async fn capture(
    mut input: impl AsyncRead + Unpin,
    send: tokio::sync::mpsc::Sender<Vec<u8>>,
) -> io::Result<()> {
    let mut bytes = [0; 1024];
    loop {
        match input.read(&mut bytes).await {
            Ok(0) => return Ok(()),
            Err(error) => return Err(error),
            Ok(size) => {
                if send.send(bytes[..size].to_vec()).await.is_err() {
                    return Ok(());
                }
            }
        }
    }
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

fn caller_present(stream: &UnixStream) -> bool {
    matches!(stream.try_read(&mut [0u8; 1]), Err(error) if error.kind() == io::ErrorKind::WouldBlock)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lossy_decoding_cannot_expand_output_past_the_bound() {
        let outcome = with_output(stopped("output_limit"), vec![255; OUTPUT_LIMIT]);
        assert!(outcome.validate().is_ok());
    }
}

fn execution_deadline(not_after: u64, seconds: u64) -> io::Result<Option<tokio::time::Instant>> {
    let start = tokio::time::Instant::now();
    let remaining = not_after.saturating_sub(super::monotonic_millis()?);
    if remaining == 0 || remaining > 30000 {
        return Ok(None);
    }
    Ok(Some(
        start + Duration::from_millis(remaining.min(seconds * 1000)),
    ))
}

fn spawn_user(
    command: &str,
) -> io::Result<(
    tokio::process::Child,
    tokio::process::ChildStdout,
    tokio::process::ChildStderr,
)> {
    // ponytail: the hard per-UID process limit is shared across agents on a node.
    // Use qualified per-Pod cgroup PID limits before increasing parallel capacity.
    let mut child = Command::new("/usr/bin/prlimit")
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
            "/bin/sh",
            "-c",
            "cd /home/agent && exec /bin/bash --noprofile --norc -c \"$1\"",
            "terminal",
            command,
        ])
        .env_clear()
        .env("HOME", "/home/agent")
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("missing stderr"))?;
    Ok((child, stdout, stderr))
}

#[cfg(test)]
mod authority_tests {
    use super::*;
    #[test]
    fn waiting_cannot_renew_expired_or_excessive_authority() {
        let now = super::super::monotonic_millis().unwrap();
        assert!(
            execution_deadline(now.saturating_sub(1), 30)
                .unwrap()
                .is_none()
        );
        assert!(execution_deadline(now + 60000, 30).unwrap().is_none());
        assert!(execution_deadline(now + 10000, 1).unwrap().is_some());
    }
}

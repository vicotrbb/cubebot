//! Trusted PID-1 supervisor. Commands have a distinct UID and no runtime mounts.
mod lease;
mod process;
mod store;
use cubebot_protocol::terminal::{REQUEST_LIMIT, Request};
use std::{
    io,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
};

pub async fn serve() -> io::Result<()> {
    if !cfg!(target_os = "linux") || std::process::id() != 1 {
        return Err(io::Error::other(
            "terminal supervisor requires a dedicated Linux PID namespace",
        ));
    }
    validate_supervisor()?;
    let store = std::sync::Arc::new(tokio::sync::Mutex::new(store::Store::open(Path::new(
        "/var/lib/cubebot-executor/journal",
    ))?));
    let listener = listen()?;
    let mut sessions = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            result = sessions.join_next(), if !sessions.is_empty() => {
                result.expect("nonempty sessions").map_err(io::Error::other)??;
            }
            accepted = listener.accept() => {
                let (mut stream, _) = accepted?;
                if !matches!(stream.peer_cred(), Ok(peer) if peer.uid() == 10001) || sessions.len() >= 8 { continue; }
                let store = store.clone();
                sessions.spawn(async move { session(&mut stream, store).await });
            }
        }
    }
}

fn listen() -> io::Result<UnixListener> {
    let socket = Path::new("/run/cubebot-executor/terminal.sock");
    let parent = std::fs::symlink_metadata(socket.parent().expect("fixed socket parent"))?;
    if parent.uid() != 0 || parent.mode() & 0o022 != 0 {
        return Err(io::Error::other("untrusted terminal socket directory"));
    }
    // A socket left by a prior process is never authority to run its invocation again.
    if socket.exists() {
        std::fs::remove_file(socket)?;
    }
    let listener = UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o666))?;
    Ok(listener)
}

async fn session(
    stream: &mut UnixStream,
    store: std::sync::Arc<tokio::sync::Mutex<store::Store>>,
) -> io::Result<()> {
    let request =
        match tokio::time::timeout(std::time::Duration::from_secs(2), read_request(stream)).await {
            Ok(Ok(request)) => request,
            _ => return Ok(()),
        };
    let outcome = {
        let mut store = store.lock().await;
        match request {
            Request::Lookup { id, authority } => store.lookup(id, authority)?,
            Request::Execute {
                invocation,
                not_after_millis,
            } => {
                invocation.validate().map_err(io::Error::other)?;
                match store.begin(&invocation)? {
                    Some(outcome) => outcome,
                    None => {
                        let outcome =
                            process::execute(&invocation, not_after_millis, stream).await?;
                        store.finish(invocation.id, &outcome)?;
                        outcome
                    }
                }
            }
        }
    };
    reply(stream, &outcome).await
}

async fn reply(
    stream: &mut UnixStream,
    outcome: &cubebot_protocol::terminal::Outcome,
) -> io::Result<()> {
    let response = serde_json::to_vec(outcome)?;
    // Completion is durable before delivery. Half-close the reply, then let the
    // caller stop its renewals and close, avoiding a reset from unread frames.
    let _ = stream.write_all(&response).await;
    let _ = stream.shutdown().await;
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(cubebot_protocol::terminal::LEASE_MILLIS),
        async {
            let mut pending = [0u8; 1024];
            while stream.read(&mut pending).await? != 0 {}
            Ok::<(), io::Error>(())
        },
    )
    .await;
    Ok(())
}

async fn read_request(stream: &mut UnixStream) -> io::Result<Request> {
    let size = stream.read_u32().await? as usize;
    if size > REQUEST_LIMIT {
        return Err(io::Error::other("terminal request too large"));
    }
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn validate_supervisor() -> io::Result<()> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    let capabilities = status
        .lines()
        .find(|line| line.starts_with("CapEff:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| u64::from_str_radix(value, 16).ok());
    if std::fs::metadata("/proc/self")?.uid() != 0
        || capabilities != Some(0xe0)
        || !status.lines().any(|line| line == "NoNewPrivs:\t1")
    {
        return Err(io::Error::other(
            "terminal requires only KILL/SETUID/SETGID and no-new-privileges",
        ));
    }
    Ok(())
}

/// A shared-Pod kernel clock. This deadline cannot be renewed by waiting in a queue.
pub fn monotonic_millis() -> io::Result<u64> {
    use nix::{
        sys::time::TimeValLike,
        time::{ClockId, clock_gettime},
    };
    let stamp = clock_gettime(ClockId::CLOCK_MONOTONIC).map_err(io::Error::other)?;
    u64::try_from(stamp.num_milliseconds()).map_err(io::Error::other)
}

#[cfg(test)]
mod reply_tests {
    use super::*;
    #[tokio::test]
    async fn response_completion_does_not_reset_pending_renewals() {
        let (mut server, mut caller) = UnixStream::pair().unwrap();
        let task = tokio::spawn(async move {
            reply(
                &mut server,
                &cubebot_protocol::terminal::Outcome::Exited {
                    code: Some(0),
                    output: "done".into(),
                },
            )
            .await
            .unwrap();
        });
        caller.write_u64(123).await.unwrap();
        let mut bytes = Vec::new();
        caller.read_to_end(&mut bytes).await.unwrap();
        let result: cubebot_protocol::terminal::Outcome = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            result,
            cubebot_protocol::terminal::Outcome::Exited {
                code: Some(0),
                output: "done".into()
            }
        );
        assert!(!task.is_finished());
        drop(caller);
        task.await.unwrap();
    }
}

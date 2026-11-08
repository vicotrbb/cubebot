//! The workspace in backups. It leaves as a tar stream through the orchestrator, which alone
//! holds the bucket's credentials, and a restored agent puts it back before it takes any work.
//! tar runs at the lowest CPU and I/O priority.
use crate::control::{Error, MemoryClient};
use futures_util::StreamExt;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// GNU tar's status when files changed while it read them, which a backup that never pauses accepts.
const CHANGED: i32 = 1;

/// As the tool user when the agent has tools; otherwise nothing but the agent writes the
/// workspace, so tar runs as the agent.
fn tar(arguments: &[&str]) -> Result<tokio::process::Command, String> {
    let mut command = match crate::tools::Sandbox::from_environment() {
        Ok(sandbox) => sandbox.command("nice"),
        Err(_) => {
            let workspace = std::env::var_os("CUBEBOT_WORKSPACE_DIRECTORY")
                .ok_or("the workspace directory is not configured")?;
            let mut command = tokio::process::Command::new("nice");
            command.current_dir(workspace).kill_on_drop(true);
            command
        }
    };
    command
        .args([
            "-n",
            "19",
            "ionice",
            "-c",
            "3",
            "tar",
            "--numeric-owner",
            "--directory=.",
        ])
        .args(arguments)
        .stderr(Stdio::null());
    Ok(command)
}

/// Sends the workspace for `backup`; the orchestrator records anything that goes wrong.
pub async fn back_up(client: MemoryClient, backup: String) {
    let command = tar(&[
        "--create",
        "--file=-",
        "--warning=no-file-changed",
        "--warning=no-file-removed",
        "--ignore-failed-read",
        ".",
    ]);
    let child = command.and_then(|mut command| {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| error.to_string())
    });
    let mut child = match child {
        Ok(child) => child,
        Err(error) => return eprintln!("Workspace backup could not start: {error}"),
    };
    let Some(output) = child.stdout.take() else {
        return;
    };
    // The stream ends in an error, so the upload fails, when tar does.
    let body =
        futures_util::stream::try_unfold((output, Some(child)), |(mut output, child)| async move {
            let mut buffer = vec![0; 256 * 1024];
            let read = output.read(&mut buffer).await?;
            if read > 0 {
                buffer.truncate(read);
                return Ok(Some((buffer, (output, child))));
            }
            let Some(mut child) = child else {
                return Ok(None);
            };
            let status = child.wait().await?;
            match status.code() {
                Some(0 | CHANGED) => Ok(None),
                _ => Err(std::io::Error::other(format!("tar ended with {status}"))),
            }
        });
    if let Err(error) = client
        .put_workspace(&backup, reqwest::Body::wrap_stream(body))
        .await
    {
        eprintln!("Workspace backup not sent: {error}");
    }
}

/// Puts back the workspace a restore left for this agent. Until that succeeds the agent takes no
/// work; each failure is reported, and the next connection attempt tries again.
pub async fn restore(client: &MemoryClient) -> Result<(), Error> {
    let Some(response) = client.workspace_restore().await? else {
        return Ok(());
    };
    eprintln!("Restoring the workspace from a backup.");
    let extracted = extract(response).await;
    client
        .workspace_restored(extracted.as_ref().err().cloned())
        .await?;
    extracted.map_err(|error| Error::unavailable(format!("workspace restore failed: {error}")))
}

async fn extract(response: reqwest::Response) -> Result<(), String> {
    let mut child = tar(&["--extract", "--file=-"])?
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|error| format!("tar could not start: {error}"))?;
    let mut input = child.stdin.take().ok_or("tar's input was not captured")?;
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|error| {
            format!(
                "the backed-up workspace could not be read: {}",
                error.without_url()
            )
        })?;
        input
            .write_all(&chunk)
            .await
            .map_err(|error| format!("tar stopped reading the workspace: {error}"))?;
    }
    drop(input);
    let status = child.wait().await.map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("tar could not extract the workspace ({status})"));
    }
    eprintln!("Workspace restored.");
    Ok(())
}

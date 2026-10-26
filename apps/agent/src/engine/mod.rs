//! Native identity/control loop. Turn admission, inference and tool execution remain pending.
use crate::{
    control::{Config, RuntimeClient},
    journal::Journal,
};
use cubebot_protocol::runtime::RuntimeDisposition;
use std::{os::unix::fs::MetadataExt, path::Path, time::Duration};
use tokio::signal::unix::{Signal, SignalKind};
use uuid::Uuid;

pub async fn run() -> Result<(), &'static str> {
    let config = Config::from_environment().map_err(|_| "invalid runtime configuration")?;
    let root =
        std::env::var_os("CUBEBOT_JOURNAL_DIRECTORY").ok_or("runtime storage not configured")?;
    let mut journal = open_journal(Path::new(&root))?;
    let mut client = RuntimeClient::new(config).map_err(|_| "runtime transport unavailable")?;
    let mut terminate = tokio::signal::unix::signal(SignalKind::terminate())
        .map_err(|_| "runtime signal handler unavailable")?;
    let mut drain = None;
    loop {
        tokio::select! {
            _ = shutdown(&mut terminate) => return Ok(()),
            outcome = cycle(&mut client, &mut journal, &mut drain) => match outcome {
                Ok(true) => return Ok(()),
                Ok(false) if !journal.pending().is_empty() => continue,
                Ok(false) => (),
                Err(error) => {
                    client.invalidate();
                    eprintln!("Runtime control waiting: {error}");
                }
            }
        }
        // ponytail: bounded fixed polling; use server push and jitter when delivery is implemented.
        tokio::select! {
            _ = shutdown(&mut terminate) => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(5)) => (),
        }
    }
}

fn open_journal(root: &Path) -> Result<Journal, &'static str> {
    let metadata =
        std::fs::symlink_metadata(root).map_err(|_| "private runtime storage unavailable")?;
    if !metadata.is_dir() || metadata.mode() & 0o077 != 0 {
        return Err("private runtime storage required");
    }
    Journal::open(&root.join("journal"))
        .map_err(|_| "runtime journal unavailable; recovery required")
}
async fn shutdown(terminate: &mut Signal) {
    tokio::select! {
        _ = terminate.recv() => (),
        _ = tokio::signal::ctrl_c() => (),
    }
}
async fn cycle(
    client: &mut RuntimeClient,
    journal: &mut Journal,
    latched: &mut Option<Uuid>,
) -> Result<bool, crate::control::Error> {
    use crate::control::Error;
    if client.requires_bootstrap() {
        client.bootstrap().await?;
    }
    let control = client.control().await?;
    if let Some(request) = latched
        && control.drain.as_ref().map(|drain| drain.request_id) != Some(*request)
    {
        return Err(Error::Protocol);
    }
    if let Some(record) = journal.pending().first() {
        client
            .synchronize_record(record.sequence, &record.payload)
            .await?;
        journal
            .acknowledge(record.sequence)
            .map_err(|_| Error::Journal)?;
    }
    if matches!(control.disposition, RuntimeDisposition::Drain) {
        let request = control.drain.ok_or(Error::Protocol)?;
        *latched = Some(request.request_id);
        // No turns or tool invocations can run in this control-only engine yet.
        // Custody acknowledgement permits local cleanup, not semantic acceptance or new work.
        if !journal.pending().is_empty() {
            return Ok(false);
        }
        client.acknowledge_drain(request.request_id).await?;
        return Ok(true);
    }
    Ok(false)
}

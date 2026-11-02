//! `cubebot-agent container`: PID 1 of the `container` sandbox (ADR 0084 as amended). The
//! launcher mounts the workspace volume on `/var/lib/cubebot` and every `*_FILE`, and passes the
//! agent's settings as `CUBEBOT_*` variables; there are no disks to mount and no routes to set.
//! It refuses a volume a `kvm` sandbox laid out, hands the settings to [`crate::init`], passes
//! SIGTERM and SIGINT on to the agent and exits with the agent's status, which ends the container.
use crate::init::{PRIVATE, STATE};
use nix::sys::signal::{SigSet, Signal, kill};
use std::{io, os::unix::fs::PermissionsExt, path::Path, process::ExitCode};

pub fn init() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("Container init failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<u8> {
    if Path::new(STATE).join("workspace.ext4").exists() {
        eprintln!("The workspace volume holds a kvm sandbox's disk; a container cannot use it");
        return Ok(cubebot_protocol::LAYOUT_MISMATCH);
    }
    // Blocked before any thread or child starts, so only the forwarding thread receives them;
    // children start with an empty mask.
    let mut termination = SigSet::empty();
    termination.add(Signal::SIGTERM);
    termination.add(Signal::SIGINT);
    termination.thread_block()?;
    // The launcher's mounts live here; the outer display's cookie joins them.
    std::fs::create_dir_all(PRIVATE)?;
    std::fs::set_permissions(PRIVATE, std::fs::Permissions::from_mode(0o700))?;
    let environment = std::env::vars()
        .filter(|(name, _)| name.starts_with("CUBEBOT_"))
        .collect();
    let agent = crate::init::start(environment, crate::desktop::Host::Socket)?;
    std::thread::spawn(move || {
        while let Ok(signal) = termination.wait() {
            let _ = kill(agent, signal);
        }
    });
    crate::init::reap(agent)
}

//! Development backend: the agent is a plain child process. No isolation claim.
use nix::{
    errno::Errno,
    sys::{
        signal::{SigSet, Signal, kill},
        wait::{WaitPidFlag, WaitStatus, waitpid},
    },
    unistd::Pid,
};
use std::{io, os::unix::fs::DirBuilderExt, process::Command};

pub fn run() -> io::Result<u8> {
    if let Some(journal) = std::env::var_os("CUBEBOT_JOURNAL_DIRECTORY") {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(journal)?;
    }
    let agent = std::env::current_exe()?.with_file_name("cubebot-agent");
    let child = Pid::from_raw(Command::new(agent).spawn()?.id() as i32);
    let mut signals = SigSet::empty();
    for signal in [Signal::SIGCHLD, Signal::SIGTERM, Signal::SIGINT] {
        signals.add(signal);
    }
    signals.thread_block().map_err(io::Error::from)?;
    // As PID 1 in a container this also reaps orphaned tool processes.
    loop {
        if let Some(code) = reap(child)? {
            return Ok(code);
        }
        match signals.wait().map_err(io::Error::from)? {
            Signal::SIGTERM | Signal::SIGINT => match kill(child, Signal::SIGTERM) {
                Ok(()) | Err(Errno::ESRCH) => (),
                Err(error) => return Err(error.into()),
            },
            _ => (),
        }
    }
}

/// Reaps every exited child and returns the agent's exit code once it has stopped.
fn reap(agent: Pid) -> io::Result<Option<u8>> {
    loop {
        match waitpid(None, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(pid, code)) if pid == agent => return Ok(Some(code as u8)),
            Ok(WaitStatus::Signaled(pid, signal, _)) if pid == agent => {
                return Ok(Some(128 + signal as u8));
            }
            Ok(WaitStatus::StillAlive) => return Ok(None),
            Ok(_) => (),
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::zombie_processes, reason = "reap() is the code under test")]
    fn reaping_reports_the_agent_exit_code_and_ignores_other_children() {
        Command::new("/bin/sh")
            .args(["-c", "exit 3"])
            .spawn()
            .unwrap();
        let agent = Command::new("/bin/sh")
            .args(["-c", "exit 7"])
            .spawn()
            .unwrap();
        let agent = Pid::from_raw(agent.id() as i32);
        let code = loop {
            if let Some(code) = reap(agent).unwrap() {
                break code;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(code, 7);
    }
}

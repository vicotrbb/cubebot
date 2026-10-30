//! One agent sandbox (ADR 0084): a KVM guest, the only way Cubebot runs an agent.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod kvm;
#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
mod png;
use std::{io, os::unix::net::UnixStream, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["run"] => run(),
        ["desktop"] => desktop().map(|()| 0),
        _ => {
            eprintln!("usage: cubebot-sandbox run | cubebot-sandbox desktop");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("Sandbox failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn run() -> io::Result<u8> {
    kvm::run()
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn run() -> io::Result<u8> {
    Err(io::Error::other("the sandbox needs x86_64 Linux"))
}

/// The host channel (ADR 0085): relays desktop request lines on stdin to the running sandbox's
/// arbiter and its responses to stdout. The orchestrator runs it inside the sandbox container,
/// so reaching it takes the platform's exec authorization for that container.
fn desktop() -> io::Result<()> {
    let path = std::env::var_os("CUBEBOT_GUEST_DESKTOP_SOCKET")
        .unwrap_or_else(|| "/tmp/cubebot-desktop.sock".into());
    let socket = UnixStream::connect(&path)?;
    let mut requests = socket.try_clone()?;
    std::thread::spawn(move || {
        let _ = io::copy(&mut io::stdin().lock(), &mut requests);
        // The arbiter answers what it already has, then sees the end.
        let _ = requests.shutdown(std::net::Shutdown::Write);
    });
    io::copy(&mut &socket, &mut io::stdout().lock()).map(drop)
}

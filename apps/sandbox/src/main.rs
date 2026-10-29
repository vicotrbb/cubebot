//! One agent sandbox (ADR 0084): a KVM guest, the only way Cubebot runs an agent.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod kvm;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args != ["run"] {
        eprintln!("usage: cubebot-sandbox run");
        return ExitCode::FAILURE;
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    let result = kvm::run();
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    let result: std::io::Result<u8> = Err(std::io::Error::other("the sandbox needs x86_64 Linux"));
    match result {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("Sandbox failed: {error}");
            ExitCode::FAILURE
        }
    }
}

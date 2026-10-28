//! One agent sandbox (ADR 0084). Only the `kvm` backend may carry an isolation claim.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod kvm;
mod process;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let backend = match args.as_slice() {
        [flag, backend] if flag == "--backend" => backend.as_str(),
        _ => {
            eprintln!("usage: cubebot-sandbox --backend kvm|process");
            return ExitCode::FAILURE;
        }
    };
    let result = match backend {
        "process" => process::run(),
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        "kvm" => kvm::run(),
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        "kvm" => Err(std::io::Error::other("the kvm backend needs x86_64 Linux")),
        _ => Err(std::io::Error::other("unknown sandbox backend")),
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("Sandbox failed: {error}");
            ExitCode::FAILURE
        }
    }
}

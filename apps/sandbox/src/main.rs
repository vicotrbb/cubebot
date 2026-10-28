//! One agent sandbox (ADR 0084). Only the `kvm` backend may carry an isolation claim.
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
        // shortcut: the guest VMM is not built yet, refuse instead of silently running unisolated.
        "kvm" => Err(std::io::Error::other("kvm backend is not implemented")),
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

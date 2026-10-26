//! Runtime-container entry point. Native agent execution is not implemented yet.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() == 2 && args[0] == "--initialize-storage" {
        return match cubebot_agent::storage::initialize(&args[1]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Storage initialization failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    eprintln!("Cubebot agent runtime is not implemented yet.");
    ExitCode::FAILURE
}

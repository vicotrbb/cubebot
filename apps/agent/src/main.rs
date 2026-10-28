//! Agent runtime entry point. It runs inside `cubebot-sandbox` (ADR 0084).
use std::process::ExitCode;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    if std::env::args().len() > 1 {
        eprintln!("Unknown runtime arguments");
        return ExitCode::FAILURE;
    }
    match cubebot_agent::engine::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Runtime startup failed: {error}");
            ExitCode::FAILURE
        }
    }
}

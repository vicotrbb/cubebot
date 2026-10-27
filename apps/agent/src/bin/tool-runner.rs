//! Trusted executor entry point. Admission requires explicit opt-in and qualified Pod controls.
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    if std::env::var("CUBEBOT_TERMINAL_QUALIFICATION").as_deref() != Ok("1")
        && std::env::var("CUBEBOT_TERMINAL_ENABLED").as_deref() != Ok("1")
    {
        eprintln!("Terminal execution is not qualified or enabled.");
        return ExitCode::FAILURE;
    }
    match cubebot_agent::execution::terminal::serve().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Terminal supervisor stopped: {}", error.kind());
            ExitCode::FAILURE
        }
    }
}

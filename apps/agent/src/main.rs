//! Agent runtime entry point. It runs inside its sandbox: a `kvm` guest or a container (ADR 0084).
use std::process::ExitCode;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // Quiet unless RUST_LOG asks; a subscriber already installed is kept, so the error is moot.
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    // Re-executed as the tool user to apply one edit_file call.
    if std::env::args().nth(1).as_deref() == Some("edit-file") && std::env::args().len() == 2 {
        return cubebot_agent::tools::edit_file();
    }
    // The kvm guest's init, which runs the agent as its child.
    #[cfg(target_os = "linux")]
    if std::env::args().nth(1).as_deref() == Some("guest") && std::env::args().len() == 2 {
        return cubebot_agent::guest::init();
    }
    // PID 1 of the container runtime's sandbox, which runs the agent as its child.
    #[cfg(target_os = "linux")]
    if std::env::args().nth(1).as_deref() == Some("container") && std::env::args().len() == 2 {
        return cubebot_agent::container::init();
    }
    // The container runtime's host channel, run by the orchestrator through the platform's exec.
    if std::env::args().nth(1).as_deref() == Some("desktop") && std::env::args().len() == 2 {
        return match cubebot_agent::desktop::channel() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Desktop host channel failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if std::env::args().len() > 1 {
        eprintln!("Unknown runtime arguments");
        return ExitCode::FAILURE;
    }
    eprintln!("cubebot-agent {} starting", env!("CARGO_PKG_VERSION"));
    match cubebot_agent::engine::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Runtime startup failed: {error}");
            ExitCode::FAILURE
        }
    }
}

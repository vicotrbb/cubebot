use std::process::ExitCode;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // Quiet unless RUST_LOG asks; a subscriber already installed is kept, so the error is moot.
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next()) {
        (None, _) => (),
        (Some("edit-file"), None) => return cubebot_agent::tools::edit_file(),
        #[cfg(target_os = "linux")]
        (Some("guest"), None) => return cubebot_agent::guest::init(),
        #[cfg(target_os = "linux")]
        (Some("container"), None) => return cubebot_agent::container::init(),
        (Some("desktop"), None) => {
            return match cubebot_agent::desktop::channel() {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("Desktop host channel failed: {error}");
                    ExitCode::FAILURE
                }
            };
        }
        _ => {
            eprintln!("Unknown runtime arguments");
            return ExitCode::FAILURE;
        }
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

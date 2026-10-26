//! Trusted runtime entry point and storage initialization command.
use std::process::ExitCode;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
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
    if args.len() == 2 && args[0] == "--initialize-network" {
        return initialize_network(&args[1]).await;
    }
    if !args.is_empty() {
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

async fn initialize_network(input: &str) -> ExitCode {
    match cubebot_agent::execution::network::initialize(input).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Network initialization failed: {error}");
            ExitCode::FAILURE
        }
    }
}

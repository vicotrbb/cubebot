use std::{error::Error, net::SocketAddr};

use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let log_directives = match std::env::var("RUST_LOG") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => "info".to_owned(),
        Err(error) => return Err(error.into()),
    };
    let filter = EnvFilter::try_new(log_directives)?;
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .try_init()?;

    let router = application().await?;
    let address: SocketAddr = match std::env::var("CUBEBOT_BIND_ADDR") {
        Ok(value) => value.parse()?,
        Err(std::env::VarError::NotPresent) => "127.0.0.1:8080".parse()?,
        Err(error) => return Err(error.into()),
    };
    let listener = TcpListener::bind(address).await?;
    tracing::info!(address = %listener.local_addr()?, "orchestrator HTTP server listening");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("SIGTERM handler must be available");
        tokio::select! {
            result = tokio::signal::ctrl_c() => result.expect("SIGINT handler must be available"),
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c()
        .await
        .expect("shutdown handler must be available");
    tracing::info!("shutdown requested");
}

async fn application() -> Result<axum::Router, Box<dyn Error + Send + Sync>> {
    match std::env::var("CUBEBOT_DATABASE_URL") {
        Ok(url) => {
            let pool = cubebot_orchestrator::operations::database::connect(&url)
                .await
                .map_err(|_| "configured database connection or migration failed")?;
            let config = cubebot_orchestrator::identity::oidc::environment()
                .map_err(|_| "invalid OIDC configuration")?;
            let origin = match std::env::var("CUBEBOT_PUBLIC_ORIGIN") {
                Ok(origin) => Some(origin),
                Err(std::env::VarError::NotPresent) => None,
                Err(_) => return Err("invalid CUBEBOT_PUBLIC_ORIGIN".into()),
            };
            cubebot_orchestrator::api::configured_router(pool, origin, config)
                .await
                .map_err(|_| "identity configuration failed".into())
        }
        Err(std::env::VarError::NotPresent) => {
            if std::env::var_os("CUBEBOT_OIDC_CONFIG").is_some() {
                return Err("OIDC requires a configured database".into());
            }
            Ok(cubebot_orchestrator::api::router())
        }
        Err(_) => Err("invalid CUBEBOT_DATABASE_URL".into()),
    }
}

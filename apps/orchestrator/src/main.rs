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

    let address: SocketAddr = match std::env::var("CUBEBOT_BIND_ADDR") {
        Ok(value) => value.parse()?,
        Err(std::env::VarError::NotPresent) => "127.0.0.1:8080".parse()?,
        Err(error) => return Err(error.into()),
    };
    let listener = TcpListener::bind(address).await?;
    let address = listener.local_addr()?;
    let (router, controller) = application().await?;
    tracing::info!(%address, "orchestrator HTTP server listening");
    cubebot_orchestrator::operations::server::serve(
        listener,
        router,
        controller,
        shutdown_signal(),
    )
    .await
    .map_err(|_| "orchestrator ownership or service failed")?;
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

type Application = (
    axum::Router,
    Option<cubebot_orchestrator::agents::kubernetes::Controller>,
);

async fn application() -> Result<Application, Box<dyn Error + Send + Sync>> {
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
            let controller = configured_controller(&pool).await?;
            let router =
                match cubebot_orchestrator::api::configured_router(pool, origin, config).await {
                    Ok(router) => router.merge(controller.runtime_router()),
                    Err(_) => {
                        controller
                            .release()
                            .await
                            .map_err(|_| "startup cleanup could not release ownership")?;
                        return Err("identity configuration failed".into());
                    }
                };
            Ok((router, Some(controller)))
        }
        Err(std::env::VarError::NotPresent) => {
            if database_required() {
                return Err(
                    "Identity, provider and controller configuration require a database".into(),
                );
            }
            Ok((cubebot_orchestrator::api::router(), None))
        }
        Err(_) => Err("invalid CUBEBOT_DATABASE_URL".into()),
    }
}

async fn configured_controller(
    pool: &sqlx::PgPool,
) -> Result<cubebot_orchestrator::agents::kubernetes::Controller, Box<dyn Error + Send + Sync>> {
    use cubebot_orchestrator::agents::kubernetes::ControllerConfig;
    match ControllerConfig::environment().map_err(|_| "invalid controller configuration")? {
        Some(config) => config
            .activate(pool.clone())
            .await
            .map_err(|_| "controller ownership or configuration failed".into()),
        None => docker_or_database_only(pool).await,
    }
}

async fn docker_or_database_only(
    pool: &sqlx::PgPool,
) -> Result<cubebot_orchestrator::agents::kubernetes::Controller, Box<dyn Error + Send + Sync>> {
    use cubebot_orchestrator::agents::{kubernetes::Controller, sandbox::DockerConfig};
    match DockerConfig::environment().map_err(|_| "invalid Docker sandbox configuration")? {
        Some(config) => Controller::docker(pool.clone(), config)
            .await
            .map_err(|_| "development ownership unavailable for Docker sandboxes".into()),
        None => Controller::database_only(pool.clone()).await.map_err(|_| {
            "database-only ownership unavailable or controller configuration required".into()
        }),
    }
}

fn database_required() -> bool {
    [
        "CUBEBOT_OIDC_CONFIG",
        "CUBEBOT_CONTROLLER_CONFIG",
        "CUBEBOT_DOCKER_CONFIG",
        "CUBEBOT_CREDENTIAL_KEY_FILE",
    ]
    .iter()
    .any(|key| std::env::var_os(key).is_some())
}

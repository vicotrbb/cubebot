use std::{error::Error, net::SocketAddr};

use cubebot_orchestrator::operations::server::{Tls, serve};
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
    let listening = listening(listener)?;
    let control = control_listener().await?;
    run(listening, control, address).await
}

struct Failure {
    message: &'static str,
    source: Box<dyn Error + Send + Sync>,
}

impl std::fmt::Debug for Failure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.message, formatter)
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl Error for Failure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

fn failure<E: Error + Send + Sync + 'static>(
    message: &'static str,
) -> impl FnOnce(E) -> Box<dyn Error + Send + Sync> {
    move |error| {
        Box::new(Failure {
            message,
            source: Box::new(error),
        })
    }
}

enum Listening {
    Plain(TcpListener),
    Tls(Tls),
}

fn listening(listener: TcpListener) -> Result<Listening, Box<dyn Error + Send + Sync>> {
    match (
        std::env::var_os("CUBEBOT_TLS_CERT_FILE"),
        std::env::var_os("CUBEBOT_TLS_KEY_FILE"),
    ) {
        (Some(chain), Some(key)) => {
            let (chain, key) = (std::fs::read(chain)?, std::fs::read(key)?);
            Ok(Listening::Tls(Tls::new(listener, &chain, &key)?))
        }
        (None, None) => Ok(Listening::Plain(listener)),
        _ => Err("set both CUBEBOT_TLS_CERT_FILE and CUBEBOT_TLS_KEY_FILE, or neither".into()),
    }
}

/// Agents' own HTTPS listener, so they never route through the public ingress.
async fn control_listener() -> Result<Option<Tls>, Box<dyn Error + Send + Sync>> {
    let (address, chain, key) = match (
        std::env::var("CUBEBOT_CONTROL_BIND_ADDR"),
        std::env::var_os("CUBEBOT_CONTROL_TLS_CERT_FILE"),
        std::env::var_os("CUBEBOT_CONTROL_TLS_KEY_FILE"),
    ) {
        (Ok(address), Some(chain), Some(key)) => (address.parse::<SocketAddr>()?, chain, key),
        (Err(std::env::VarError::NotPresent), None, None) => return Ok(None),
        _ => {
            return Err("set CUBEBOT_CONTROL_BIND_ADDR, CUBEBOT_CONTROL_TLS_CERT_FILE and CUBEBOT_CONTROL_TLS_KEY_FILE together, or none".into());
        }
    };
    let listener = TcpListener::bind(address).await?;
    tracing::info!(address = %listener.local_addr()?, "control listener ready");
    let (chain, key) = (std::fs::read(chain)?, std::fs::read(key)?);
    Ok(Some(Tls::new(listener, &chain, &key)?))
}

async fn run(
    listening: Listening,
    control: Option<Tls>,
    address: SocketAddr,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let listening = restored(listening).await?;
    let (router, controller) = application().await?;
    let router = cubebot_orchestrator::api::traced(router);
    let scheme = match listening {
        Listening::Tls(_) => "HTTPS",
        Listening::Plain(_) => "HTTP",
    };
    tracing::info!(%address, scheme, "orchestrator server listening");
    let served = match listening {
        Listening::Tls(listener) => {
            serve(listener, control, router, controller, shutdown_signal()).await
        }
        Listening::Plain(listener) => {
            serve(listener, control, router, controller, shutdown_signal()).await
        }
    };
    served.map_err(failure("orchestrator ownership or service failed"))?;
    Ok(())
}

/// A fresh installation with a restore source restores before anything else starts, serving
/// only its progress meanwhile. A failed restore keeps serving its reason until stopped.
async fn restored(listening: Listening) -> Result<Listening, Box<dyn Error + Send + Sync>> {
    let Ok(url) = std::env::var("CUBEBOT_DATABASE_URL") else {
        return Ok(listening);
    };
    let plan = cubebot_orchestrator::backup::restore::plan(&url)
        .await
        .map_err(failure("the restore could not start"))?;
    let Some(plan) = plan else {
        return Ok(listening);
    };
    Ok(match listening {
        Listening::Plain(listener) => Listening::Plain(restore_on(listener, plan).await?),
        Listening::Tls(listener) => Listening::Tls(restore_on(listener, plan).await?),
    })
}

async fn restore_on<L>(
    listener: L,
    plan: cubebot_orchestrator::backup::restore::Plan,
) -> Result<L, Box<dyn Error + Send + Sync>>
where
    L: axum::serve::Listener<Addr = SocketAddr>,
{
    use cubebot_orchestrator::backup::restore;
    let (progress, watching) = tokio::sync::watch::channel(restore::RestoreProgress::default());
    let mut restoring = tokio::spawn(plan.run(progress));
    let restored = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ended = {
        let restored = restored.clone();
        async move {
            tokio::select! {
                outcome = &mut restoring => {
                    if !matches!(outcome, Ok(Ok(()))) {
                        shutdown_signal().await;
                        return;
                    }
                    restored.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                () = shutdown_signal() => {}
            }
        }
    };
    let lent = Lent::new(listener)?;
    axum::serve(lent.clone(), restore::router(watching))
        .with_graceful_shutdown(ended)
        .await?;
    if !restored.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("stopped before the restore finished".into());
    }
    lent.into_inner()
}

/// Lends a listener to the restore's server, and gives it back once that server stops.
struct Lent<L> {
    listener: std::sync::Arc<tokio::sync::Mutex<L>>,
    address: SocketAddr,
}

impl<L> Clone for Lent<L> {
    fn clone(&self) -> Self {
        Self {
            listener: self.listener.clone(),
            address: self.address,
        }
    }
}

impl<L: axum::serve::Listener<Addr = SocketAddr>> Lent<L> {
    fn new(listener: L) -> std::io::Result<Self> {
        let address = listener.local_addr()?;
        Ok(Self {
            listener: std::sync::Arc::new(tokio::sync::Mutex::new(listener)),
            address,
        })
    }

    fn into_inner(self) -> Result<L, Box<dyn Error + Send + Sync>> {
        std::sync::Arc::try_unwrap(self.listener)
            .map(tokio::sync::Mutex::into_inner)
            .map_err(|_| "the listener is still lent after the restore".into())
    }
}

impl<L: axum::serve::Listener<Addr = SocketAddr>> axum::serve::Listener for Lent<L> {
    type Io = L::Io;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        self.listener.lock().await.accept().await
    }
    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        Ok(self.address)
    }
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
                .map_err(failure(
                    "configured database connection or migration failed",
                ))?;
            startup_records(&pool).await?;
            let config = cubebot_orchestrator::identity::oidc::environment()
                .map_err(failure("invalid OIDC configuration"))?;
            let origin = match std::env::var("CUBEBOT_PUBLIC_ORIGIN") {
                Ok(origin) => Some(origin),
                Err(std::env::VarError::NotPresent) => None,
                Err(_) => return Err("invalid CUBEBOT_PUBLIC_ORIGIN".into()),
            };
            let storage = shared_files(&pool).await?;
            let controller = configured_controller(&pool).await?;
            let backups = backups(&pool, &url, storage.clone())?;
            let Ok(router) =
                cubebot_orchestrator::api::configured_router(pool, origin, config).await
            else {
                controller
                    .release()
                    .await
                    .map_err(failure("startup cleanup could not release ownership"))?;
                return Err("identity configuration failed".into());
            };
            let router =
                with_storage(controller.routes(router), storage).layer(axum::Extension(backups));
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

async fn startup_records(pool: &sqlx::PgPool) -> Result<(), Box<dyn Error + Send + Sync>> {
    credential_key(pool).await?;
    initial_administrator(pool).await?;
    cubebot_orchestrator::knowledge::Knowledge::seed_shipped_skills(pool)
        .await
        .map_err(failure("shipped skills could not be seeded"))?;
    cubebot_orchestrator::work::Work::new(pool.clone())
        .close_open_calls()
        .await
        .map_err(failure("open calls could not be closed"))?;
    Ok(())
}

/// Creates the first administrator from the deployment while setup is still open; the password
/// comes from a file, such as a mounted Secret, never from the environment.
async fn initial_administrator(pool: &sqlx::PgPool) -> Result<(), Box<dyn Error + Send + Sync>> {
    let Some(path) = std::env::var_os("CUBEBOT_BOOTSTRAP_PASSWORD_FILE") else {
        return Ok(());
    };
    let login = std::env::var("CUBEBOT_BOOTSTRAP_LOGIN")
        .map_err(|_| "CUBEBOT_BOOTSTRAP_PASSWORD_FILE needs CUBEBOT_BOOTSTRAP_LOGIN")?;
    let password = std::fs::read_to_string(path)
        .map_err(failure("CUBEBOT_BOOTSTRAP_PASSWORD_FILE could not be read"))?;
    let identity = cubebot_orchestrator::identity::Identity::new(pool.clone());
    if !identity
        .setup_open()
        .await
        .map_err(failure("installation setup state unavailable"))?
    {
        return Ok(());
    }
    match identity
        .bootstrap(&login, password.trim_end_matches(['\r', '\n']))
        .await
    {
        Ok(id) => tracing::info!(account = %id, "initial administrator created"),
        // Another path created the administrator first.
        Err(cubebot_orchestrator::identity::IdentityError::Conflict) => {}
        Err(cubebot_orchestrator::identity::IdentityError::InvalidInput) => {
            return Err("the initial administrator was refused: a login is 3 to 64 lowercase letters, digits, '.', '_' or '-', and a password 12 to 1024 characters".into());
        }
        Err(error) => {
            return Err(failure("the initial administrator could not be created")(
                error,
            ));
        }
    }
    Ok(())
}

/// Refuses a key that cannot decrypt what is stored, before a new key can encrypt anything.
async fn credential_key(pool: &sqlx::PgPool) -> Result<(), Box<dyn Error + Send + Sync>> {
    use cubebot_orchestrator::connections::{Connections, Credentials};
    let Some(path) = std::env::var_os("CUBEBOT_CREDENTIAL_KEY_FILE") else {
        return Ok(());
    };
    let credentials = Credentials::from_file(std::path::Path::new(&path)).map_err(failure(
        "CUBEBOT_CREDENTIAL_KEY_FILE is not a private 32-byte key",
    ))?;
    let matches = Connections::new(pool.clone(), credentials)
        .key_matches()
        .await
        .map_err(failure("stored credentials could not be read"))?;
    if !matches {
        return Err("the credential key does not match the one that encrypted this installation's credentials; restore the previous CUBEBOT_CREDENTIAL_KEY_FILE".into());
    }
    Ok(())
}

async fn shared_files(
    pool: &sqlx::PgPool,
) -> Result<Option<cubebot_orchestrator::storage::Storage>, Box<dyn Error + Send + Sync>> {
    let storage = cubebot_orchestrator::storage::Storage::environment(pool.clone())
        .await
        .map_err(|source| Failure {
            message: "shared-file storage unavailable or reconciliation failed",
            source,
        })?;
    if let Some(storage) = &storage {
        let settings = cubebot_orchestrator::operations::settings::Settings::load(pool)
            .await
            .map_err(failure("installation settings unavailable"))?;
        let (quota, upload) = settings.limits();
        storage.limit(quota, upload);
    }
    Ok(storage)
}

/// Starts the backup worker, once this orchestrator owns the installation.
fn backups(
    pool: &sqlx::PgPool,
    url: &str,
    storage: Option<cubebot_orchestrator::storage::Storage>,
) -> Result<cubebot_orchestrator::backup::Backups, Box<dyn Error + Send + Sync>> {
    let credentials = std::env::var_os("CUBEBOT_CREDENTIAL_KEY_FILE")
        .map(|path| {
            cubebot_orchestrator::connections::Credentials::from_file(std::path::Path::new(&path))
        })
        .transpose()
        .map_err(failure(
            "CUBEBOT_CREDENTIAL_KEY_FILE is not a private 32-byte key",
        ))?;
    let backups = cubebot_orchestrator::backup::Backups::new(
        pool.clone(),
        url.to_owned(),
        credentials,
        storage,
    );
    backups.start();
    Ok(backups)
}

fn with_storage(
    router: axum::Router,
    storage: Option<cubebot_orchestrator::storage::Storage>,
) -> axum::Router {
    match storage {
        Some(storage) => router.layer(axum::Extension(storage)),
        None => router,
    }
}

async fn configured_controller(
    pool: &sqlx::PgPool,
) -> Result<cubebot_orchestrator::agents::kubernetes::Controller, Box<dyn Error + Send + Sync>> {
    use cubebot_orchestrator::agents::kubernetes::ControllerConfig;
    match ControllerConfig::environment().map_err(failure("invalid controller configuration"))? {
        Some(config) => config
            .activate(pool.clone())
            .await
            .map_err(failure("controller ownership or configuration failed")),
        None => docker_or_database_only(pool).await,
    }
}

async fn docker_or_database_only(
    pool: &sqlx::PgPool,
) -> Result<cubebot_orchestrator::agents::kubernetes::Controller, Box<dyn Error + Send + Sync>> {
    use cubebot_orchestrator::agents::{kubernetes::Controller, sandbox::DockerConfig};
    match DockerConfig::environment().map_err(failure("invalid Docker sandbox configuration"))? {
        Some(config) => Controller::docker(pool.clone(), config)
            .await
            .map_err(failure(
                "orchestrator ownership unavailable for Docker sandboxes",
            )),
        None => Controller::database_only(pool.clone())
            .await
            .map_err(failure(
                "database-only ownership unavailable or controller configuration required",
            )),
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

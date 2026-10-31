//! Local operator commands: bootstrap, agent credential revocation, and the browser API document.
//! No public registration
//! endpoint is exposed.

use cubebot_orchestrator::{
    agents::{AgentId, runtime},
    identity::Identity,
    operations::database,
};

const USAGE: &str = "usage: cubebot-admin bootstrap | cubebot-admin revoke-runtime <agent-id> | cubebot-admin openapi";

/// A command failure that reads, and prints from `main`, as its message alone.
struct Failure {
    message: &'static str,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
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

impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_deref().map(|source| source as _)
    }
}

/// Environment errors are not carried: a non-Unicode value would hold the URL or password.
impl From<&'static str> for Failure {
    fn from(message: &'static str) -> Self {
        Self {
            message,
            source: None,
        }
    }
}

fn failure<E: std::error::Error + Send + Sync + 'static>(
    message: &'static str,
) -> impl FnOnce(E) -> Failure {
    move |error| Failure {
        message,
        source: Some(Box::new(error)),
    }
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["openapi"] {
        println!("{}", cubebot_orchestrator::api::openapi());
        return Ok(());
    }
    let url = std::env::var("CUBEBOT_DATABASE_URL").map_err(|_| "database URL required")?;
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["bootstrap"] => bootstrap(&url).await,
        ["revoke-runtime", agent] => revoke(&url, agent).await,
        _ => Err(USAGE.into()),
    }
}

async fn bootstrap(url: &str) -> Result<(), Failure> {
    let login = std::env::var("CUBEBOT_BOOTSTRAP_LOGIN").map_err(|_| "bootstrap login required")?;
    let password =
        std::env::var("CUBEBOT_BOOTSTRAP_PASSWORD").map_err(|_| "bootstrap password required")?;
    let pool = database::connect(url)
        .await
        .map_err(failure("database unavailable"))?;
    // IdentityError has no std::error::Error impl to carry as a source.
    let id = Identity::new(pool)
        .bootstrap(&login, &password)
        .await
        .map_err(
            |_| "bootstrap rejected: invalid input, already bootstrapped, or database failure",
        )?;
    println!("Administrator created: {id}");
    Ok(())
}

async fn revoke(url: &str, agent: &str) -> Result<(), Failure> {
    let agent = AgentId(agent.parse().map_err(failure(USAGE))?);
    let pool = database::connect(url)
        .await
        .map_err(failure("database unavailable"))?;
    let revoked = runtime::revoke(&pool, agent)
        .await
        .map_err(failure("revocation failed"))?;
    println!("Revoked agent credentials: {revoked}");
    Ok(())
}

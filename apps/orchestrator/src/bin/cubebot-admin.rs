//! Local operator commands: bootstrap, agent credential revocation, and the browser API document.
//! No public registration
//! endpoint is exposed.

use cubebot_orchestrator::{
    agents::{AgentId, runtime},
    identity::Identity,
    operations::database,
};

const USAGE: &str = "usage: cubebot-admin bootstrap | cubebot-admin revoke-runtime <agent-id> | cubebot-admin openapi";

#[tokio::main]
async fn main() -> Result<(), &'static str> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["openapi"] {
        println!("{}", cubebot_orchestrator::api::openapi());
        return Ok(());
    }
    let url = std::env::var("CUBEBOT_DATABASE_URL").map_err(|_| "database URL required")?;
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["bootstrap"] => bootstrap(&url).await,
        ["revoke-runtime", agent] => revoke(&url, agent).await,
        _ => Err(USAGE),
    }
}

async fn bootstrap(url: &str) -> Result<(), &'static str> {
    let login = std::env::var("CUBEBOT_BOOTSTRAP_LOGIN").map_err(|_| "bootstrap login required")?;
    let password =
        std::env::var("CUBEBOT_BOOTSTRAP_PASSWORD").map_err(|_| "bootstrap password required")?;
    let pool = database::connect(url)
        .await
        .map_err(|_| "database unavailable")?;
    let id = Identity::new(pool)
        .bootstrap(&login, &password)
        .await
        .map_err(
            |_| "bootstrap rejected: invalid input, already bootstrapped, or database failure",
        )?;
    println!("Administrator created: {id}");
    Ok(())
}

async fn revoke(url: &str, agent: &str) -> Result<(), &'static str> {
    let agent = AgentId(agent.parse().map_err(|_| USAGE)?);
    let pool = database::connect(url)
        .await
        .map_err(|_| "database unavailable")?;
    let revoked = runtime::revoke(&pool, agent)
        .await
        .map_err(|_| "revocation failed")?;
    println!("Revoked agent credentials: {revoked}");
    Ok(())
}

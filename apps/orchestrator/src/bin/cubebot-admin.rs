//! Local operator bootstrap. No public registration endpoint is exposed.

use cubebot_orchestrator::{identity::Identity, operations::database};

#[tokio::main]
async fn main() -> Result<(), &'static str> {
    if std::env::args().nth(1).as_deref() != Some("bootstrap") {
        return Err("usage: cubebot-admin bootstrap");
    }
    let url = std::env::var("CUBEBOT_DATABASE_URL").map_err(|_| "database URL required")?;
    let login = std::env::var("CUBEBOT_BOOTSTRAP_LOGIN").map_err(|_| "bootstrap login required")?;
    let password =
        std::env::var("CUBEBOT_BOOTSTRAP_PASSWORD").map_err(|_| "bootstrap password required")?;
    let pool = database::connect(&url)
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

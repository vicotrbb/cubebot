//! Concrete PostgreSQL ownership. Only the orchestrator opens this pool.

use std::time::Duration;

use sqlx::{PgConnection, PgPool, postgres::PgPoolOptions};

pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    // A cancelled migration must never return a session-level advisory lock to the pool.
    let mut connection = pool.acquire().await?.detach();
    tokio::time::timeout(Duration::from_secs(10), run_migrations(&mut connection))
        .await
        .map_err(|_| sqlx::migrate::MigrateError::Execute(sqlx::Error::PoolTimedOut))?
}

async fn run_migrations(connection: &mut PgConnection) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::query("SET statement_timeout = '5s'")
        .execute(&mut *connection)
        .await?;
    sqlx::migrate!("./migrations").run(connection).await
}

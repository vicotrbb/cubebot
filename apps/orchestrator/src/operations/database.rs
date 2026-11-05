//! Concrete PostgreSQL ownership. Only the orchestrator opens this pool.

use std::time::Duration;

use sqlx::{PgPool, postgres::PgPoolOptions};

// The URL carries the database password, so the span records nothing.
#[tracing::instrument(skip_all)]
pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

#[tracing::instrument(skip_all)]
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    // These settings and the session-level advisory lock must never reach the pool.
    let mut connection = pool.acquire().await?.detach();
    // A migration may run as long as it needs, but a blocked one fails rather than queueing traffic.
    sqlx::query!("SET statement_timeout = 0")
        .execute(&mut connection)
        .await?;
    sqlx::query!("SET lock_timeout = '5s'")
        .execute(&mut connection)
        .await?;
    sqlx::migrate!("./migrations").run(&mut connection).await
}

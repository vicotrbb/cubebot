use serde::Serialize;
use sqlx::PgPool;
use std::{sync::LazyLock, time::Instant};
use uuid::Uuid;

static STARTED: LazyLock<Instant> = LazyLock::new(Instant::now);

pub fn mark_start() {
    LazyLock::force(&STARTED);
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct About {
    pub version: &'static str,
    pub installation_id: Uuid,
    pub installed_at: String,
    pub uptime_seconds: u64,
    pub namespace: Option<String>,
    pub postgres_version: String,
    pub database_bytes: i64,
    pub people: i64,
    pub agents: i64,
    pub rooms: i64,
    pub teams: i64,
    pub provider: Option<String>,
    pub model: Option<String>,
}

pub async fn about(pool: &PgPool) -> Result<About, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT i.id,
            to_jsonb(i.created_at) #>> '{}' AS "installed_at!",
            (SELECT namespace FROM installation_kubernetes) AS namespace,
            current_setting('server_version') AS "postgres_version!",
            pg_database_size(current_database()) AS "database_bytes!",
            (SELECT count(*) FROM human_accounts WHERE active) AS "people!",
            (SELECT count(*) FROM agents WHERE desired_state<>'retired') AS "agents!",
            (SELECT count(*) FROM rooms WHERE NOT task AND NOT archived) AS "rooms!",
            (SELECT count(*) FROM teams WHERE deleted_at IS NULL) AS "teams!",
            (SELECT provider FROM provider_connection) AS provider,
            (SELECT model FROM provider_connection) AS model
        FROM installation i
        "#
    )
    .fetch_one(pool)
    .await?;
    Ok(About {
        version: env!("CARGO_PKG_VERSION"),
        installation_id: row.id,
        installed_at: row.installed_at,
        uptime_seconds: STARTED.elapsed().as_secs(),
        namespace: row.namespace,
        postgres_version: row.postgres_version,
        database_bytes: row.database_bytes,
        people: row.people,
        agents: row.agents,
        rooms: row.rooms,
        teams: row.teams,
        provider: row.provider,
        model: row.model,
    })
}

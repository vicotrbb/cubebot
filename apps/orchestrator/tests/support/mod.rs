use std::hash::{DefaultHasher, Hash, Hasher};

use cubebot_orchestrator::operations::database;
use sqlx::{Connection, PgConnection, PgPool, postgres::PgConnectOptions};

pub async fn database() -> PgPool {
    let template = template(&admin_options()).await;
    create_database(Some(&template)).await
}

fn admin_options() -> PgConnectOptions {
    let url = std::env::var("CUBEBOT_TEST_DATABASE_URL").expect("isolated database URL required");
    url.parse().unwrap()
}

/// Without a template the database is unmigrated. The admin database name prefixes
/// it so a run on a shared server can drop exactly its own.
pub async fn create_database(template: Option<&str>) -> PgPool {
    let options = admin_options();
    let clause = template.map_or(String::new(), |name| format!("TEMPLATE {name}"));
    let prefix = options.get_database().unwrap_or("test");
    let name = format!("{prefix}_{}", uuid::Uuid::new_v4().simple());
    let mut admin = PgConnection::connect_with(&options).await.unwrap();
    sqlx::query(&format!("CREATE DATABASE {name} {clause}"))
        .execute(&mut admin)
        .await
        .unwrap();
    admin.close().await.unwrap();
    PgPool::connect_with(options.database(&name)).await.unwrap()
}

/// Migrates once per migration set and server; IS_TEMPLATE marks a completed migration.
async fn template(options: &PgConnectOptions) -> String {
    let mut hasher = DefaultHasher::new();
    for migration in sqlx::migrate!("./migrations").iter() {
        migration.checksum.hash(&mut hasher);
    }
    let name = format!("cubebot_template_{:016x}", hasher.finish());
    let mut admin = PgConnection::connect_with(options).await.unwrap();
    sqlx::query("SELECT pg_advisory_lock(hashtext($1))")
        .bind(&name)
        .execute(&mut admin)
        .await
        .unwrap();
    let ready: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1 AND datistemplate)",
    )
    .bind(&name)
    .fetch_one(&mut admin)
    .await
    .unwrap();
    if !ready {
        for statement in [
            format!("DROP DATABASE IF EXISTS {name}"),
            format!("CREATE DATABASE {name}"),
        ] {
            sqlx::query(&statement).execute(&mut admin).await.unwrap();
        }
        let pool = PgPool::connect_with(options.clone().database(&name))
            .await
            .unwrap();
        database::migrate(&pool).await.unwrap();
        pool.close().await;
        sqlx::query(&format!("ALTER DATABASE {name} IS_TEMPLATE true"))
            .execute(&mut admin)
            .await
            .unwrap();
    }
    // Closing the session releases the advisory lock.
    admin.close().await.unwrap();
    name
}

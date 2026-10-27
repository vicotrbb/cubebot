use cubebot_orchestrator::operations::database;
use sqlx::{PgPool, postgres::PgConnectOptions};

pub async fn database() -> PgPool {
    let pool = unmigrated_database().await;
    database::migrate(&pool).await.unwrap();
    pool
}

pub async fn unmigrated_database() -> PgPool {
    let url = std::env::var("CUBEBOT_TEST_DATABASE_URL").expect("isolated database URL required");
    let options: PgConnectOptions = url.parse().unwrap();
    let admin = PgPool::connect_with(options.clone()).await.unwrap();
    let name = format!("test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    PgPool::connect_with(options.database(&name)).await.unwrap()
}

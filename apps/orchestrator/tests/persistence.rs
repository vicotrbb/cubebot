use crate::support;
use cubebot_orchestrator::operations::database;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn migrations_are_repeatable_and_identity_survives_reconnection() {
    let pool = support::database().await;
    let id: uuid::Uuid = sqlx::query_scalar!(
        "
        INSERT INTO human_accounts (login, password_hash, role)
        VALUES ('persistent-user', 'test-verifier', 'member')
        RETURNING id
        "
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let options = pool.connect_options().as_ref().clone();
    pool.close().await;
    let pool = sqlx::PgPool::connect_with(options).await.unwrap();
    database::migrate(&pool).await.unwrap();
    let login: String = sqlx::query_scalar!(
        "
        SELECT login
        FROM human_accounts
        WHERE id = $1
        ",
        id,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(login, "persistent-user");
    let duplicate = sqlx::query!(
        "
        INSERT INTO human_accounts (login, password_hash, role)
        VALUES ('persistent-user', 'test-verifier', 'administrator')
        "
    )
    .execute(&pool)
    .await;
    assert!(
        duplicate
            .unwrap_err()
            .as_database_error()
            .unwrap()
            .is_unique_violation()
    );
    let invalid = sqlx::query!(
        "
        INSERT INTO human_accounts (login, password_hash, role)
        VALUES ('invalid', 'test-verifier', 'superuser')
        "
    )
    .execute(&pool)
    .await;
    assert!(invalid.is_err());
    let mut transaction = pool.begin().await.unwrap();
    sqlx::query!(
        "
        INSERT INTO human_accounts (login, password_hash, role)
        VALUES ('rolled-back', 'test-verifier', 'member')
        "
    )
    .execute(&mut *transaction)
    .await
    .unwrap();
    transaction.rollback().await.unwrap();
    let count: i64 = sqlx::query_scalar!(
        r#"
        SELECT count(*) AS "count!"
        FROM human_accounts
        WHERE login = 'rolled-back'
        "#
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn rejects_invalid_database_configuration() {
    assert!(database::connect("not-a-postgres-url").await.is_err());
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn migration_lock_contention_fails_within_startup_deadline() {
    let pool = support::database().await;
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query!(
        "
        LOCK TABLE _sqlx_migrations IN ACCESS EXCLUSIVE MODE
        "
    )
    .execute(&mut *blocker)
    .await
    .unwrap();
    let started = std::time::Instant::now();
    let result =
        tokio::time::timeout(std::time::Duration::from_secs(8), database::migrate(&pool)).await;
    assert!(started.elapsed() < std::time::Duration::from_secs(8));
    assert!(
        result
            .expect("migration deadline must fire before the outer guard")
            .is_err()
    );
    blocker.rollback().await.unwrap();
    // Cancellation must not leave the migration connection's advisory lock in the pool.
    database::migrate(&pool).await.unwrap();
}

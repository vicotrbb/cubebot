mod support;

use cubebot_orchestrator::identity::{Identity, IdentityError};

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn login_budget_survives_reconnect_and_resets_after_its_window() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("limited-admin", "long administrator password")
        .await
        .unwrap();
    for _ in 0..10 {
        assert!(matches!(
            identity.login("limited-admin", "wrong password").await,
            Err(IdentityError::Unauthorized)
        ));
    }
    assert!(matches!(
        identity
            .login("limited-admin", "long administrator password")
            .await,
        Err(IdentityError::Unauthorized)
    ));
    let identity = Identity::new(
        sqlx::PgPool::connect_with(pool.connect_options().as_ref().clone())
            .await
            .unwrap(),
    );
    assert!(matches!(
        identity
            .login("limited-admin", "long administrator password")
            .await,
        Err(IdentityError::Unauthorized)
    ));
    sqlx::query("UPDATE human_accounts SET auth_window_started_at = clock_timestamp() - interval '61 seconds'")
        .execute(&pool).await.unwrap();
    let session = identity
        .login("limited-admin", "long administrator password")
        .await
        .unwrap();
    assert!(
        identity
            .authenticate(&session.token)
            .await
            .unwrap()
            .administrator
    );
    let attempts: i32 = sqlx::query_scalar("SELECT auth_attempts FROM human_accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(attempts, 1);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn concurrent_attempts_cannot_overdraw_the_account_login_budget() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("concurrent-admin", "long administrator password")
        .await
        .unwrap();
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..16 {
        let identity = identity.clone();
        tasks.spawn(async move { identity.login("concurrent-admin", "wrong password").await });
    }
    while let Some(result) = tasks.join_next().await {
        assert!(matches!(
            result.unwrap(),
            Err(IdentityError::Unauthorized | IdentityError::Busy)
        ));
    }
    let attempts: i32 = sqlx::query_scalar("SELECT auth_attempts FROM human_accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(attempts, 10);
    let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM human_sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(sessions, 0);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn invitation_replay_cannot_bypass_the_account_password_budget() {
    let identity = Identity::new(support::database().await);
    identity
        .bootstrap("proof-admin", "long administrator password")
        .await
        .unwrap();
    let session = identity
        .login("proof-admin", "long administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    let invite = identity
        .invite(&admin, "proof-member", uuid::Uuid::new_v4())
        .await
        .unwrap();
    let token = invite.token.unwrap();
    let request = uuid::Uuid::new_v4();
    identity
        .accept_invitation(&token, "long original password", request)
        .await
        .unwrap();
    for _ in 0..10 {
        assert!(matches!(
            identity
                .accept_invitation(&token, "long incorrect password", request)
                .await,
            Err(IdentityError::Conflict)
        ));
    }
    assert!(matches!(
        identity
            .accept_invitation(&token, "long original password", request)
            .await,
        Err(IdentityError::Unauthorized)
    ));
    assert!(matches!(
        identity
            .login("proof-member", "long original password")
            .await,
        Err(IdentityError::Unauthorized)
    ));
}

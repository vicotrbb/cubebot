mod support;
use cubebot_orchestrator::identity::{Identity, IdentityError};

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn bootstrap_sessions_and_revocation_survive_restart() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    let (first, second) = tokio::join!(
        identity.bootstrap("admin-one", "long bootstrap password one"),
        identity.bootstrap("admin-two", "long bootstrap password two")
    );
    assert_ne!(first.is_ok(), second.is_ok());
    let (login, password) = if first.is_ok() {
        ("admin-one", "long bootstrap password one")
    } else {
        ("admin-two", "long bootstrap password two")
    };
    assert!(matches!(
        identity.login(login, "wrong password").await,
        Err(IdentityError::Unauthorized)
    ));
    let session = identity.login(login, password).await.unwrap();
    let actor = identity.authenticate(&session.token).await.unwrap();
    assert!(actor.administrator);
    let member = identity
        .create_account(
            &actor,
            "member-one",
            "long member password",
            false,
            uuid::Uuid::new_v4(),
        )
        .await
        .unwrap();
    let member_session = identity
        .login("member-one", "long member password")
        .await
        .unwrap();
    let member_actor = identity.authenticate(&member_session.token).await.unwrap();
    assert!(matches!(
        identity
            .create_account(
                &member_actor,
                "escalated",
                "long other password",
                true,
                uuid::Uuid::new_v4()
            )
            .await,
        Err(IdentityError::Forbidden)
    ));
    let identity = Identity::new(
        sqlx::PgPool::connect_with(pool.connect_options().as_ref().clone())
            .await
            .unwrap(),
    );
    assert_eq!(
        identity.authenticate(&session.token).await.unwrap().id,
        actor.id
    );
    identity
        .suspend(&actor, member, 1, uuid::Uuid::new_v4())
        .await
        .unwrap();
    assert!(matches!(
        identity.authenticate(&member_session.token).await,
        Err(IdentityError::Unauthorized)
    ));
    assert!(matches!(
        identity.login("member-one", "long member password").await,
        Err(IdentityError::Unauthorized)
    ));
    identity.logout(&session.token).await.unwrap();
    assert!(matches!(
        identity.authenticate(&session.token).await,
        Err(IdentityError::Unauthorized)
    ));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn rejects_expired_sessions_and_invalid_account_inputs() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    assert!(matches!(
        identity.bootstrap("x", "short").await,
        Err(IdentityError::InvalidInput)
    ));
    assert!(matches!(
        identity.authenticate("invalid-token").await,
        Err(IdentityError::Unauthorized)
    ));
    identity
        .bootstrap("expiry-admin", "long expiry password")
        .await
        .unwrap();
    let token = "0".repeat(64);
    // The test token has a deliberately expired interval, unlike production issuance.
    sqlx::query("INSERT INTO human_sessions (token_hash, account_id, created_at, expires_at) SELECT $1, id, now() - interval '2 days', now() - interval '1 day' FROM human_accounts LIMIT 1")
        .bind(sha2::Sha256::digest(token.as_bytes()).to_vec()).execute(&pool).await.unwrap();
    assert!(matches!(
        identity.authenticate(&token).await,
        Err(IdentityError::Unauthorized)
    ));
}

use sha2::Digest;

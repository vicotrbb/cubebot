mod support;
use cubebot_orchestrator::{
    connections::{ConfigureProvider, Connections, Credentials},
    identity::{Identity, IdentityError},
};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn provider_is_encrypted_durable_and_administered_with_revision_checks() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    identity
        .create_account(&admin, "member", "member password", false, Uuid::new_v4())
        .await
        .unwrap();
    let session = identity.login("member", "member password").await.unwrap();
    let member = identity.authenticate(&session.token).await.unwrap();
    let connections = Connections::new(pool.clone(), Credentials::new([7; 32]));
    let input = ConfigureProvider {
        request_id: Uuid::new_v4(),
        expected_revision: 0,
        model: "test-model".into(),
        api_key: "secret-provider-key".into(),
        access: None,
    };
    assert!(matches!(
        connections.configure(&member, &input).await,
        Err(IdentityError::Forbidden)
    ));
    let configured = connections.configure(&admin, &input).await.unwrap();
    assert_eq!(configured.revision, 1);
    assert_eq!(
        connections
            .configure(&admin, &input)
            .await
            .unwrap()
            .revision,
        1
    );
    assert!(matches!(
        connections.inspect(&member).await,
        Err(IdentityError::Forbidden)
    ));
    let encrypted: Vec<u8> = sqlx::query_scalar("SELECT credential FROM provider_connection")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        !encrypted
            .windows(input.api_key.len())
            .any(|window| window == input.api_key.as_bytes())
    );
    let restarted = Connections::new(pool, Credentials::new([7; 32]));
    assert_eq!(
        restarted.inspect(&admin).await.unwrap().unwrap().model,
        "test-model"
    );
    let stale = ConfigureProvider {
        request_id: Uuid::new_v4(),
        ..input
    };
    assert!(matches!(
        restarted.configure(&admin, &stale).await,
        Err(IdentityError::Conflict)
    ));
}
